//! Reactive reasoner — Phase 3 of the reasoner rollout.
//!
//! [`ReactiveReasoner`] implements [`TransactObserver`] so derived facts
//! stay fresh automatically as base facts change. When a transaction
//! commits, the observer inspects the delta to find which predicates
//! were touched, maps those to affected rules via a pre-built index,
//! computes the transitive closure of dependent rules, partitions by
//! stratum, and re-runs only the affected strata.
//!
//! Truth maintenance is re-derive-and-diff: each affected rule is fully
//! re-derived from the current world state, and the result is diffed
//! against previously-stored derivations. New tuples are asserted;
//! disappeared tuples are retracted. Full incremental TMS (tracking
//! individual derivation support sets) is deferred to Phase 5.

use std::collections::{BTreeSet, HashMap};
use std::sync::RwLock;

use super::evaluate;
use super::parse::RuleSet;
use super::stratify;
use crate::store::{Datum, Delta, Store, TransactObserver};

/// A reactive reasoner that re-derives affected rules when base facts change.
///
/// Register with [`Store::add_observer`] after loading the ruleset. The
/// observer skips transactions whose `source` starts with `"reasoner:"`
/// to avoid re-triggering on its own output.
///
/// The ruleset is swappable at runtime via [`ReactiveReasoner::reload`], so a
/// long-running server can pick up rules loaded through `POST /shapes` without
/// a restart (gap G6 of `docs/design/semantic-reasoning-gaps.md` — the startup
/// snapshot that "bit this workstream five times").
pub struct ReactiveReasoner {
    /// The ruleset and its derived indexes, swapped atomically on reload so
    /// `after_commit` never sees a ruleset paired with another ruleset's
    /// indexes.
    index: RwLock<RuleIndex>,
    /// Tracks total reactive evaluation stats across the session.
    stats: RwLock<ReactiveStats>,
}

/// A ruleset with the pre-built lookup structures `after_commit` needs.
struct RuleIndex {
    /// The loaded ruleset.
    ruleset: RuleSet,
    /// Maps predicate IRI → indices into `ruleset.rules` whose body
    /// references that predicate.
    pred_to_rules: HashMap<String, Vec<usize>>,
    /// Maps rule index → indices of rules that transitively depend on it
    /// (rules whose body references a predicate that appears in this
    /// rule's head). Pre-computed so `after_commit` is a cheap lookup.
    rule_dependents: HashMap<usize, Vec<usize>>,
}

impl RuleIndex {
    fn new(ruleset: RuleSet) -> Self {
        let pred_to_rules = build_pred_index(&ruleset);
        let rule_dependents = build_rule_dependents(&ruleset);
        Self {
            ruleset,
            pred_to_rules,
            rule_dependents,
        }
    }
}

/// Cumulative statistics for reactive evaluations.
#[derive(Debug, Clone, Default)]
pub struct ReactiveStats {
    /// Number of times `after_commit` fired and found work.
    pub triggers: usize,
    /// Total facts asserted across all reactive evaluations.
    pub total_asserted: usize,
    /// Total facts retracted across all reactive evaluations.
    pub total_retracted: usize,
}

impl ReactiveReasoner {
    /// Build a reactive reasoner from a parsed ruleset.
    ///
    /// Constructs the predicate-to-rule index and the rule dependency
    /// graph used to compute the transitive closure of affected rules.
    /// An empty ruleset is a valid starting state: the observer is a no-op
    /// until [`ReactiveReasoner::reload`] hands it rules.
    pub fn new(ruleset: RuleSet) -> Self {
        Self {
            index: RwLock::new(RuleIndex::new(ruleset)),
            stats: RwLock::new(ReactiveStats::default()),
        }
    }

    /// Replace the ruleset (and its derived indexes) atomically.
    ///
    /// This is what makes rules loaded through `POST /shapes` take effect
    /// without a server restart: the server keeps an `Arc` to the registered
    /// observer and calls this after a successful shapes write. Later
    /// `after_commit` calls see the new ruleset; the swap never mixes one
    /// ruleset with another's indexes.
    pub fn reload(&self, ruleset: RuleSet) {
        *self.index.write().expect("rule index lock poisoned") = RuleIndex::new(ruleset);
    }

    /// Number of rules currently loaded.
    pub fn rule_count(&self) -> usize {
        self.index
            .read()
            .expect("rule index lock poisoned")
            .ruleset
            .len()
    }

