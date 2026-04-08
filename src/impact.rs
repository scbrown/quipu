//! Impact analysis — bounded transitive walk over entity→entity edges.
//!
//! Phase 1 of the reasoner rollout (see `docs/design/reasoner.md`). This
//! answers the motivating question — "what is downstream of this entity?" —
//! using only the existing EAVT fact log, without the rule engine.
//!
//! It proves the question is *answerable* on current data, and surfaces
//! ontology gaps: if a walk from a package doesn't reach the service it
//! obviously depends on, the missing edge is in the ontology, not the query.
//!
//! The implementation is a deliberate subset of what SPARQL property paths
//! can express: a breadth-first walk bounded by `--hops`. Property paths
//! cannot express a depth cap, so we walk the store directly. The reasoner
//! (Phase 2+) and counterfactual queries (Phase 4) will build on the SPARQL
//! engine instead.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

/// Default BFS depth when no `--hops` is supplied.
pub const DEFAULT_HOPS: usize = 5;

/// A single entity reached during an impact walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactNode {
    /// IRI of the reached entity.
    pub iri: String,
    /// Distance from the root in edge hops (root itself is depth 0).
    pub depth: usize,
    /// Predicate of the edge that reached this node at its discovered depth.
    /// `None` only for the root.
    pub via_predicate: Option<String>,
    /// Subject entity of the edge that reached this node at its discovered depth.
    /// `None` only for the root.
    pub via_subject: Option<String>,
}

/// Result of an impact walk.
#[derive(Debug, Clone)]
pub struct ImpactReport {
    /// IRI of the entity the walk started from.
    pub root: String,
    /// Entities reached, including the root at depth 0. Ordered by depth
    /// then discovery order.
    pub reached: Vec<ImpactNode>,
    /// The depth bound that was used for this walk.
    pub hops: usize,
    /// Total number of outgoing reference edges examined during the walk.
    pub edges_traversed: usize,
}

/// Options controlling an impact walk.
#[derive(Debug, Clone)]
pub struct ImpactOptions {
    /// Maximum number of edge hops to follow from the root.
    pub hops: usize,
    /// Restrict the walk to these predicate IRIs. Empty = follow all edges.
    pub predicates: Vec<String>,
}

impl Default for ImpactOptions {
    fn default() -> Self {
        Self {
            hops: DEFAULT_HOPS,
            predicates: Vec::new(),
        }
    }
}

