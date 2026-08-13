//! `PageRank` surfaces: the `quipu_project` pagerank/PPR arm and score
//! write-back (quipu-mq7, pagerank design Phase 3).

use serde_json::Value as JsonValue;

use super::{PageRankConfig, ProjectedGraph, page_rank};
use crate::error::Result;
use crate::namespace;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// The `"pagerank" | "ppr"` arm of `tool_project` — split from `graph.rs`
/// when Phase 3 grew it (quipu-mq7).
pub(super) fn run_pagerank(
    store: &mut Store,
    pg: &ProjectedGraph,
    input: &JsonValue,
) -> Result<JsonValue> {
    let damping = input
        .get("damping")
        .and_then(serde_json::Value::as_f64)
        .map_or(0.85, |v| v as f32);
    let max_iters = input
        .get("max_iters")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(100) as u32;
    let tolerance = input
        .get("tolerance")
        .and_then(serde_json::Value::as_f64)
        .map_or(1e-6, |v| v as f32);
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(20) as usize;

    // Seeds accepted as IRIs (resolved to term IDs) or raw integer IDs.
    let mut seeds: Vec<i64> = Vec::new();
    if let Some(arr) = input.get("seeds").and_then(|v| v.as_array()) {
        for s in arr {
            if let Some(iri) = s.as_str() {
                if let Some(id) = store.lookup(iri)? {
                    seeds.push(id);
                }
            } else if let Some(id) = s.as_i64() {
                seeds.push(id);
            }
        }
    }

    let personalized = !seeds.is_empty();
    let cfg = PageRankConfig {
        damping,
        seeds,
        max_iters,
        tolerance,
    };
    let ranked = page_rank(pg, &cfg)?;

    // Opt-in persistence (Phase 3), mirroring the louvain arm. GLOBAL scores
    // only: a personalized run is query-specific, and persisting it would
    // supersede the store's global importance with one query's neighbourhood —
    // refused loudly rather than recorded silently.
    let persist = input
        .get("persist")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let persisted = if persist {
        if personalized {
            return Err(crate::Error::InvalidValue(
                "'persist' applies to the global ranking; a seeded (personalized) run is \
                 query-specific and is not persisted"
                    .into(),
            ));
        }
        let now = crate::time::now_iso();
        let timestamp = input
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or(&now);
        Some(persist_pagerank(store, &ranked, timestamp)?)
    } else {
        None
    };

    let results: Vec<JsonValue> = ranked
        .into_iter()
        .take(limit)
        .map(|(entity_id, score)| {
            let iri = store
                .resolve(entity_id)
                .unwrap_or_else(|_| format!("ref:{entity_id}"));
            serde_json::json!({"entity": iri, "score": score})
        })
        .collect();
    Ok(serde_json::json!({
        "algorithm": "pagerank",
        "personalized": personalized,
        "results": results,
        "count": results.len(),
        "persisted": persisted,
    }))
}

/// Persist `PageRank` scores as `quipu:pageRank` facts (Phase 3), superseding
/// any prior derivation — the same reconcile discipline as
/// [`super::persist_communities`]: an unchanged score is left alone (no
/// supersede churn), a changed or vanished entity's score is retracted, and
/// only genuinely-new scores are asserted.
///
/// Scores land as `Value::Float`, SPARQL-orderable (`ORDER BY DESC(?pr)`),
/// bitemporal like any fact — score history per entity is a `valid_at`
/// time-travel away — and reasoner-visible.
///
/// Returns the number of entities whose score is now current.
///
/// # Errors
/// Store errors from interning or the write transaction.
pub fn persist_pagerank(
    store: &mut Store,
    scores: &[(i64, f32)],
    timestamp: &str,
) -> Result<usize> {
    let pred_id = store.intern(&format!("{}pageRank", namespace::QUIPU))?;

    let mut desired: std::collections::HashMap<i64, f64> = scores
        .iter()
        .map(|&(entity, score)| (entity, f64::from(score)))
        .collect();
    let total = desired.len();

    let mut datums: Vec<Datum> = Vec::new();

    // Reconcile against current state: an entity whose stored score equals the
    // new one is dropped from `desired` (nothing to write); everything else
    // currently stored is stale and retracted.
    for fact in store.current_facts()? {
        if fact.attribute != pred_id {
            continue;
        }
        let keep = matches!(
            (&fact.value, desired.get(&fact.entity)),
            (Value::Float(stored), Some(new)) if stored == new
        );
        if keep {
            desired.remove(&fact.entity);
        } else {
            datums.push(Datum {
                entity: fact.entity,
                attribute: pred_id,
                value: fact.value,
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Retract,
            });
        }
    }

    // Assert the changed/new scores (deterministic order).
    let mut to_assert: Vec<(i64, f64)> = desired.into_iter().collect();
    to_assert.sort_unstable_by_key(|a| a.0);
    for (entity, score) in to_assert {
        datums.push(Datum {
            entity,
            attribute: pred_id,
            value: Value::Float(score),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }

    if !datums.is_empty() {
        store.transact(&datums, timestamp, None, Some("algo:pagerank"))?;
    }
    Ok(total)
}

/// Project the ROOT graph AS IT STOOD at a bitemporal point (quipu-bli,
/// pagerank design Phase 5) — `page_rank` over the result ranks the graph as
/// it was, not as it is.
///
/// ROOT-scoped like [`crate::store::AsOf`] reads generally are: time travel
/// scopes within a graph (`docs/design/named-graphs.md` §1).
///
/// # Errors
/// Store errors from the as-of fact scan.
pub fn project_as_of(
    store: &Store,
    type_filter: Option<&str>,
    predicate_filter: Option<&str>,
    as_of: &crate::store::AsOf,
) -> Result<ProjectedGraph> {
    let facts = store.facts_as_of(as_of)?;
    super::project_facts(store, &facts, type_filter, predicate_filter)
}

/// Rank a COUNTERFACTUAL: "how would influence shift if these facts landed?"
/// (quipu-bli, Phase 5). The hypothetical datums are applied inside
/// [`Store::speculate`]'s savepoint, the projection and `PageRank` run against
/// that fork, and the savepoint rolls back — the store is never mutated.
///
/// # Errors
/// Store errors from the speculative write or the projection.
pub fn rank_counterfactual(
    store: &mut Store,
    hypothetical: &[Datum],
    timestamp: &str,
    type_filter: Option<&str>,
    predicate_filter: Option<&str>,
    cfg: &PageRankConfig,
) -> Result<Vec<(i64, f32)>> {
    store.speculate(hypothetical, timestamp, |s| {
        let pg = super::project_in_graph(s, type_filter, predicate_filter, None)?;
        page_rank(&pg, cfg)
    })
}