    /// Return the current reactive evaluation statistics.
    pub fn stats(&self) -> ReactiveStats {
        self.stats.read().expect("stats lock poisoned").clone()
    }
}

impl RuleIndex {
    /// Determine which rule indices are affected by a set of changed
    /// predicate IRIs, including transitive dependents.
    fn affected_rules(&self, changed_preds: &BTreeSet<String>) -> BTreeSet<usize> {
        let mut affected = BTreeSet::new();

        // Direct: rules whose body references a changed predicate.
        for pred in changed_preds {
            if let Some(indices) = self.pred_to_rules.get(pred) {
                for &idx in indices {
                    affected.insert(idx);
                }
            }
        }

        // Transitive closure: if rule R is affected and its head predicate
        // appears in another rule's body, that rule is also affected.
        let mut frontier: Vec<usize> = affected.iter().copied().collect();
        while let Some(idx) = frontier.pop() {
            if let Some(deps) = self.rule_dependents.get(&idx) {
                for &dep_idx in deps {
                    if affected.insert(dep_idx) {
                        frontier.push(dep_idx);
                    }
                }
            }
        }

        affected
    }
}

impl TransactObserver for ReactiveReasoner {
    fn after_commit(&self, store: &mut Store, delta: &Delta) -> crate::error::Result<()> {
        // Skip our own output — and the inferred plane's bookkeeping writes
        // (freshness notes, tags): reacting to those could loop if a rule
        // ever ranged over the note predicate.
        if let Some(src) = &delta.source
            && (src.starts_with("reasoner:")
                || src == crate::store::inferred::PLANE_SOURCE
                || src == crate::store::inferred::MIGRATE_SOURCE)
        {
            return Ok(());
        }

        // Collect predicate IRIs that were touched in this delta.
        let mut changed_attrs: BTreeSet<i64> = BTreeSet::new();
        for d in &delta.asserts {
            changed_attrs.insert(d.attribute);
        }
        for d in &delta.retracts {
            changed_attrs.insert(d.attribute);
        }

        if changed_attrs.is_empty() {
            return Ok(());
        }

        // Resolve attribute IDs to predicate IRIs.
        let mut changed_preds = BTreeSet::new();
        for &attr_id in &changed_attrs {
            if let Ok(iri) = store.resolve(attr_id) {
                changed_preds.insert(iri);
            }
        }

        // Hold the read lock across the whole evaluation so a concurrent
        // reload cannot swap the ruleset out from under the affected-set.
        let index = self.index.read().expect("rule index lock poisoned");
        let affected = index.affected_rules(&changed_preds);
        if affected.is_empty() {
            return Ok(());
        }

        // Re-derive affected rules and commit per-rule with proper
        // `reasoner:<rule-id>` provenance, matching the full-evaluate path.
        let report = evaluate_affected(store, &index.ruleset, &affected)?;

        // Update stats.
        if let Ok(mut stats) = self.stats.write() {
            stats.triggers += 1;
            stats.total_asserted += report.asserted;
            stats.total_retracted += report.retracted;
        }

        Ok(())
    }
}

/// Result of a reactive evaluation pass.
struct ReactiveReport {
    asserted: usize,
    retracted: usize,
}

