//! MCP tool handler for entity resolution.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// MCP tool: `quipu_resolve_entity` -- Check for existing near-duplicate entities.
///
/// Runs entity resolution (vector similarity + canonical name matching) and
/// returns candidates that may be duplicates of the proposed entity.
///
/// Input: `{ "name": "Alice Smith", "properties": { "role": "engineer" }, "top_k": 3, "threshold": 0.85 }`
/// Output: `{ "has_matches": bool, "candidates": [...] }`
pub fn tool_resolve_entity(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;

    let properties: Vec<(String, String)> = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let top_k = input
        .get("top_k")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(3) as usize;

    let threshold = input
        .get("threshold")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.85);

    let result = crate::resolution::resolve_entity(store, name, &properties, threshold, top_k)?;

    let candidates: Vec<JsonValue> = result
        .candidates
        .iter()
        .map(|c| {
            serde_json::json!({
                "iri": c.iri,
                "score": c.score,
                "matched_on": c.matched_on
            })
        })
        .collect();

    Ok(serde_json::json!({
        "has_matches": result.has_matches,
        "candidates": candidates,
        "count": candidates.len()
    }))
}
