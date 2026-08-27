//! Reasoner evaluation engine built on datafrog.
//!
//! For each stratum, allocate one datafrog [`Iteration`], seed base-fact
//! variables from the current EAVT snapshot, compile every rule into a
//! [`Plan`], and run the iteration to a fixed point. Derived tuples are
//! then diffed against previously-stored reasoner output and written back
//! via [`Store::transact`] — one transaction per rule so the `source`
//! tag ends up `reasoner:<rule-id>` for provenance tracking.
//!
//! See `docs/design/reasoner.md` for the broader rollout plan.

use std::collections::{BTreeMap, BTreeSet};

use datafrog::{Iteration, Relation, Variable};
use rusqlite::params;

use super::Result;
use super::compile::{Plan, compile_rule};
use super::parse::RuleSet;
use super::stratify::stratify;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// Summary of a single `evaluate` call.
#[derive(Debug, Clone, Default)]
pub struct EvalReport {
    /// Newly asserted derived facts across all rules.
    pub asserted: usize,
    /// Derived facts whose support disappeared and were retracted.
    pub retracted: usize,
    /// Number of strata actually executed (non-empty).
    pub strata_run: usize,
    /// `(rule_id, new_assertions)` pairs for the per-rule delta.
    pub per_rule: Vec<(String, usize)>,
}

/// Run the ruleset to a fixed point and persist the derived facts.
///
/// Full re-derivation: for each rule the complete set of tuples matching
/// its body is computed, then diffed against the currently-stored
/// derivations tagged `reasoner:<rule-id>`. New tuples are asserted; tuples
/// that disappeared are retracted. Within a single call this looks like a
/// normal bitemporal write — unchanged facts stay untouched.
pub fn evaluate(store: &mut Store, ruleset: &RuleSet, timestamp: &str) -> Result<EvalReport> {
    evaluate_in_graph(store, ruleset, timestamp, crate::schema::ROOT_GRAPH)
}

/// Run the ruleset against one graph and write every derivation back to it.
///
/// Premises and prior `reasoner:<rule-id>` output are both graph-scoped, so an
/// overlay cannot derive facts into its parent or retract a sibling's output.
pub fn evaluate_in_graph(
    store: &mut Store,
    ruleset: &RuleSet,
    timestamp: &str,
    graph: i64,
) -> Result<EvalReport> {
    if ruleset.is_empty() {
        return Ok(EvalReport::default());
    }

    let strata = stratify(ruleset)?;
    if strata.levels.is_empty() {
        return Ok(EvalReport::default());
    }

    // Build the shared term-id cache. Constants appearing in rule heads
    // need to be interned (so we can write them as `Value::Ref`). Predicate
    // IRIs are interned too so they become attribute ids.
    let mut world = World::load_graph(store, ruleset, graph)?;

    // Per-rule accumulator: fully derived (entity, value) sets for each
    // rule, collected across strata so we can diff + write them at the end.
    let mut derived_by_rule: BTreeMap<usize, BTreeSet<(i64, i64)>> = BTreeMap::new();

    let mut report = EvalReport::default();

    for rule_indices in &strata.levels {
        if rule_indices.is_empty() {
            continue;
        }
        report.strata_run += 1;
        run_stratum(ruleset, rule_indices, &mut world, &mut derived_by_rule)?;
    }

    // Write the delta back through the store, per rule.
    for (rule_idx, new_tuples) in &derived_by_rule {
        let rule = &ruleset.rules[*rule_idx];
        let (asserted, retracted) =
            write_rule_delta(store, rule, new_tuples, timestamp, graph, &mut world)?;
        report.asserted += asserted;
        report.retracted += retracted;
        report.per_rule.push((rule.id.clone(), asserted));
    }

    Ok(report)
}

// ── Stratum evaluation ─────────────────────────────────────────

