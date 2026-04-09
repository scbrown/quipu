//! Stratification of a loaded ruleset.
//!
//! Builds a predicate dependency graph from the rules, computes strongly
//! connected components (SCCs), and assigns a stratum to each predicate.
//! A ruleset is **stratifiable** iff no SCC contains a negative edge.
//!
//! # Semantics
//!
//! For a rule `H :- L1, L2, ..., Ln`, we add a directed edge from every
//! body predicate to the head predicate, labelled positive or negative
//! according to whether that body literal is negated. The classic
//! stratification condition is then:
//!
//! - Positive edges may live inside an SCC (positive recursion is fine —
//!   semi-naive evaluation converges in a finite number of steps over a
//!   finite Herbrand universe).
//! - Negative edges may only cross strata: `stratum(body) < stratum(head)`.
//!   A negative edge inside an SCC is a cycle through negation and rejected.
//!
//! # Output shape
//!
//! Stratum 0 contains predicates that depend on nothing (or only on other
//! stratum-0 predicates). Stratum N depends on strata `< N`. Rules are
//! assigned to the stratum of their head predicate. Strata are returned
//! as a `Vec<Vec<usize>>` of rule indices into `RuleSet::rules`, ordered
//! lowest first.
//!
//! Extensional predicates (those that appear only in rule bodies, never in
//! any head) are considered stratum 0 implicitly; they contribute no rules.
//!
//! See `docs/design/reasoner.md` § "Stratification termination proof" for
//! the full justification.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

use super::parse::RuleSet;
use super::{ReasonerError, Result};

/// Result of a successful stratification.
#[derive(Debug, Clone)]
pub struct Strata {
    /// Rules grouped by stratum, lowest first. Each inner `Vec` holds
    /// indices into `RuleSet::rules`.
    pub levels: Vec<Vec<usize>>,
    /// Predicate → stratum number. Includes extensional predicates (which
    /// always sit at stratum 0).
    pub predicate_stratum: BTreeMap<String, usize>,
}

impl Strata {
    /// Number of stratum levels with at least one rule.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }
}

