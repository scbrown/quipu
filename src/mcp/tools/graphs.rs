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

/// MCP tool: `quipu_graph_freeze` — relocate a graph's full history into a
/// read-only archive pack (deep freeze).
///
/// Input: `{ "graph": "<iri>", "out_dir": "<dir>", "timestamp": "...",
/// "actor": "..." }` (`out_dir` optional — defaults beside the store file).
pub fn tool_graph_freeze(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let graph = input
        .get("graph")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'graph' parameter".into()))?;
    let timestamp = input
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'timestamp' parameter".into()))?;
    let out_dir = match input.get("out_dir").and_then(JsonValue::as_str) {
        Some(d) => d.to_string(),
        None => store.db_parent_dir().ok_or_else(|| {
            Error::InvalidValue(
                "this store has no file path (in-memory?); pass 'out_dir' explicitly".into(),
            )
        })?,
    };
    let actor = input.get("actor").and_then(JsonValue::as_str);
    let r = store.freeze_graph(graph, &out_dir, timestamp, actor)?;
    Ok(serde_json::json!({
        "graph": r.graph_iri,
        "pack": r.path,
        "alias": r.alias,
        "content_hash": r.content_hash,
        "facts": r.facts,
        "transactions": r.transactions,
        "vectors": r.vectors,
        "vectors_omitted": r.vectors_omitted,
    }))
}

/// MCP tool: `quipu_graph_thaw` — restore a frozen graph's history from its
/// archive pack and reopen it for writes.
pub fn tool_graph_thaw(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let graph = input
        .get("graph")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'graph' parameter".into()))?;
    let timestamp = input
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'timestamp' parameter".into()))?;
    let actor = input.get("actor").and_then(JsonValue::as_str);
    let (facts, vectors) = store.thaw_graph(graph, timestamp, actor)?;
    Ok(serde_json::json!({
        "graph": graph,
        "facts_restored": facts,
        "vectors_restored": vectors,
    }))
}
