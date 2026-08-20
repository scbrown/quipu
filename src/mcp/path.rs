//! MCP tools for golden-path analysis: the provenance cone and the backtest.
//!
//! Both are reads. The vocabulary namespace comes from the store's
//! configured `base_ns` unless the caller overrides it — a parameter, never
//! a hardcoded hostname.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::path::{ConeOptions, PathVocab, backtest, cone};
use crate::store::Store;

fn vocab_for(store: &Store, input: &JsonValue) -> PathVocab {
    let ns = input
        .get("base_ns")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| store.base_ns());
    PathVocab::new(ns)
}

fn string_list(input: &JsonValue, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// MCP tool: `quipu_path_cone` — which steps did the verified result depend on?
///
/// Input: `{ "trajectory": "<IRI>", "via": ["<derivation predicate IRI>", ...],
///           "hops": N, "base_ns": "<ns override>" }`
pub fn tool_path_cone(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let trajectory = input
        .get("trajectory")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'trajectory' IRI parameter".into()))?;
    let vocab = vocab_for(store, input);
    let opts = ConeOptions {
        via: string_list(input, "via"),
        hops: input
            .get("hops")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(crate::path::cone::DEFAULT_CONE_HOPS as u64) as usize,
    };
    let report = cone(store, trajectory, &vocab, &opts)?;
    serde_json::to_value(&report).map_err(|e| Error::InvalidValue(e.to_string()))
}

/// MCP tool: `quipu_path_backtest` — replay a pruned candidate over history.
///
/// Input: `{ "exemplar": "<trajectory IRI>", "omit": ["<step IRI>", ...],
///           "base_ns": "<ns override>" }`
pub fn tool_path_backtest(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let exemplar = input
        .get("exemplar")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'exemplar' trajectory IRI parameter".into()))?;
    let vocab = vocab_for(store, input);
    let omit = string_list(input, "omit");
    let report = backtest(store, exemplar, &omit, &vocab)?;
    serde_json::to_value(&report).map_err(|e| Error::InvalidValue(e.to_string()))
}
