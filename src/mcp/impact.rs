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
        let timestamp = input
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("1970-01-01T00:00:00Z");
        crate::impact::speculate_remove(store, entity_iri, timestamp, |s| {
            crate::impact::impact(s, entity_iri, &opts)
        })?
    } else {
        crate::impact::impact(store, entity_iri, &opts)?
    };

    let reached: Vec<JsonValue> = report
        .reached
        .iter()
        .map(|n| {
            serde_json::json!({
                "iri": n.iri,
                "depth": n.depth,
                "via_predicate": n.via_predicate,
                "via_subject": n.via_subject,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "root": report.root,
        "reached": reached,
        "reached_count": report.reached.len().saturating_sub(1),
        "hops": report.hops,
        "edges": report.edges_traversed,
        "counterfactual": remove,
    }))
}
