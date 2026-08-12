//! MCP tool for impact analysis with optional counterfactual removal.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// MCP tool: `quipu_impact` -- Impact analysis with optional counterfactual removal.
///
/// Input: `{ "entity": "<IRI>", "remove": bool, "hops": N, "predicates": ["<IRI>", ...] }`
/// Output: `{ "root": "...", "reached": [...], "hops": N, "edges": N, "counterfactual": bool }`
///
/// When `remove` is `true`, speculatively retracts all facts for the entity
/// (via `Store::speculate`), runs the reasoner inside the fork, then walks the
/// graph to show what remains reachable. The store is never mutated.
pub fn tool_impact(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let entity_iri = input
        .get("entity")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'entity' IRI parameter".into()))?;

    let remove = input
        .get("remove")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let hops = input
        .get("hops")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(crate::impact::DEFAULT_HOPS as u64) as usize;

    let predicates: Vec<String> = input
        .get("predicates")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let opts = crate::impact::ImpactOptions { hops, predicates };

    let report = if remove {
        let now = crate::time::now_iso();
        let timestamp = input
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or(&now);
        crate::impact::speculate_remove(store, entity_iri, timestamp, |s| {
            crate::impact::impact(s, entity_iri, &opts)
        })?
    } else {
        crate::impact::impact(store, entity_iri, &opts)?
    };

    // Optional PPR re-rank (quipu-mq7, pagerank design Phase 4): order the
    // reached set by Personalized `PageRank` seeded at the root, so the most
    // structurally entangled entities lead instead of BFS discovery order.
    // Each entry carries its `ppr` score; an entity outside the projected
    // graph scores 0 and sorts last, which is itself informative.
    let rank_by_ppr = input
        .get("rank_by_ppr")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let ppr_scores: Option<std::collections::HashMap<String, f32>> = if rank_by_ppr {
        let pg = crate::graph::project_cached(store, None, None, None)?;
        let seed = store.lookup(entity_iri)?.into_iter().collect::<Vec<_>>();
        let cfg = crate::graph::PageRankConfig {
            seeds: seed,
            ..Default::default()
        };
        let ranked = crate::graph::page_rank(&pg, &cfg)?;
        Some(
            ranked
                .into_iter()
                .filter_map(|(id, score)| store.resolve(id).ok().map(|iri| (iri, score)))
                .collect(),
        )
    } else {
        None
    };

    let mut nodes: Vec<&crate::impact::ImpactNode> = report.reached.iter().collect();
    if let Some(scores) = &ppr_scores {
        nodes.sort_by(|a, b| {
            let (sa, sb) = (
                scores.get(&a.iri).copied().unwrap_or(0.0),
                scores.get(&b.iri).copied().unwrap_or(0.0),
            );
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.iri.cmp(&b.iri))
        });
    }
    let reached: Vec<JsonValue> = nodes
        .iter()
        .map(|n| {
            let mut entry = serde_json::json!({
                "iri": n.iri,
                "depth": n.depth,
                "via_predicate": n.via_predicate,
                "via_subject": n.via_subject,
            });
            if let Some(scores) = &ppr_scores {
                entry["ppr"] = serde_json::json!(scores.get(&n.iri).copied().unwrap_or(0.0));
            }
            entry
        })
        .collect();

    Ok(serde_json::json!({
        "root": report.root,
        "reached": reached,
        "reached_count": report.reached.len().saturating_sub(1),
        "hops": report.hops,
        "edges": report.edges_traversed,
        "counterfactual": remove,
        "ranked_by_ppr": rank_by_ppr,
    }))
}