fn run_stratum(
    ruleset: &RuleSet,
    rule_indices: &[usize],
    world: &mut World,
    derived_by_rule: &mut BTreeMap<usize, BTreeSet<(i64, i64)>>,
) -> Result<()> {
    // Collect every predicate that will be touched by this stratum's rules
    // (head + body). Each gets a datafrog Variable pre-seeded with anything
    // already in `world` (extensional facts or lower-stratum derivations).
    let mut preds: BTreeSet<String> = BTreeSet::new();
    for &idx in rule_indices {
        let rule = &ruleset.rules[idx];
        preds.insert(rule.head.predicate.clone());
        for body in &rule.body {
            preds.insert(body.atom().predicate.clone());
        }
    }

    let mut iteration = Iteration::new();
    let mut vars: BTreeMap<String, Variable<(i64, i64)>> = BTreeMap::new();
    for pred in &preds {
        let var = iteration.variable::<(i64, i64)>(pred);
        if let Some(tuples) = world.tuples.get(pred) {
            var.extend(tuples.iter().copied());
        }
        vars.insert(pred.clone(), var);
    }

    // Compile every rule; this may allocate helper variables on `iteration`.
    // `world.tuples` is what negated atoms antijoin against — lower strata
    // are complete by the time this stratum compiles.
    let mut plans: Vec<Plan> = Vec::with_capacity(rule_indices.len());
    for &idx in rule_indices {
        let rule = &ruleset.rules[idx];
        plans.push(compile_rule(
            &mut iteration,
            rule,
            &world.const_ids,
            &vars,
            &world.tuples,
        )?);
    }

    // Main fixpoint loop.
    while iteration.changed() {
        for plan in &plans {
            plan.step(&vars);
        }
    }

    // Drain variables for predicates derived in this stratum back into
    // `world` so later strata can read them. Non-derived predicates are
    // left alone — datafrog's complete() is consuming so we only call it
    // on variables we actually need.
    let stratum_heads: BTreeSet<&str> = rule_indices
        .iter()
        .map(|i| ruleset.rules[*i].head.predicate.as_str())
        .collect();

    for (pred, var) in vars {
        if !stratum_heads.contains(pred.as_str()) {
            continue;
        }
        let relation: Relation<(i64, i64)> = var.complete();
        let entry = world.tuples.entry(pred).or_default();
        for tuple in relation.iter() {
            entry.insert(*tuple);
        }
    }

    // Per-rule book-keeping: record each rule's projection against the
    // final world. For stratum-local rules this is the final fixpoint;
    // for later strata this information stays valid because lower strata
    // are already fully computed and never change.
    for &idx in rule_indices {
        let rule = &ruleset.rules[idx];
        let entry = derived_by_rule.entry(idx).or_default();
        project_rule_body(rule, world, entry);
    }

    Ok(())
}

/// Compute the set of tuples a rule derives from the current world state.
///
/// Used by the reactive reasoner to re-derive a single rule's output
/// without running the full datafrog iteration.
#[cfg(feature = "reactive-reasoner")]
pub(crate) fn project_rule_from_world(
    rule: &super::ast::Rule,
    world: &World,
) -> BTreeSet<(i64, i64)> {
    let mut out = BTreeSet::new();
    project_rule_body(rule, world, &mut out);
    out
}