/// Walk the store outward from `entity_iri`, returning all reachable
/// entities up to `opts.hops` hops away.
///
/// Only reference-valued facts (edges between entities) are followed;
/// literal-valued facts are ignored — they are not "impact" in the
/// sense of propagating effect to another entity.
pub fn impact(store: &Store, entity_iri: &str, opts: &ImpactOptions) -> Result<ImpactReport> {
    let root_id = store
        .lookup(entity_iri)?
        .ok_or_else(|| Error::InvalidValue(format!("entity not found: {entity_iri}")))?;

    // Resolve optional predicate filter once.
    let predicate_filter: Option<HashSet<i64>> = if opts.predicates.is_empty() {
        None
    } else {
        let mut ids = HashSet::new();
        for p in &opts.predicates {
            if let Some(pid) = store.lookup(p)? {
                ids.insert(pid);
            }
            // Unknown predicates silently contribute nothing — same behaviour
            // as SPARQL: a predicate that doesn't exist in the store has no
            // bindings, it's not an error.
        }
        Some(ids)
    };

    let mut reached = vec![ImpactNode {
        iri: entity_iri.to_string(),
        depth: 0,
        via_predicate: None,
        via_subject: None,
    }];
    let mut visited: HashSet<i64> = HashSet::from([root_id]);
    let mut frontier: Vec<i64> = vec![root_id];
    let mut edges_traversed: usize = 0;

    for depth in 1..=opts.hops {
        if frontier.is_empty() {
            break;
        }
        let mut next_frontier: Vec<i64> = Vec::new();
        for &current in &frontier {
            let current_iri = store.resolve(current)?;
            let facts = store.entity_facts(current)?;
            for fact in &facts {
                // Honour predicate filter.
                if let Some(ref allowed) = predicate_filter
                    && !allowed.contains(&fact.attribute)
                {
                    continue;
                }
                // Only follow entity→entity edges.
                let Value::Ref(target_id) = fact.value else {
                    continue;
                };
                edges_traversed += 1;
                if !visited.insert(target_id) {
                    continue;
                }
                let target_iri = store.resolve(target_id)?;
                let predicate_iri = store.resolve(fact.attribute)?;
                reached.push(ImpactNode {
                    iri: target_iri,
                    depth,
                    via_predicate: Some(predicate_iri),
                    via_subject: Some(current_iri.clone()),
                });
                next_frontier.push(target_id);
            }
        }
        frontier = next_frontier;
    }

    Ok(ImpactReport {
        root: entity_iri.to_string(),
        reached,
        hops: opts.hops,
        edges_traversed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Datum;
    use crate::types::Op;

    const TS: &str = "2026-04-07T00:00:00Z";

    /// Create an entity and return its id.
    fn intern(store: &mut Store, iri: &str) -> i64 {
        store.intern(iri).expect("intern")
    }

    /// Assert an entity→entity edge.
    fn assert_edge(store: &mut Store, s: &str, p: &str, o: &str) {
        let sid = intern(store, s);
        let pid = intern(store, p);
        let oid = intern(store, o);
        let datum = Datum {
            entity: sid,
            attribute: pid,
            value: Value::Ref(oid),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        };
        store
            .transact(&[datum], TS, None, Some("test"))
            .expect("transact");
    }

    /// Assert a literal (should be ignored by the walker).
    fn assert_literal(store: &mut Store, s: &str, p: &str, v: Value) {
        let sid = intern(store, s);
        let pid = intern(store, p);
        let datum = Datum {
            entity: sid,
            attribute: pid,
            value: v,
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        };
        store
            .transact(&[datum], TS, None, Some("test"))
            .expect("transact");
    }

    fn open_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open(tmp.path().to_str().unwrap()).unwrap()
    }

    #[test]
    fn unknown_entity_errors() {
        let store = open_store();
        let err = impact(&store, "http://ex/nope", &ImpactOptions::default()).unwrap_err();
        assert!(
            matches!(err, Error::InvalidValue(_)),
            "expected InvalidValue, got {err:?}"
        );
    }

    #[test]
    fn root_only_when_entity_has_no_edges() {
        let mut store = open_store();
        intern(&mut store, "http://ex/a");
        let report = impact(&store, "http://ex/a", &ImpactOptions::default()).unwrap();
        assert_eq!(report.reached.len(), 1);
        assert_eq!(report.reached[0].iri, "http://ex/a");
        assert_eq!(report.reached[0].depth, 0);
        assert_eq!(report.reached[0].via_predicate, None);
        assert_eq!(report.edges_traversed, 0);
    }

    #[test]
    fn literals_are_ignored() {
        let mut store = open_store();
        intern(&mut store, "http://ex/a");
        assert_literal(
            &mut store,
            "http://ex/a",
            "http://ex/label",
            Value::Str("hello".to_string()),
        );
        let report = impact(&store, "http://ex/a", &ImpactOptions::default()).unwrap();
        assert_eq!(report.reached.len(), 1, "literals should not expand walk");
        assert_eq!(report.edges_traversed, 0);
    }

    #[test]
    fn single_hop_reaches_neighbour() {
        let mut store = open_store();
        assert_edge(
            &mut store,
            "http://ex/pkg",
            "http://ex/installedIn",
            "http://ex/ct",
        );
        let report = impact(&store, "http://ex/pkg", &ImpactOptions::default()).unwrap();
        assert_eq!(report.reached.len(), 2);
        let ct = &report.reached[1];
        assert_eq!(ct.iri, "http://ex/ct");
        assert_eq!(ct.depth, 1);
        assert_eq!(ct.via_predicate.as_deref(), Some("http://ex/installedIn"));
        assert_eq!(ct.via_subject.as_deref(), Some("http://ex/pkg"));
    }

    #[test]
    fn transitive_walk_visits_depth_then_breadth() {
        let mut store = open_store();
        // pkg -installedIn-> ct -runsService-> svc -runningOn-> host
        assert_edge(
            &mut store,
            "http://ex/pkg",
            "http://ex/installedIn",
            "http://ex/ct",
        );
        assert_edge(
            &mut store,
            "http://ex/ct",
            "http://ex/runsService",
            "http://ex/svc",
        );
        assert_edge(
            &mut store,
            "http://ex/svc",
            "http://ex/runningOn",
            "http://ex/host",
        );

        let report = impact(&store, "http://ex/pkg", &ImpactOptions::default()).unwrap();
        let iris: Vec<&str> = report.reached.iter().map(|n| n.iri.as_str()).collect();
        assert_eq!(
            iris,
            vec![
                "http://ex/pkg",
                "http://ex/ct",
                "http://ex/svc",
                "http://ex/host",
            ]
        );
        let depths: Vec<usize> = report.reached.iter().map(|n| n.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 3]);
    }

    #[test]
    fn hops_bound_is_enforced() {
        let mut store = open_store();
        assert_edge(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");
        assert_edge(&mut store, "http://ex/b", "http://ex/p", "http://ex/c");
        assert_edge(&mut store, "http://ex/c", "http://ex/p", "http://ex/d");

        let opts = ImpactOptions {
            hops: 2,
            ..Default::default()
        };
        let report = impact(&store, "http://ex/a", &opts).unwrap();
        let iris: Vec<&str> = report.reached.iter().map(|n| n.iri.as_str()).collect();
        assert_eq!(iris, vec!["http://ex/a", "http://ex/b", "http://ex/c"]);
        assert!(
            !iris.contains(&"http://ex/d"),
            "d is at depth 3, should be excluded by hops=2"
        );
    }

    #[test]
    fn cycles_terminate() {
        let mut store = open_store();
        // a ↔ b cycle.
        assert_edge(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");
        assert_edge(&mut store, "http://ex/b", "http://ex/p", "http://ex/a");

        let opts = ImpactOptions {
            hops: 10,
            ..Default::default()
        };
        let report = impact(&store, "http://ex/a", &opts).unwrap();
        assert_eq!(report.reached.len(), 2);
    }

    #[test]
    fn predicate_filter_restricts_walk() {
        let mut store = open_store();
        assert_edge(&mut store, "http://ex/a", "http://ex/good", "http://ex/b");
        assert_edge(&mut store, "http://ex/a", "http://ex/bad", "http://ex/c");

        let opts = ImpactOptions {
            hops: 5,
            predicates: vec!["http://ex/good".to_string()],
        };
        let report = impact(&store, "http://ex/a", &opts).unwrap();
        let iris: Vec<&str> = report.reached.iter().map(|n| n.iri.as_str()).collect();
        assert_eq!(iris, vec!["http://ex/a", "http://ex/b"]);
    }

    #[test]
    fn unknown_predicate_filter_matches_nothing() {
        let mut store = open_store();
        assert_edge(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");

        let opts = ImpactOptions {
            hops: 5,
            predicates: vec!["http://ex/never-created".to_string()],
        };
        let report = impact(&store, "http://ex/a", &opts).unwrap();
        assert_eq!(
            report.reached.len(),
            1,
            "only the root should appear when no predicates match"
        );
    }

    #[test]
    fn branching_fan_out() {
        let mut store = open_store();
        // a → {b, c, d}, b → e
        assert_edge(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");
        assert_edge(&mut store, "http://ex/a", "http://ex/p", "http://ex/c");
        assert_edge(&mut store, "http://ex/a", "http://ex/p", "http://ex/d");
        assert_edge(&mut store, "http://ex/b", "http://ex/p", "http://ex/e");

        let report = impact(&store, "http://ex/a", &ImpactOptions::default()).unwrap();
        assert_eq!(report.reached.len(), 5);
        // Root at depth 0, b/c/d at depth 1, e at depth 2.
        let depths_at_1: usize = report.reached.iter().filter(|n| n.depth == 1).count();
        assert_eq!(depths_at_1, 3);
        let depths_at_2: usize = report.reached.iter().filter(|n| n.depth == 2).count();
        assert_eq!(depths_at_2, 1);
    }
}