/// Stratify a ruleset.
///
/// Returns [`ReasonerError::UnstratifiableCycle`] if any strongly connected
/// component in the predicate dependency graph contains a negative edge.
pub fn stratify(ruleset: &RuleSet) -> Result<Strata> {
    // Collect every predicate mentioned anywhere.
    let mut index: HashMap<String, NodeIndex> = HashMap::new();
    let mut graph: DiGraph<String, EdgeKind> = DiGraph::new();

    // Pre-intern all predicates referenced by the rules so every edge can
    // be added to a known node.
    for rule in &ruleset.rules {
        intern_pred(&mut graph, &mut index, &rule.head.predicate);
        for body in &rule.body {
            intern_pred(&mut graph, &mut index, &body.atom().predicate);
        }
    }

    // Add edges body → head, labelled by polarity. A rule with N body
    // atoms contributes N edges, one per atom.
    for rule in &ruleset.rules {
        let head_node = index[&rule.head.predicate];
        for body in &rule.body {
            let body_node = index[&body.atom().predicate];
            let kind = if body.is_positive() {
                EdgeKind::Positive
            } else {
                EdgeKind::Negative
            };
            graph.add_edge(body_node, head_node, kind);
        }
    }

    // Tarjan's SCC returns components in reverse topological order: sinks
    // first, sources last. We reverse at the end so stratum 0 means "no
    // upstream dependencies".
    let sccs_reverse_topo = tarjan_scc(&graph);

    // Map each node to its SCC index. SCC index mirrors the order in
    // `sccs_reverse_topo` so we can detect cycle membership cheaply.
    let mut node_to_scc: HashMap<NodeIndex, usize> = HashMap::new();
    for (scc_idx, scc) in sccs_reverse_topo.iter().enumerate() {
        for node in scc {
            node_to_scc.insert(*node, scc_idx);
        }
    }

    // Check the stratification condition: no negative edge inside any SCC.
    for edge in graph.edge_references() {
        if *edge.weight() == EdgeKind::Negative {
            let src_scc = node_to_scc[&edge.source()];
            let dst_scc = node_to_scc[&edge.target()];
            if src_scc == dst_scc && sccs_reverse_topo[src_scc].len() > 1 {
                // True SCC with multiple predicates.
                return Err(cycle_error(&graph, &sccs_reverse_topo[src_scc]));
            }
            if src_scc == dst_scc && edge.source() == edge.target() {
                // Self-loop through negation (e.g. `p :- not p`).
                return Err(cycle_error(&graph, &sccs_reverse_topo[src_scc]));
            }
        }
    }

    // Stratum = position in the topological order of the SCC condensation.
    // tarjan_scc returns components in reverse topo order, so the last SCC
    // in the returned vec depends on nothing and becomes stratum 0.
    let scc_count = sccs_reverse_topo.len();
    let stratum_of_scc = |scc_idx: usize| -> usize { scc_count - 1 - scc_idx };

    // Build predicate → stratum map.
    let mut predicate_stratum: BTreeMap<String, usize> = BTreeMap::new();
    for (pred, node) in &index {
        let scc_idx = node_to_scc[node];
        predicate_stratum.insert(pred.clone(), stratum_of_scc(scc_idx));
    }

    // Group rules by the stratum of their head predicate.
    let mut by_stratum: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (rule_idx, rule) in ruleset.rules.iter().enumerate() {
        let stratum = predicate_stratum[&rule.head.predicate];
        by_stratum.entry(stratum).or_default().push(rule_idx);
    }

    // Dense vector keyed by stratum. Produce contiguous indices 0..max so
    // stratum levels are iterable in order even if some numbers are empty.
    let max_stratum = by_stratum.keys().max().copied().unwrap_or(0);
    let mut levels: Vec<Vec<usize>> = Vec::with_capacity(max_stratum + 1);
    for s in 0..=max_stratum {
        levels.push(by_stratum.remove(&s).unwrap_or_default());
    }
    // Drop trailing empty levels so `level_count` is meaningful.
    while levels.last().is_some_and(Vec::is_empty) {
        levels.pop();
    }

    Ok(Strata {
        levels,
        predicate_stratum,
    })
}

// ── internals ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    Positive,
    Negative,
}

fn intern_pred(
    graph: &mut DiGraph<String, EdgeKind>,
    index: &mut HashMap<String, NodeIndex>,
    pred: &str,
) {
    if !index.contains_key(pred) {
        let node = graph.add_node(pred.to_string());
        index.insert(pred.to_string(), node);
    }
}

fn cycle_error(graph: &DiGraph<String, EdgeKind>, scc: &[NodeIndex]) -> ReasonerError {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for node in scc {
        names.insert(graph[*node].clone());
    }
    ReasonerError::UnstratifiableCycle {
        predicates: names.into_iter().collect(),
    }
}

/// Convenience: return the number of rules allocated to each stratum.
///
/// Useful for CLI progress reporting.
#[must_use]
pub fn rules_per_stratum(strata: &Strata) -> Vec<usize> {
    strata.levels.iter().map(Vec::len).collect()
}

/// Free-standing helper for testing `Strata` without plumbing through the
/// rest of the module.
#[cfg(test)]
pub(crate) fn head_predicates_at(
    strata: &Strata,
    ruleset: &RuleSet,
    stratum: usize,
) -> Vec<String> {
    strata
        .levels
        .get(stratum)
        .into_iter()
        .flatten()
        .map(|idx| ruleset.rules[*idx].head.predicate.clone())
        .collect::<Vec<_>>()
}

#[cfg(test)]
pub(crate) fn rule_at_index(rule_set: &RuleSet, idx: usize) -> &super::ast::Rule {
    &rule_set.rules[idx]
}