/// Project a rule's body against the world and add the resulting head
/// tuples to `out`. This runs one final time after fixpoint to attribute
/// each derived tuple back to the rule that produced it, and is also the
/// reactive path's per-rule re-derivation. General over any body length
/// (quipu-923): positive atoms join left-deep; negated atoms then filter,
/// mirroring the compiled pipeline's stratified antijoin.
fn project_rule_body(rule: &super::ast::Rule, world: &World, out: &mut BTreeSet<(i64, i64)>) {
    use super::ast::{BodyAtom, Term};
    let mut positives: Vec<&super::ast::Atom> = Vec::new();
    let mut negatives: Vec<&super::ast::Atom> = Vec::new();
    for b in &rule.body {
        match b {
            BodyAtom::Positive(a) => positives.push(a),
            BodyAtom::Negative(a) => negatives.push(a),
        }
    }
    if positives.is_empty() {
        return;
    }
    // Safety, mirroring compile: a negated atom over a variable no positive
    // atom binds derives nothing rather than something surprising.
    let positive_vars: BTreeSet<&str> = positives
        .iter()
        .flat_map(|a| a.args.iter())
        .filter_map(|t| match t {
            Term::Var(v) => Some(v.as_str()),
            _ => None,
        })
        .collect();
    for neg in &negatives {
        for term in &neg.args {
            if let Term::Var(v) = term
                && !positive_vars.contains(v.as_str())
            {
                return;
            }
        }
    }

    let head_tuple = |row: &BTreeMap<&str, i64>| -> Option<(i64, i64)> {
        let mut out_row = [0_i64; 2];
        for (i, term) in rule.head.args.iter().enumerate() {
            out_row[i] = match term {
                Term::Var(v) => row.get(v.as_str()).copied()?,
                Term::Iri(iri) => *world.const_ids.get(iri)?,
                Term::Str(_) => return None,
            };
        }
        Some((out_row[0], out_row[1]))
    };

    // Left-deep join over the positive atoms.
    let mut rows: Vec<BTreeMap<&str, i64>> = vec![BTreeMap::new()];
    for atom in &positives {
        let Some(tuples) = world.tuples.get(&atom.predicate) else {
            return;
        };
        let mut next = Vec::new();
        for row in &rows {
            for &(c0, c1) in tuples {
                let mut candidate = row.clone();
                if bind_atom(atom, &[c0, c1], world, &mut candidate) {
                    next.push(candidate);
                }
            }
        }
        rows = next;
        if rows.is_empty() {
            return;
        }
    }

    // Negation-as-failure over the world's (lower-stratum-complete) tuples.
    'row: for row in rows {
        for neg in &negatives {
            // An absent predicate has no tuples: the negation holds vacuously.
            for &(c0, c1) in world.tuples.get(&neg.predicate).into_iter().flatten() {
                let mut probe = row.clone();
                if bind_atom(neg, &[c0, c1], world, &mut probe) {
                    continue 'row;
                }
            }
        }
        if let Some(t) = head_tuple(&row) {
            out.insert(t);
        }
    }
}

/// Unify one body atom against one stored tuple.
///
/// Returns false — rejecting the tuple — when a constant column disagrees
/// with the fact, or when an already-bound variable would have to take a
/// second value (the shared-variable check for a two-atom join).
///
/// The constant arm is aegis-jgxas: this function used to bind variables and
/// *silently skip* `Term::Iri`, so `rdf:type(?x, <Commit>)` matched every
/// typed entity in the graph and the reactive path derived `GitCommit` for
/// all of them. A constant column is a filter, and an unresolvable constant
/// matches nothing rather than matching everything.
fn bind_atom<'a>(
    atom: &'a super::ast::Atom,
    row: &[i64],
    world: &World,
    out: &mut BTreeMap<&'a str, i64>,
) -> bool {
    use super::ast::Term;
    for (term, &val) in atom.args.iter().zip(row.iter()) {
        match term {
            Term::Var(name) => match out.get(name.as_str()) {
                Some(existing) if *existing != val => return false,
                _ => {
                    out.insert(name.as_str(), val);
                }
            },
            Term::Iri(iri) => match world.const_ids.get(iri) {
                Some(&id) if id == val => {}
                _ => return false,
            },
            // Facts are `Value::Ref` triples; a literal cannot match one.
            Term::Str(_) => return false,
        }
    }
    true
}

// ── World: term-id cache + per-predicate tuples ───────────────

pub(crate) struct World {
    /// Predicate IRI → set of `(entity, value_ref)` tuples currently known
    /// to hold (base facts + derivations from lower strata).
    pub(crate) tuples: BTreeMap<String, BTreeSet<(i64, i64)>>,
    /// Predicate IRI → attribute term id. Populated lazily on first use.
    pub(crate) attr_ids: BTreeMap<String, i64>,
    /// IRI → term id for constants referenced in rule heads.
    pub(crate) const_ids: BTreeMap<String, i64>,
}

