//! OWL 2 RL materialization — entailment writing, extracted from `owl.rs`.
//!
//! Materialization runs derivation passes to FIXPOINT, not one shot per axiom
//! family. The one-pass version had a recorded staleness defect (gap G3 of
//! `docs/design/semantic-reasoning-gaps.md`): a type introduced by
//! `rdfs:range` never fed the subclass closure in the same run, and the
//! operational workaround was re-encoding OWL axioms as Datalog rules
//! (`shapes/aegis-class-subsumption.rules.ttl`).
//!
//! Termination: every pass only adds facts (no retraction), each derived fact
//! combines entities, properties, and classes already present in the store or
//! the loaded ontology, and a pass that derives nothing new ends the loop —
//! monotone growth over a finite universe.

use std::collections::{HashMap, HashSet};

use crate::error::Result;
use crate::store::{Datum, Store};
use crate::types::{Fact, Op, Value};

use super::owl_parse::{collect_predicate_facts, collect_type_facts, transitive_closure};
use super::{MaterializeReport, Ontology, RDF_TYPE};

/// Accumulates one derivation pass: a fact is staged only if it is neither in
/// the store already nor staged earlier in this pass, so counts stay honest
/// and the fixpoint loop terminates.
struct Pass<'a> {
    seen: HashSet<(i64, i64, Vec<u8>)>,
    datums: Vec<Datum>,
    timestamp: &'a str,
}

impl<'a> Pass<'a> {
    fn from_facts(facts: &[Fact], timestamp: &'a str) -> Self {
        let mut seen = HashSet::new();
        for f in facts {
            seen.insert((f.entity, f.attribute, f.value.to_bytes()));
        }
        Self {
            seen,
            datums: Vec::new(),
            timestamp,
        }
    }

