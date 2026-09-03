//! Data-driven IRI compaction for human- and agent-facing result surfaces.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value as JsonValue};

use crate::error::Result;
use crate::store::Store;

/// Prefix bindings extracted from the shape sets loaded in this store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefixMap(BTreeMap<String, String>);

impl PrefixMap {
    /// Build the prefix table from persisted shape data, never a duplicated list.
    pub fn from_store(store: &Store) -> Result<Self> {
        static DECLARATION: OnceLock<Regex> = OnceLock::new();
        let declaration = DECLARATION.get_or_init(|| {
            Regex::new(r"(?im)(?:@prefix|prefix)\s+([A-Za-z][A-Za-z0-9_-]*):\s*<([^>]+)>")
                .expect("static prefix regex")
        });
        let mut prefixes = BTreeMap::new();
        for (_, turtle, _) in store.list_shapes()? {
            for capture in declaration.captures_iter(&turtle) {
                prefixes
                    .entry(capture[1].to_string())
                    .or_insert_with(|| capture[2].to_string());
            }
        }
        Ok(Self(prefixes))
    }

    /// Compact an IRI when a loaded prefix matches; unknown namespaces stay full.
    #[must_use]
    pub fn compact(&self, iri: &str) -> String {
        self.0
            .iter()
            .filter_map(|(prefix, namespace)| {
                iri.strip_prefix(namespace).and_then(|local| {
                    is_safe_local(local).then(|| (namespace.len(), format!("{prefix}:{local}")))
                })
            })
            .max_by_key(|(length, _)| *length)
            .map_or_else(|| iri.to_string(), |(_, compact)| compact)
    }

    /// JSON-LD context object containing the exact persisted prefix bindings.
    #[must_use]
    pub fn json_ld_context(&self) -> JsonValue {
        JsonValue::Object(
            self.0
                .iter()
                .map(|(prefix, iri)| (prefix.clone(), JsonValue::String(iri.clone())))
                .collect::<Map<_, _>>(),
        )
    }

    /// Expand a compact IRI, used to prove compaction round-trips losslessly.
    #[must_use]
    pub fn expand(&self, value: &str) -> String {
        value
            .split_once(':')
            .and_then(|(prefix, local)| self.0.get(prefix).map(|ns| format!("{ns}{local}")))
            .unwrap_or_else(|| value.to_string())
    }
}

fn is_safe_local(local: &str) -> bool {
    !local.is_empty()
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'~'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_prefix_compacts_and_round_trips_while_unknown_stays_full() {
        let store = Store::open_in_memory().unwrap();
        store
            .load_shapes(
                "test",
                "@prefix ex: <http://example.org/> .\n@prefix sh: <http://www.w3.org/ns/shacl#> .",
                "2026-09-03T00:00:00Z",
            )
            .unwrap();
        let prefixes = PrefixMap::from_store(&store).unwrap();
        let iri = "http://example.org/alice";
        assert_eq!(prefixes.compact(iri), "ex:alice");
        assert_eq!(prefixes.expand(&prefixes.compact(iri)), iri);
        assert_eq!(
            prefixes.compact("http://example.org/nested/alice"),
            "http://example.org/nested/alice"
        );
        assert_eq!(
            prefixes.compact("https://unknown.example/x"),
            "https://unknown.example/x"
        );
    }
}
