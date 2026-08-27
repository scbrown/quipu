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
use crate::types::{Op, Value};

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
    fn from_store(store: &Store, timestamp: &'a str) -> Result<Self> {
        let mut seen = HashSet::new();
        for f in &store.current_facts()? {
            seen.insert((f.entity, f.attribute, f.value.to_bytes()));
        }
        Ok(Self {
            seen,
            datums: Vec::new(),
            timestamp,
        })
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
        let mut report = MaterializeReport::default();
        loop {
            let datums = self.derive_pass(store, timestamp, &mut report)?;
            if datums.is_empty() {
                break;
            }
            report.total += datums.len();
            store.transact(&datums, timestamp, Some("owl"), Some("owl:materialize"))?;
        }
        Ok(report)
    }

    /// One derivation pass over the store's current facts. Returns only facts
    /// not already present.
    fn derive_pass(
        &self,
        store: &Store,
        timestamp: &str,
        report: &mut MaterializeReport,
    ) -> Result<Vec<Datum>> {
        let mut pass = Pass::from_store(store, timestamp)?;
        let rdf_type_id = store.intern(RDF_TYPE)?;
        let type_facts = collect_type_facts(store, rdf_type_id)?;

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
            for (entity_id, value) in &collect_predicate_facts(store, sub_id)? {
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
            for (s, o) in &collect_predicate_facts(store, p_id)? {
                if let Value::Ref(o_id) = o {
                    pass.push(*o_id, q_id, Value::Ref(*s), &mut report.inverse_inferences);
                }
            }
        }

        // 4. Symmetric properties: if (a P b), assert (b P a).
        for prop in &self.axioms.symmetric_properties {
            let prop_id = store.intern(prop)?;
            for (s, o) in &collect_predicate_facts(store, prop_id)? {
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
        for prop in &self.axioms.transitive_properties {
            let Some(prop_id) = store.lookup(prop)? else {
                continue;
            };
            let mut adjacency: HashMap<i64, Vec<i64>> = HashMap::new();
            for (s, v) in &collect_predicate_facts(store, prop_id)? {
                if let Value::Ref(o_id) = v {
                    adjacency.entry(*s).or_default().push(*o_id);
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
                for (s, v) in &collect_predicate_facts(store, from_id)? {
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
            for (s, _) in &collect_predicate_facts(store, prop_id)? {
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
            for (_, o) in &collect_predicate_facts(store, prop_id)? {
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

        Ok(pass.datums)
    }
}
