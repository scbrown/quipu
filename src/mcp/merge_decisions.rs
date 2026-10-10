//! MCP tools: `quipu_merge_decisions` / `quipu_merge_apply` (aegis-yavo9c).
//!
//! Two tools, not one with a mode, for the reason `align` gives: a client
//! judges a tool by its annotation, and the read that emits (and proposes)
//! decisions must not carry the writer's. The REST routes call these same
//! functions, so the surfaces cannot diverge.
//!
//! Shares arrive INLINE, in the shape `/import` takes, and are verified by
//! hash exactly as a share read from disk. A server-side path is never
//! accepted: that would let a caller name any directory on the server.

use serde_json::{Value as JsonValue, json};

use crate::error::{Error, Result};
use crate::share_merge_decisions::{
    DecisionFile, InlineShare, SharePair, apply_pair, dry_run_pair, emit_pair, propose,
};
use crate::store::Store;

fn share(input: &JsonValue, key: &str) -> Result<InlineShare> {
    let value = input
        .get(key)
        .ok_or_else(|| Error::InvalidValue(format!("missing '{key}' share")))?;
    serde_json::from_value(value.clone())
        .map_err(|e| Error::InvalidValue(format!("'{key}' is not an inline share: {e}")))
}

fn pair(input: &JsonValue) -> Result<SharePair> {
    SharePair::inline(share(input, "incoming")?, share(input, "base")?)
}

/// READ. The conflicts of merging `incoming` into ROOT, as a decisions file;
/// with `propose: true`, each row also carries a mechanical proposal.
///
/// # Errors
/// A missing or unverifiable share, a base that is not the incoming share's
/// parent, or a store read error.
pub fn tool_merge_decisions(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let mut file = emit_pair(store, pair(input)?)?;
    if input.get("propose").and_then(JsonValue::as_bool) == Some(true) {
        file = propose(file);
    }
    serde_json::to_value(&file).map_err(|e| Error::Serialization(e.to_string()))
}

/// WRITE. Finish the merge from a decided file, atomically.
///
/// # Errors
/// As [`crate::share_merge_decisions::apply`]: an undecided row, a stale
/// file, too many values, or an invalid term all refuse and write nothing.
pub fn tool_merge_apply(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let decisions = input
        .get("decisions")
        .ok_or_else(|| Error::InvalidValue("missing 'decisions'".into()))?;
    let file: DecisionFile = serde_json::from_value(decisions.clone())
        .map_err(|e| Error::InvalidValue(format!("'decisions' is not a decisions file: {e}")))?;
    let reviewer = input
        .get("reviewer")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'reviewer'".into()))?;
    let actor = input.get("actor").and_then(JsonValue::as_str);
    // The hash recorded in provenance is of the decisions exactly as received.
    let bytes = serde_json::to_vec(decisions).map_err(|e| Error::Serialization(e.to_string()))?;
    let result = if input.get("dry_run").and_then(JsonValue::as_bool) == Some(true) {
        dry_run_pair(store, pair(input)?, &file, &bytes, reviewer)?
    } else {
        apply_pair(
            store,
            pair(input)?,
            &file,
            &bytes,
            reviewer,
            &crate::time::now_iso(),
            actor,
        )?
    };
    Ok(json!(result))
}
