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
    let mut world = World::load(store, ruleset)?;

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
            write_rule_delta(store, rule, new_tuples, timestamp, &mut world)?;
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
    let mut plans: Vec<Plan> = Vec::with_capacity(rule_indices.len());
    for &idx in rule_indices {
        let rule = &ruleset.rules[idx];
        plans.push(compile_rule(&mut iteration, rule, &world.const_ids, &vars)?);
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

/// Project a rule's body against the world and add the resulting head
/// tuples to `out`. This runs one final time after fixpoint to attribute
/// each derived tuple back to the rule that produced it.
fn project_rule_body(rule: &super::ast::Rule, world: &World, out: &mut BTreeSet<(i64, i64)>) {
    use super::ast::{BodyAtom, Term};
    let body: Vec<&super::ast::Atom> = rule
        .body
        .iter()
        .filter_map(|b| match b {
            BodyAtom::Positive(a) => Some(a),
            BodyAtom::Negative(_) => None,
        })
        .collect();

    let head_slot =
        |name: &str, row: &BTreeMap<&str, i64>| -> Option<i64> { row.get(name).copied() };

    let head_tuple = |row: &BTreeMap<&str, i64>| -> Option<(i64, i64)> {
        let mut out_row = [0_i64; 2];
        for (i, term) in rule.head.args.iter().enumerate() {
            out_row[i] = match term {
                Term::Var(v) => head_slot(v.as_str(), row)?,
                Term::Iri(iri) => *world.const_ids.get(iri)?,
                Term::Str(_) => return None,
            };
        }
        Some((out_row[0], out_row[1]))
    };

    match body.len() {
        1 => {
            let a = body[0];
            let Some(tuples) = world.tuples.get(&a.predicate) else {
                return;
            };
            for &(c0, c1) in tuples {
                let mut row: BTreeMap<&str, i64> = BTreeMap::new();
                bind_atom(a, &[c0, c1], &mut row);
                if let Some(t) = head_tuple(&row) {
                    out.insert(t);
                }
            }
        }
        2 => {
            let (l, r) = (body[0], body[1]);
            let Some(l_tuples) = world.tuples.get(&l.predicate) else {
                return;
            };
            let Some(r_tuples) = world.tuples.get(&r.predicate) else {
                return;
            };
            for &(lc0, lc1) in l_tuples {
                let mut row_l: BTreeMap<&str, i64> = BTreeMap::new();
                bind_atom(l, &[lc0, lc1], &mut row_l);
                for &(rc0, rc1) in r_tuples {
                    let mut row = row_l.clone();
                    if !bind_atom_with_check(r, &[rc0, rc1], &mut row) {
                        continue;
                    }
                    if let Some(t) = head_tuple(&row) {
                        out.insert(t);
                    }
                }
            }
        }
        _ => {}
    }
}

fn bind_atom<'a>(atom: &'a super::ast::Atom, row: &[i64], out: &mut BTreeMap<&'a str, i64>) {
    use super::ast::Term;
    for (term, &val) in atom.args.iter().zip(row.iter()) {
        if let Term::Var(name) = term {
            out.insert(name.as_str(), val);
        }
    }
}

/// Like `bind_atom` but fails (returns false) if an existing binding for
/// the same variable disagrees with the new value. Used for the second
/// atom of a two-atom join where the shared variable must be consistent.
fn bind_atom_with_check<'a>(
    atom: &'a super::ast::Atom,
    row: &[i64],
    out: &mut BTreeMap<&'a str, i64>,
) -> bool {
    use super::ast::Term;
    for (term, &val) in atom.args.iter().zip(row.iter()) {
        if let Term::Var(name) = term {
            match out.get(name.as_str()) {
                Some(existing) if *existing != val => return false,
                _ => {
                    out.insert(name.as_str(), val);
                }
            }
        }
    }
    true
}

// ── World: term-id cache + per-predicate tuples ───────────────

struct World {
    /// Predicate IRI → set of `(entity, value_ref)` tuples currently known
    /// to hold (base facts + derivations from lower strata).
    tuples: BTreeMap<String, BTreeSet<(i64, i64)>>,
    /// Predicate IRI → attribute term id. Populated lazily on first use.
    attr_ids: BTreeMap<String, i64>,
    /// IRI → term id for constants referenced in rule heads.
    const_ids: BTreeMap<String, i64>,
}

impl World {
    fn load(store: &Store, ruleset: &RuleSet) -> Result<Self> {
        let mut preds: BTreeSet<String> = BTreeSet::new();
        for rule in &ruleset.rules {
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

        // Load all current facts once; partition by predicate.
        let facts = store.current_facts()?;
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

        // Intern constants used in rule heads so we can emit `Value::Ref`
        // for them. Constants that don't exist yet are fine for compilation
        // but any rule that references them will be rejected at compile
        // time — see `head_slot` in `compile.rs`.
        let mut const_ids: BTreeMap<String, i64> = BTreeMap::new();
        for rule in &ruleset.rules {
            for term in &rule.head.args {
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
    world: &mut World,
) -> Result<(usize, usize)> {
    let attr_id = world.ensure_attr(store, &rule.head.predicate)?;
    let source = format!("reasoner:{}", rule.id);

    let old_tuples = load_existing_derivations(store, attr_id, &source)?;

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
    store.transact(&datums, timestamp, Some("reasoner"), Some(&source))?;
    Ok((asserted, retracted))
}

/// Load the currently-asserted tuples derived by `source` (typically
/// `reasoner:<rule-id>`). Only reference-valued facts on the rule's head
/// attribute are considered — other shapes cannot be produced by Phase 2
/// and must not exist under this source.
fn load_existing_derivations(
    store: &Store,
    attr_id: i64,
    source: &str,
) -> Result<BTreeSet<(i64, i64)>> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT f.e, f.v FROM facts f \
             JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL \
               AND f.a = ?1 AND t.source = ?2",
        )
        .map_err(crate::Error::from)?;
    let mut rows = stmt
        .query(params![attr_id, source])
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
