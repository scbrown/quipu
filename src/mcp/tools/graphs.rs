//! MCP tool: `quipu_graph_list` — the graph registry, listed.
//!
//! The read half of the graph-kinds surface (the writes are
//! `quipu_graph_create` / `quipu_graph_label` in `mcp::governance`). Also the
//! consumer capability probe: a store serving this tool has the kind axis; a
//! 404/unknown-tool answer means the store predates it, which a consumer must
//! treat as "cannot tell", never as "no graphs".

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// List registered graphs, optionally filtered.
///
/// Input: `{ "kind": "<token>", "lifecycle": "frozen" }` — both optional.
/// Output: `{ "graphs": [{iri, g, class, source, lifecycle, labels: {…}}, …],
/// "count": N }`.
pub fn tool_graph_list(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let kind = match input.get("kind").and_then(JsonValue::as_str) {
        // Strict parse, matching quipu_graph_label: an unrecognised shape is
        // an error, never a filter that silently matches nothing.
        Some(k) => Some(crate::lattice_kind::DataKind::parse(k)?),
        None => None,
    };
    let lifecycle = input.get("lifecycle").and_then(JsonValue::as_str);
    if let Some(lc) = lifecycle
        && lc != "frozen"
    {
        return Err(Error::InvalidValue(format!(
            "unknown lifecycle filter '{lc}'; the only lifecycle state is 'frozen'"
        )));
    }

    let graphs = store.list_graphs(
        kind.as_ref().map(crate::lattice_kind::DataKind::as_str),
        lifecycle,
    )?;
    let rows: Vec<JsonValue> = graphs
        .iter()
        .map(|gi| {
            serde_json::json!({
                "iri": gi.iri,
                "g": gi.g,
                "class": gi.class,
                "source": gi.source,
                "lifecycle": gi.lifecycle,
                "labels": {
                    "freshness": gi.freshness,
                    "durability": gi.durability,
                    "trust_rank": gi.trust_rank,
                    "trust_chain": gi.trust_chain,
                    "policy": gi.policy,
                    "kind": gi.kind,
                },
            })
        })
        .collect();
    Ok(serde_json::json!({ "count": rows.len(), "graphs": rows }))
}