/// Re-derive affected rules and commit per-rule with proper provenance.
///
/// This is the core of the reactive path. It loads the current world state,
/// runs only the affected rules, diffs the result against existing
/// derivations, and commits each rule's delta through `store.transact()`
/// with `source = reasoner:<rule-id>` — matching the full-evaluate path.
fn evaluate_affected(
    store: &mut Store,
    ruleset: &RuleSet,
    affected: &BTreeSet<usize>,
) -> crate::error::Result<ReactiveReport> {
    use crate::types::{Op, Value};

    let strata = stratify::stratify(ruleset).map_err(|e| crate::Error::Store(e.to_string()))?;

    // Determine which strata contain affected rules.
    let affected_strata: BTreeSet<usize> = strata
        .levels
        .iter()
        .enumerate()
        .filter(|(_idx, rule_indices): &(usize, &Vec<usize>)| {
            rule_indices.iter().any(|idx| affected.contains(idx))
        })
        .map(|(stratum_idx, _)| stratum_idx)
        .collect();

    if affected_strata.is_empty() {
        return Ok(ReactiveReport {
            asserted: 0,
            retracted: 0,
        });
    }

    let mut total_asserted = 0_usize;
    let mut total_retracted = 0_usize;
    let timestamp = "reactive";

    // Placement (quipu-0b6): reactive derivation reads premises from ROOT
    // plus its companion inferred graph and writes derivations to the
    // companion, matching the full-evaluate path.
    let companion = store
        .ensure_companion_inferred_graph(crate::schema::ROOT_GRAPH, timestamp)
        .map_err(|e| crate::Error::Store(e.to_string()))?;
    let premise_head = store.transaction_head()?;

    for stratum_idx in &affected_strata {
        // Reload the world before each stratum so that derived facts
        // from earlier strata (committed to the store) are visible to
        // rules in later strata.
        let rule_indices: Vec<usize> = strata.levels[*stratum_idx]
            .iter()
            .copied()
            .filter(|idx| affected.contains(idx))
            .collect();
        let world = evaluate::World::load_graphs_rule_indices(
            store,
            ruleset,
            &[crate::schema::ROOT_GRAPH, companion],
            &rule_indices,
        )
        .map_err(|e| crate::Error::Store(e.to_string()))?;

        for &rule_idx in &rule_indices {
            let rule = &ruleset.rules[rule_idx];

            // Compute what the rule derives from the current world.
            let new_tuples = evaluate::project_rule_from_world(rule, &world);

            // Ensure the head predicate attribute id exists.
            let attr_id = store.intern(&rule.head.predicate)?;
            let source = format!("reasoner:{}", rule.id);
            let old_tuples =
                evaluate::load_existing_derivations_in_graph(store, attr_id, &source, companion)
                    .map_err(|e| crate::Error::Store(e.to_string()))?;

            // Build the diff datums for this rule.
            let mut datums = Vec::new();
            for &(e, v) in old_tuples.difference(&new_tuples) {
                datums.push(Datum {
                    entity: e,
                    attribute: attr_id,
                    value: Value::Ref(v),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Retract,
                });
            }
            for &(e, v) in new_tuples.difference(&old_tuples) {
                datums.push(Datum {
                    entity: e,
                    attribute: attr_id,
                    value: Value::Ref(v),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                });
            }

            if datums.is_empty() {
                continue;
            }

            let asserted = datums.iter().filter(|d| d.op == Op::Assert).count();
            let retracted = datums.iter().filter(|d| d.op == Op::Retract).count();
            store.transact_to_graph(
                &datums,
                timestamp,
                Some("reasoner"),
                Some(&source),
                companion,
            )?;
            total_asserted += asserted;
            total_retracted += retracted;
        }
    }

    store
        .note_inferred_freshness(companion, premise_head, timestamp)
        .map_err(|e| crate::Error::Store(e.to_string()))?;

    Ok(ReactiveReport {
        asserted: total_asserted,
        retracted: total_retracted,
    })
}

// ── Index construction ────────────────────────────────────────

/// Build predicate IRI → rule indices for body predicates.
fn build_pred_index(ruleset: &RuleSet) -> HashMap<String, Vec<usize>> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, rule) in ruleset.rules.iter().enumerate() {
        for body in &rule.body {
            index
                .entry(body.atom().predicate.clone())
                .or_default()
                .push(idx);
        }
    }
    index
}