impl World {
    /// Load only predicates and constants referenced by selected rules.
    ///
    /// Reactive evaluation calls this after its dependency analysis has
    /// already isolated the affected rules. Keeping that scope through the
    /// SQL read avoids loading the entire fact table and then throwing nearly
    /// all of it away in `attr_to_pred`.
    #[cfg(feature = "reactive-reasoner")]
    pub(crate) fn load_rule_indices(
        store: &Store,
        ruleset: &RuleSet,
        rule_indices: &[usize],
    ) -> Result<Self> {
        Self::load_graph_rule_indices(store, ruleset, crate::schema::ROOT_GRAPH, rule_indices)
    }

    fn load_graph(store: &Store, ruleset: &RuleSet, graph: i64) -> Result<Self> {
        let indices: Vec<usize> = (0..ruleset.rules.len()).collect();
        Self::load_graph_rule_indices(store, ruleset, graph, &indices)
    }

    fn load_graph_rule_indices(
        store: &Store,
        ruleset: &RuleSet,
        graph: i64,
        rule_indices: &[usize],
    ) -> Result<Self> {
        let mut preds: BTreeSet<String> = BTreeSet::new();
        for &rule_idx in rule_indices {
            let rule = &ruleset.rules[rule_idx];
            preds.insert(rule.head.predicate.clone());
            for body in &rule.body {
                preds.insert(body.atom().predicate.clone());
            }
        }

        // Look up (don't intern) — a predicate with no existing facts is
        // fine, it just starts empty and may get written into later.
        let mut attr_ids: BTreeMap<String, i64> = BTreeMap::new();
        let mut attr_to_pred: BTreeMap<i64, String> = BTreeMap::new();
        for pred in &preds {
            if let Some(id) = store.lookup(pred)? {
                attr_ids.insert(pred.clone(), id);
                attr_to_pred.insert(id, pred.clone());
            }
        }

        let mut tuples: BTreeMap<String, BTreeSet<(i64, i64)>> = BTreeMap::new();
        for pred in &preds {
            tuples.insert(pred.clone(), BTreeSet::new());
        }

        // Load only facts for predicates referenced by these rules. The old
        // path loaded the full current table once per affected stratum and
        // discarded every attribute absent from `attr_to_pred` afterwards.
        //
        // The evaluated rules' OWN prior derivations are excluded (quipu-923):
        // they are diff targets in `write_rule_delta`, never premises. Feeding
        // them back in let mutually supporting rules hold each other up after
        // their base support was retracted — the stable non-converging
        // fixpoint `probe_mutual_class_equivalence_under_retraction` used to
        // pin. Chaining is unaffected in both regimes: within one evaluation,
        // a lower stratum's output reaches later strata through
        // `world.tuples` in memory; across reactive wakes, a rule NOT being
        // re-derived here keeps its stored output readable as a premise.
        let excluded_sources: Vec<String> = rule_indices
            .iter()
            .map(|&i| format!("reasoner:{}", ruleset.rules[i].id))
            .collect();
        let attribute_ids: Vec<i64> = attr_to_pred.keys().copied().collect();
        let facts = store.current_facts_for_attributes_in_graph_excluding_sources(
            &attribute_ids,
            graph,
            &excluded_sources,
        )?;
        for fact in facts {
            if let Some(pred) = attr_to_pred.get(&fact.attribute)
                && let Value::Ref(target) = fact.value
            {
                tuples
                    .get_mut(pred)
                    .expect("predicate seeded above")
                    .insert((fact.entity, target));
            }
        }

        // Resolve constants used in rule heads so we can emit `Value::Ref`
        // for them, and in rule BODIES so a constant column can be matched
        // against stored facts. Constants that don't exist yet are simply
        // absent: in the head that is a compile-time rejection
        // (`head_slot`), in the body it makes the rule derive nothing
        // (`unsatisfiable`) — see `compile.rs`.
        //
        // Body constants were omitted here until aegis-jgxas. Without them
        // `project_rule_body` had no term id to compare against and silently
        // ignored the constant, matching every tuple of the predicate.
        let mut const_ids: BTreeMap<String, i64> = BTreeMap::new();
        for &rule_idx in rule_indices {
            let rule = &ruleset.rules[rule_idx];
            let head_terms = rule.head.args.iter();
            let body_terms = rule.body.iter().flat_map(|b| b.atom().args.iter());
            for term in head_terms.chain(body_terms) {
                if let super::ast::Term::Iri(iri) = term
                    && !const_ids.contains_key(iri)
                    && let Some(id) = store.lookup(iri)?
                {
                    const_ids.insert(iri.clone(), id);
                }
            }
        }

        Ok(Self {
            tuples,
            attr_ids,
            const_ids,
        })
    }

