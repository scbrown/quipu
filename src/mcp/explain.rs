//! MCP tool for the derivation-chain walk (quipu-923, gap G8).

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// MCP tool: `quipu_explain` — walk a fact's derivation chain.
///
/// Input: `s`, `p`, `o` (IRIs; a non-IRI `o` is treated as a string
/// literal), optional `depth` (default 5). Read-only. Same walk as
/// CLI `quipu explain` and REST `POST /explain`.
pub fn tool_explain(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let field = |k: &str| -> Result<&str> {
        input
            .get(k)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| Error::InvalidValue(format!("missing required field '{k}' (an IRI)")))
    };
    let depth = input
        .get("depth")
        .and_then(JsonValue::as_u64)
        .map_or(crate::explain::DEFAULT_EXPLAIN_DEPTH, |d| d as usize);
    crate::explain::explain(store, field("s")?, field("p")?, field("o")?, depth)
}