    fn push(&mut self, entity: i64, attribute: i64, value: Value, counter: &mut usize) {
        if self.seen.insert((entity, attribute, value.to_bytes())) {
            self.datums.push(Datum {
                entity,
                attribute,
                value,
                valid_from: self.timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
            *counter += 1;
        }
    }
}

impl Ontology {
    /// Materialize OWL 2 RL entailments into the store, to fixpoint.
    ///
    /// Writes derived facts with `source = "owl:materialize"` so they can be
    /// identified and re-materialized when the ontology changes. Passes repeat
    /// until one derives nothing new, so axiom families compose: a type
    /// introduced by `rdfs:range` feeds the subclass closure of the next pass.
    pub fn materialize(&self, store: &mut Store, timestamp: &str) -> Result<MaterializeReport> {
        self.materialize_from(store, timestamp, None)
    }

    /// Materialize SEMI-NAIVELY from a committed delta (aegis-2dp8e2).
    ///
    /// The full path above re-reads EVERY current fact on every pass. At
    /// 641,803 facts that measured ~1.15 s per scan (from `reads.rs`'s own
    /// "608 ms in release at 340k triples"), >= 2 scans per relevant write, and
    /// ~67 s of work per 60 s of wall clock at a 29 writes/min offered load —
    /// it could not keep up, and it ratcheted, because its own output lands in
    /// the companion graph which is a premise for the next run.
    ///
    /// Seven of the eight rule families are SINGLE-PREMISE: each derived fact
    /// comes from ONE store fact plus the AXIOMS, and the axiom closures are
    /// computed from the ontology rather than the store. Those need the delta
    /// alone. Only `transitive_properties` joins two store facts, and it is
    /// given (delta x existing) restricted to its own predicate.
    pub fn materialize_delta(
        &self,
        store: &mut Store,
        timestamp: &str,
        seed: &[Fact],
    ) -> Result<MaterializeReport> {
        self.materialize_from(store, timestamp, Some(seed))
    }

    fn materialize_from(
        &self,
        store: &mut Store,
        timestamp: &str,
        seed: Option<&[Fact]>,
    ) -> Result<MaterializeReport> {
        // Placement (quipu-0b6): premises are ROOT plus its companion inferred
        // graph; every entailment is written to the companion. The freshness
        // note records the premise head the closure reflects.
        let companion =
            store.ensure_companion_inferred_graph(crate::schema::ROOT_GRAPH, timestamp)?;
        let premise_head = store.transaction_head()?;
        let mut report = MaterializeReport::default();
        // BOUNDED. The previous `loop` terminated on monotonicity — correct, and
        // unbounded in COST. A budget that is hit must be LOUD: a silent stop is
        // a partial closure that looks complete.
        const MAX_PASSES: usize = 64;
        let mut frontier: Option<Vec<Fact>> = seed.map(<[Fact]>::to_vec);
        let mut passes = 0usize;
        loop {
            if passes >= MAX_PASSES {
                report.pass_budget_exhausted = true;
                eprintln!(
                    "owl materialize: STOPPED at the {MAX_PASSES}-pass budget with work still \
                     pending — the closure is PARTIAL. This is not a normal termination \
                     (aegis-2dp8e2)."
                );
                break;
            }
            passes += 1;
            let datums = self.derive_pass(
                store,
                companion,
                timestamp,
                &mut report,
                frontier.as_deref(),
            )?;
            if datums.is_empty() {
                break;
            }
            report.total += datums.len();
            store.transact_to_graph(
                &datums,
                timestamp,
                Some("owl"),
                Some("owl:materialize"),
                companion,
            )?;
            // Semi-naive: the next pass's premises are THIS pass's output. The
            // full path keeps re-reading everything, as before.
            if frontier.is_some() {
                frontier = Some(
                    datums
                        .iter()
                        .map(|d| Fact {
                            entity: d.entity,
                            attribute: d.attribute,
                            value: d.value.clone(),
                            tx: 0,
                            valid_from: d.valid_from.clone(),
                            valid_to: None,
                            op: Op::Assert,
                        })
                        .collect(),
                );
            }
        }
        report.passes = passes;
        store.note_inferred_freshness(companion, premise_head, timestamp)?;
        Ok(report)
    }

    /// One derivation pass over the store's current facts. Returns only facts
    /// not already present.
    /// One derivation pass.
    ///
    /// `seed = None` is the NAIVE path: premises are every current fact, and the
    /// de-duplication set is built from all of them. That is what an ontology
    /// LOAD wants — there is no delta to start from.
    ///
    /// `seed = Some(delta)` is SEMI-NAIVE (aegis-2dp8e2). The seven
    /// single-premise families read the delta and nothing else. `transitive`
    /// alone joins two store facts, so it gets (delta x existing) restricted to
    /// its own predicate.
    ///
    /// The de-duplication set is the subtle part. `Pass::from_facts` exists to
    /// stop staging a fact the store already has, and under the naive path it is
    /// built from the whole store. Rebuilding that per pass is the cost being
    /// removed — but dropping it would re-assert facts that already exist, which
    /// is the 99.6%-no-op behaviour measured on the ingest lane, in a different
    /// place. So it is built ATTRIBUTE-SCOPED instead: every fact these rules can
    /// possibly derive has `rdf:type` or an axiom-named property as its
    /// attribute, and nothing else needs to be in the set.
    fn derive_pass(
        &self,
        store: &Store,
        companion: i64,
        timestamp: &str,
        report: &mut MaterializeReport,
        seed: Option<&[Fact]>,
    ) -> Result<Vec<Datum>> {
        let graphs = [crate::schema::ROOT_GRAPH, companion];
        let rdf_type_id = store.intern(RDF_TYPE)?;

        let (premises, dedup): (Vec<Fact>, Vec<Fact>) = match seed {
            None => {
                let all = store.current_facts_in_graphs(&graphs)?;
                (all.clone(), all)
            }
            Some(delta) => {
                // Dedup scoped by ENTITY, not by attribute. Attribute scoping
                // looked right and was not: `rdf:type` sits on nearly every
                // entity, so the dedup set still grew with the store and the
                // per-write cost kept tracking it — measured 307 facts read at
                // 1x and 3007 at 10x, the naive shape with a smaller constant.
                // Caught by `per_write_cost_does_not_scale_with_store_size`,
                // which is the only reason it is not still in here.
                //
                // NO PRELOADED DEDUP SET AT ALL.
                //
                // There was one, described as "a cheap first cut". It was not
                // cheap: it was `current_facts_for_entities_in_graphs`, and with
                // no index leading on `e` it full-scanned. Re-measured after the
                // post-filter was indexed and the wall clock did not move —
                // 1400.4 ms before, 1396.7 ms after — because the scan had never
                // been in the post-filter, it was here.
                //
                // It is not needed. `Pass::seen` starts empty and still
                // de-duplicates WITHIN a pass, because `push` inserts as it
                // goes; and everything already in the STORE is removed by the
                // post-filter at the end of this function, which is the actual
                // correctness guarantee and is scoped to the candidates. A
                // preload can only ever be an optimisation, and this one cost
                // more than it saved.
                (delta.to_vec(), Vec::new())
            }
        };
        report.premise_facts_read += premises.len() + dedup.len();
        let mut pass = Pass::from_facts(&dedup, timestamp);
        let type_facts = collect_type_facts(&premises, rdf_type_id);

        // 1. Subclass transitive closure: if x : A and A ⊑ B, then x : B.
        let class_closure = transitive_closure(&self.axioms.subclass_of);
        for (entity_id, class_id) in &type_facts {
            let class_iri = store.resolve(*class_id)?;
            if let Some(supers) = class_closure.get(&class_iri) {
                for super_class in supers {
                    let super_id = store.intern(super_class)?;
                    pass.push(
                        *entity_id,
                        rdf_type_id,
                        Value::Ref(super_id),
                        &mut report.subclass_inferences,
                    );
                }
            }
        }

        // 1b. Subproperty transitive closure: if x p y and p ⊑ q, then x q y.
        //
        // This was PARSED and then dropped on the floor (aegis-qfncf): `Axioms`
        // carried `sub_property_of`, `axiom_summary()` counted it, `/ontology`
        // reported it back — and nothing ever read it. The axiom class was
        // inert end to end while reporting as accepted.
        let property_closure = transitive_closure(&self.axioms.subproperty_of);
        for (sub_property, supers) in &property_closure {
            // `lookup`, not `intern`: a subproperty naming a predicate that no
            // fact uses has nothing to restate, and interning it would mint a
            // dangling id as a side effect of reasoning about it.
            let Some(sub_id) = store.lookup(sub_property)? else {
                continue;
            };
            for (entity_id, value) in &collect_predicate_facts(&premises, sub_id) {
                for super_property in supers {
                    let super_id = store.intern(super_property)?;
                    pass.push(
                        *entity_id,
                        super_id,
                        value.clone(),
                        &mut report.sub_property_inferences,
                    );
                }
            }
        }

        // 2. Equivalent classes: bidirectional subclass.
        for (a, b) in &self.axioms.equivalent_classes {
            let a_id = store.intern(a)?;
            let b_id = store.intern(b)?;
            for (entity_id, class_id) in &type_facts {
                let other = if *class_id == a_id {
                    b_id
                } else if *class_id == b_id {
                    a_id
                } else {
                    continue;
                };
                pass.push(
                    *entity_id,
                    rdf_type_id,
                    Value::Ref(other),
                    &mut report.equivalent_class_inferences,
                );
            }
        }

        // 3. Inverse properties: if (a P b) and P inverseOf Q, assert (b Q a).
        for (p, q) in &self.axioms.inverse_of {
            let p_id = store.intern(p)?;
            let q_id = store.intern(q)?;
            for (s, o) in &collect_predicate_facts(&premises, p_id) {
                if let Value::Ref(o_id) = o {
                    pass.push(*o_id, q_id, Value::Ref(*s), &mut report.inverse_inferences);
                }
            }
        }

        // 4. Symmetric properties: if (a P b), assert (b P a).
        for prop in &self.axioms.symmetric_properties {
            let prop_id = store.intern(prop)?;
            for (s, o) in &collect_predicate_facts(&premises, prop_id) {
                if let Value::Ref(o_id) = o {
                    pass.push(
                        *o_id,
                        prop_id,
                        Value::Ref(*s),
                        &mut report.symmetric_inferences,
                    );
                }
            }
        }

        // 4b. Transitive properties: if (a P b) and (b P c), assert (a P c) —
        // the full closure to fixpoint, not one join pass, so a→b→c→d yields
        // a→d. Same recovered dead-end shape as 1b: parsed, counted, never
        // materialized (gap G1).
        //
        // THE ONE FAMILY THAT NEEDS HISTORY (aegis-2dp8e2). `a→b` and `b→c` may
        // arrive in different transactions, so a delta-only adjacency would miss
        // `a→c` whenever the other edge is older. The delta is therefore UNIONED
        // with the existing edges OF THIS PREDICATE — attribute-scoped, never a
        // whole-store read. Under `seed = None` `premises` already is everything
        // and the union is a no-op.
        for prop in &self.axioms.transitive_properties {
            let Some(prop_id) = store.lookup(prop)? else {
                continue;
            };
            let mut adjacency: HashMap<i64, Vec<i64>> = HashMap::new();
            let history: Vec<Fact> = if seed.is_some() {
                store.current_facts_for_attributes_in_graphs_excluding_sources(
                    &[prop_id],
                    &graphs,
                    &[],
                )?
            } else {
                Vec::new()
            };
            for (s, v) in collect_predicate_facts(&premises, prop_id)
                .iter()
                .chain(collect_predicate_facts(&history, prop_id).iter())
            {
                if let Value::Ref(o_id) = v {
                    let e = adjacency.entry(*s).or_default();
                    if !e.contains(o_id) {
                        e.push(*o_id);
                    }
                }
            }
            for &start in adjacency.keys() {
                let mut reached: HashSet<i64> = HashSet::new();
                let mut stack: Vec<i64> = adjacency[&start].clone();
                while let Some(node) = stack.pop() {
                    if !reached.insert(node) {
                        continue;
                    }
                    if let Some(nexts) = adjacency.get(&node) {
                        stack.extend(nexts.iter().copied());
                    }
                }
                for target in reached {
                    if target != start {
                        pass.push(
                            start,
                            prop_id,
                            Value::Ref(target),
                            &mut report.transitive_inferences,
                        );
                    }
                }
            }
        }

        // 4c. Equivalent properties: facts under either property restate under
        // the other (bidirectional subproperty semantics). Same recovered
        // dead-end shape as 1b and 4b (gap G2).
        for (p, q) in &self.axioms.equivalent_properties {
            for (from, to) in [(p, q), (q, p)] {
                let Some(from_id) = store.lookup(from)? else {
                    continue;
                };
                let to_id = store.intern(to)?;
                for (s, v) in &collect_predicate_facts(&premises, from_id) {
                    pass.push(
                        *s,
                        to_id,
                        v.clone(),
                        &mut report.equivalent_property_inferences,
                    );
                }
            }
        }

        // 5. Domain/range inference: if (s P o) and P domain D, assert s : D;
        // if P range R, assert o : R.
        for (prop, class) in &self.axioms.domains {
            let prop_id = store.intern(prop)?;
            let class_id = store.intern(class)?;
            for (s, _) in &collect_predicate_facts(&premises, prop_id) {
                pass.push(
                    *s,
                    rdf_type_id,
                    Value::Ref(class_id),
                    &mut report.domain_range_inferences,
                );
            }
        }
        for (prop, class) in &self.axioms.ranges {
            let prop_id = store.intern(prop)?;
            let class_id = store.intern(class)?;
            for (_, o) in &collect_predicate_facts(&premises, prop_id) {
                if let Value::Ref(o_id) = o {
                    pass.push(
                        *o_id,
                        rdf_type_id,
                        Value::Ref(class_id),
                        &mut report.domain_range_inferences,
                    );
                }
            }
        }

        // THE CORRECTNESS GUARANTEE for the semi-naive path (aegis-2dp8e2).
        //
        // The naive path's `seen` was built from EVERY current fact, so nothing
        // already in the store could be staged. Semi-naive cannot afford that
        // read, and an entity-scoped approximation of it was WRONG — it missed
        // facts and re-derived asserted ROOT triples into the companion, which
        // is a silent divergence from the naive fixpoint.
        //
        // So the staged set is re-checked against the store for exactly the
        // entities it names. That read is bounded by the CANDIDATES, not by the
        // store, which is the property this whole change is about, and it is
        // correct by construction rather than by an argument about which
        // entities a rule can touch.
        if seed.is_some() && !pass.datums.is_empty() {
            let mut ents: Vec<i64> = Vec::new();
            let mut attrs: Vec<i64> = Vec::new();
            for d in &pass.datums {
                if !ents.contains(&d.entity) {
                    ents.push(d.entity);
                }
                if !attrs.contains(&d.attribute) {
                    attrs.push(d.attribute);
                }
            }
            // Scoped on BOTH axes so `idx_aevt` can serve it. Entity-only
            // returned the right rows and FULL-SCANNED to find them — 8 rows in
            // 1400 ms at 1,000,002 facts — which `premise_facts_read` cannot
            // see, because it counts rows RETURNED, not rows scanned.
            let present: HashSet<(i64, i64, Vec<u8>)> = store
                .current_facts_for_attributes_and_entities_in_graphs(&attrs, &ents, &graphs)?
                .into_iter()
                .map(|f| (f.entity, f.attribute, f.value.to_bytes()))
                .collect();
            report.premise_facts_read += present.len();
            pass.datums
                .retain(|d| !present.contains(&(d.entity, d.attribute, d.value.to_bytes())));
        }

        Ok(pass.datums)
    }
}