    /// Ensure the head predicate's attribute id is interned. Called right
    /// before writing derivations — predicates that had no facts in the
    /// store still need an id for `Datum::attribute`.
    fn ensure_attr(&mut self, store: &mut Store, pred: &str) -> Result<i64> {
        if let Some(id) = self.attr_ids.get(pred) {
            return Ok(*id);
        }
        let id = store.intern(pred)?;
        self.attr_ids.insert(pred.to_string(), id);
        Ok(id)
    }
}

// ── Write-back: diff against stored reasoner output, transact ──

fn write_rule_delta(
    store: &mut Store,
    rule: &super::ast::Rule,
    new_tuples: &BTreeSet<(i64, i64)>,
    timestamp: &str,
    graph: i64,
    world: &mut World,
) -> Result<(usize, usize)> {
    let attr_id = world.ensure_attr(store, &rule.head.predicate)?;
    let source = format!("reasoner:{}", rule.id);

    let old_tuples = load_existing_derivations_in_graph(store, attr_id, &source, graph)?;

    let mut datums: Vec<Datum> = Vec::new();
    // Retract anything that used to hold and no longer does.
    for tuple in old_tuples.difference(new_tuples) {
        datums.push(Datum {
            entity: tuple.0,
            attribute: attr_id,
            value: Value::Ref(tuple.1),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Retract,
        });
    }
    // Assert anything new.
    for tuple in new_tuples.difference(&old_tuples) {
        datums.push(Datum {
            entity: tuple.0,
            attribute: attr_id,
            value: Value::Ref(tuple.1),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }

    if datums.is_empty() {
        return Ok((0, 0));
    }

    let asserted = datums.iter().filter(|d| d.op == Op::Assert).count();
    let retracted = datums.iter().filter(|d| d.op == Op::Retract).count();
    store.transact_to_graph(&datums, timestamp, Some("reasoner"), Some(&source), graph)?;
    Ok((asserted, retracted))
}

/// Load the currently-asserted tuples derived by `source` (typically
/// `reasoner:<rule-id>`). Only reference-valued facts on the rule's head
/// attribute are considered — other shapes cannot be produced by Phase 2
/// and must not exist under this source.
#[cfg(feature = "reactive-reasoner")]
pub(crate) fn load_existing_derivations(
    store: &Store,
    attr_id: i64,
    source: &str,
) -> Result<BTreeSet<(i64, i64)>> {
    load_existing_derivations_in_graph(store, attr_id, source, crate::schema::ROOT_GRAPH)
}

fn load_existing_derivations_in_graph(
    store: &Store,
    attr_id: i64,
    source: &str,
    graph: i64,
) -> Result<BTreeSet<(i64, i64)>> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT f.e, f.v FROM facts f \
             JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL \
               AND f.a = ?1 AND t.source = ?2 AND f.g = ?3",
        )
        .map_err(crate::Error::from)?;
    let mut rows = stmt
        .query(params![attr_id, source, graph])
        .map_err(crate::Error::from)?;
    let mut out: BTreeSet<(i64, i64)> = BTreeSet::new();
    while let Some(row) = rows.next().map_err(crate::Error::from)? {
        let e: i64 = row.get(0).map_err(crate::Error::from)?;
        let v_bytes: Vec<u8> = row.get(1).map_err(crate::Error::from)?;
        if let Value::Ref(target) = Value::from_bytes(&v_bytes)? {
            out.insert((e, target));
        }
    }
    Ok(out)
}