/// Build rule → dependent rules mapping.
///
/// If rule A's head predicate appears in rule B's body, then B depends
/// on A. When A is affected, B must also be re-evaluated.
fn build_rule_dependents(ruleset: &RuleSet) -> HashMap<usize, Vec<usize>> {
    // head predicate → rule index that produces it
    let mut head_to_rule: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, rule) in ruleset.rules.iter().enumerate() {
        head_to_rule
            .entry(rule.head.predicate.as_str())
            .or_default()
            .push(idx);
    }

    // For each rule, find rules whose body references another rule's head.
    let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();
    for (consumer_idx, consumer_rule) in ruleset.rules.iter().enumerate() {
        for body in &consumer_rule.body {
            let pred = body.atom().predicate.as_str();
            if let Some(producers) = head_to_rule.get(pred) {
                for &producer_idx in producers {
                    if producer_idx != consumer_idx {
                        dependents
                            .entry(producer_idx)
                            .or_default()
                            .push(consumer_idx);
                    }
                }
            }
        }
    }

    // Deduplicate.
    for deps in dependents.values_mut() {
        deps.sort_unstable();
        deps.dedup();
    }

    dependents
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoner::RULE_NS;
    use crate::reasoner::parse::parse_rules;

    const PFX: &str = "http://ex/";

    fn make_ruleset(ttl: &str) -> RuleSet {
        parse_rules(ttl, Some(PFX)).unwrap()
    }

    #[test]
    fn pred_index_maps_body_predicates_to_rules() {
        let ttl = format!(
            r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ; rule:id "R1" ;
    rule:head "h(?x, ?y)" ; rule:body "p(?x, ?y)" .
ex:r2 a rule:Rule ; rule:id "R2" ;
    rule:head "g(?x, ?y)" ; rule:body "p(?x, ?z), q(?z, ?y)" .
"#
        );
        let rs = make_ruleset(&ttl);
        let idx = build_pred_index(&rs);

        // p appears in both rules
        let p_rules = idx.get(&format!("{PFX}p")).unwrap();
        assert!(p_rules.contains(&0));
        assert!(p_rules.contains(&1));

        // q appears only in R2
        let q_rules = idx.get(&format!("{PFX}q")).unwrap();
        assert_eq!(q_rules, &[1]);
    }

    #[test]
    fn rule_dependents_captures_transitive_chains() {
        // R1: h :- p  (h is derived from p)
        // R2: g :- h, q  (g depends on h, so R2 depends on R1)
        let ttl = format!(
            r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ; rule:id "R1" ;
    rule:head "h(?x, ?y)" ; rule:body "p(?x, ?y)" .
ex:r2 a rule:Rule ; rule:id "R2" ;
    rule:head "g(?x, ?y)" ; rule:body "h(?x, ?z), q(?z, ?y)" .
"#
        );
        let rs = make_ruleset(&ttl);
        let deps = build_rule_dependents(&rs);

        // R1 (index 0) produces h, which R2 (index 1) consumes
        assert_eq!(deps.get(&0).unwrap(), &[1]);
        // R2 produces g, nothing consumes it
        assert!(!deps.contains_key(&1));
    }

    #[test]
    fn affected_rules_includes_transitive_dependents() {
        let ttl = format!(
            r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ; rule:id "R1" ;
    rule:head "h(?x, ?y)" ; rule:body "p(?x, ?y)" .
ex:r2 a rule:Rule ; rule:id "R2" ;
    rule:head "g(?x, ?y)" ; rule:body "h(?x, ?z), q(?z, ?y)" .
ex:r3 a rule:Rule ; rule:id "R3" ;
    rule:head "f(?x, ?y)" ; rule:body "g(?x, ?y)" .
"#
        );
        let rs = make_ruleset(&ttl);
        let reasoner = ReactiveReasoner::new(rs);
        let index = reasoner.index.read().unwrap();

        // Changing p should affect R1, R2, R3 (transitive chain)
        let mut changed = BTreeSet::new();
        changed.insert(format!("{PFX}p"));
        let affected = index.affected_rules(&changed);
        assert!(affected.contains(&0)); // R1
        assert!(affected.contains(&1)); // R2
        assert!(affected.contains(&2)); // R3

        // Changing q should affect only R2 and R3 (not R1)
        let mut changed_q = BTreeSet::new();
        changed_q.insert(format!("{PFX}q"));
        let affected_q = index.affected_rules(&changed_q);
        assert!(!affected_q.contains(&0)); // R1 unaffected
        assert!(affected_q.contains(&1)); // R2
        assert!(affected_q.contains(&2)); // R3
    }

    /// `reload` must swap the ruleset AND its indexes atomically (quipu-923,
    /// gap G6): a reasoner registered with no rules starts deriving once a
    /// ruleset is loaded, with no re-registration and no restart.
    #[test]
    fn reload_swaps_ruleset_and_indexes() {
        let reasoner = ReactiveReasoner::new(RuleSet::empty(PFX));
        assert_eq!(reasoner.rule_count(), 0);

        let mut changed = BTreeSet::new();
        changed.insert(format!("{PFX}p"));
        assert!(
            reasoner
                .index
                .read()
                .unwrap()
                .affected_rules(&changed)
                .is_empty(),
            "an empty reasoner has nothing to affect"
        );

        let ttl = format!(
            r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ; rule:id "R1" ;
    rule:head "h(?x, ?y)" ; rule:body "p(?x, ?y)" .
"#
        );
        reasoner.reload(make_ruleset(&ttl));
        assert_eq!(reasoner.rule_count(), 1);
        let affected = reasoner.index.read().unwrap().affected_rules(&changed);
        assert!(
            affected.contains(&0),
            "after reload the new rule must be reachable from its body predicate"
        );
    }
}
