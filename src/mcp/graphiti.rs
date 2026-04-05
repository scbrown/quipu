//! Graphiti-compatible REST endpoint handlers.
//!
//! These endpoints match the Graphiti API surface so that existing formulas
//! and scripts (e.g. `mol-ontology-ingest`) can call Quipu without changes.

use serde_json::Value as JsonValue;

use crate::episode::{self, Episode};
use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;
use crate::vector::KnowledgeVectorStore;

/// Graphiti-compatible endpoint: `search_nodes` — semantic entity search by name/description.
///
/// Input: `{ "query": "text", "group_ids": ["g1"], "max_results": 10, "entity_type_filter": "Type" }`
/// Output: `{ "nodes": [{ "name": "...", "type": "...", ... }], "count": N }`
///
/// Uses vector search when an `EmbeddingProvider` is configured, falling back to
/// SPARQL FILTER on `rdfs:label` / `rdfs:comment`.
pub fn tool_search_nodes(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let query_text = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'query' parameter".into()))?;

    let max_results = input
        .get("max_results")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10) as usize;

    let entity_type_filter = input.get("entity_type_filter").and_then(|v| v.as_str());
    let group_ids: Option<Vec<&str>> = input.get("group_ids").and_then(|v| {
        v.as_array()
            .map(|arr| arr.iter().filter_map(|g| g.as_str()).collect())
    });

    // Try vector search first (semantic), fall back to SPARQL text filter.
    let vector_results = store
        .embed_query(query_text)
        .ok()
        .flatten()
        .and_then(|emb| {
            // Oversample to leave room for post-filtering.
            store.vector_search(&emb, max_results * 3, None).ok()
        });

    let mut seen_entities = std::collections::HashSet::new();
    let mut nodes = Vec::new();

    // Collect from vector results.
    if let Some(matches) = &vector_results {
        for m in matches {
            if seen_entities.len() >= max_results {
                break;
            }
            let Ok(iri) = store.resolve(m.entity_id) else {
                continue;
            };
            let Ok(facts) = store.entity_facts(m.entity_id) else {
                continue;
            };
            let node = facts_to_graphiti_node(store, &iri, &facts);

            if !passes_filters(&node, entity_type_filter, group_ids.as_deref()) {
                continue;
            }

            if seen_entities.insert(iri.clone()) {
                nodes.push(node);
            }
        }
    }

    // If we don't have enough results, supplement with SPARQL label search.
    // CONTAINS/LCASE aren't supported in Quipu's SPARQL, so we fetch all
    // labelled entities and filter in Rust.
    if nodes.len() < max_results {
        let oversample = max_results * 5;

        let sparql = format!(
            "SELECT DISTINCT ?s ?label WHERE {{ \
             ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . \
             }} LIMIT {oversample}",
        );

        let query_lower = query_text.to_lowercase();

        if let Ok(result) = crate::sparql::query(store, &sparql) {
            for row in result.rows() {
                if nodes.len() >= max_results {
                    break;
                }

                // Text match on label in Rust.
                let label_matches = match row.get("label") {
                    Some(Value::Str(s)) => s.to_lowercase().contains(&query_lower),
                    _ => false,
                };
                if !label_matches {
                    continue;
                }

                if let Some(Value::Ref(id)) = row.get("s") {
                    let Ok(iri) = store.resolve(*id) else {
                        continue;
                    };
                    if seen_entities.contains(&iri) {
                        continue;
                    }
                    let Ok(facts) = store.entity_facts(*id) else {
                        continue;
                    };
                    let node = facts_to_graphiti_node(store, &iri, &facts);

                    if !passes_filters(&node, entity_type_filter, group_ids.as_deref()) {
                        continue;
                    }

                    if seen_entities.insert(iri) {
                        nodes.push(node);
                    }
                }
            }
        }
    }

    Ok(serde_json::json!({
        "nodes": nodes,
        "count": nodes.len()
    }))
}

/// Graphiti-compatible endpoint: `episodes_complete` — flat episode ingestion.
///
/// Input: `{ "name": "...", "episode_body": "...", "group_id": "...", "source_description": "..." }`
/// Output: `{ "tx_id": N, "count": N, "episode": "name" }`
///
/// Converts Graphiti's flat episode format to Quipu's `Episode` struct and
/// calls the internal episode ingestion pipeline.
pub fn tool_episodes_complete(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;

    let episode = Episode {
        name: name.to_string(),
        episode_body: input
            .get("episode_body")
            .and_then(|v| v.as_str())
            .map(String::from),
        source: input
            .get("source_description")
            .and_then(|v| v.as_str())
            .map(String::from),
        group_id: input
            .get("group_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        nodes: Vec::new(),
        edges: Vec::new(),
        shapes: None,
    };

    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    let (tx_id, count) = episode::ingest_episode(
        store,
        &episode,
        timestamp,
        crate::namespace::DEFAULT_BASE_NS,
    )?;

    Ok(serde_json::json!({
        "tx_id": tx_id,
        "count": count,
        "episode": name
    }))
}

// ── Helpers ──────────────────────────────────────────────────────

/// Check whether a node passes `entity_type` and `group_ids` filters.
fn passes_filters(
    node: &JsonValue,
    entity_type_filter: Option<&str>,
    group_ids: Option<&[&str]>,
) -> bool {
    if let Some(type_filter) = entity_type_filter {
        match node.get("type").and_then(|v| v.as_str()) {
            Some(t) if t.contains(type_filter) => {}
            _ => return false,
        }
    }
    if let Some(gids) = group_ids {
        match node.get("group_id").and_then(|v| v.as_str()) {
            Some(gid) if gids.contains(&gid) => {}
            _ => return false,
        }
    }
    true
}

/// Convert entity facts into a Graphiti-compatible node JSON object.
fn facts_to_graphiti_node(store: &Store, iri: &str, facts: &[crate::types::Fact]) -> JsonValue {
    let mut name = String::new();
    let mut node_type = String::new();
    let mut description = String::new();
    let mut group_id = String::new();
    let mut properties = serde_json::Map::new();

    for fact in facts {
        let pred = store.resolve(fact.attribute).unwrap_or_default();
        match pred.as_str() {
            "http://www.w3.org/2000/01/rdf-schema#label" => {
                if let Value::Str(s) = &fact.value {
                    name.clone_from(s);
                }
            }
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type" => {
                if let Value::Ref(id) = &fact.value {
                    node_type = store.resolve(*id).unwrap_or_default();
                }
            }
            "http://www.w3.org/2000/01/rdf-schema#comment" => {
                if let Value::Str(s) = &fact.value {
                    description.clone_from(s);
                }
            }
            p if p.ends_with("groupId") => {
                if let Value::Str(s) = &fact.value {
                    group_id.clone_from(s);
                }
            }
            _ => {
                let key = pred
                    .rsplit_once('/')
                    .or_else(|| pred.rsplit_once('#'))
                    .map_or(&*pred, |(_, k)| k);
                properties.insert(key.to_string(), super::value_to_json(store, &fact.value));
            }
        }
    }

    let mut node = serde_json::json!({
        "iri": iri,
        "name": name,
    });
    let obj = node.as_object_mut().unwrap();
    if !node_type.is_empty() {
        obj.insert("type".to_string(), JsonValue::String(node_type));
    }
    if !description.is_empty() {
        obj.insert("description".to_string(), JsonValue::String(description));
    }
    if !group_id.is_empty() {
        obj.insert("group_id".to_string(), JsonValue::String(group_id));
    }
    if !properties.is_empty() {
        obj.insert("properties".to_string(), JsonValue::Object(properties));
    }
    node
}
