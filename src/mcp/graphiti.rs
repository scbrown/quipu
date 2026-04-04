//! Graphiti-compatible REST endpoint handlers.
//!
//! These wrappers present Quipu's knowledge graph through the Graphiti API
//! surface so existing formulas and scripts (e.g. `mol-ontology-ingest`,
//! crew `CLAUDE.md` instructions) work unchanged.

use serde_json::Value as JsonValue;

use crate::episode;
use crate::error::{Error, Result};
use crate::sparql;
use crate::store::Store;
use crate::types::Value;

/// Graphiti-compatible endpoint: search nodes by name/description.
///
/// Input: `{ "query": "...", "group_ids": ["..."], "max_results": N, "entity_type_filter": "..." }`
/// Output: `{ "nodes": [{ "uuid", "name", "group_id", "labels", "summary", "created_at" }] }`
///
/// Wraps the context pipeline's text search, returning results in the flat node
/// format expected by Graphiti consumers (e.g. `mol-ontology-ingest`).
pub fn tool_search_nodes(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let query = input
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
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
    });

    // Get candidate entities — optionally constrained by rdf:type.
    let sparql = if let Some(type_iri) = entity_type_filter {
        let safe_type = type_iri.replace('>', "\\>");
        format!("SELECT DISTINCT ?s WHERE {{ ?s a <{safe_type}> }}")
    } else {
        "SELECT DISTINCT ?s WHERE { ?s ?p ?o }".to_string()
    };

    let result = sparql::query(store, &sparql)?;

    let query_lower = query.to_lowercase();
    let mut nodes = Vec::new();
    for row in result.rows() {
        if nodes.len() >= max_results {
            break;
        }
        if let Some(Value::Ref(id)) = row.get("s") {
            let iri = store.resolve(*id)?;
            let facts = store.entity_facts(*id)?;

            // Extract label, types, description, group_id from facts.
            let mut name = None;
            let mut summary = None;
            let mut labels = Vec::new();
            let mut group_id = None;
            let mut created_at = None;

            for f in &facts {
                let pred = store.resolve(f.attribute).unwrap_or_default();
                if pred.ends_with("#label") || pred.ends_with("/label") {
                    if let Value::Str(s) = &f.value {
                        name = Some(s.clone());
                    }
                } else if pred.ends_with("#comment") || pred.ends_with("/comment") {
                    if let Value::Str(s) = &f.value {
                        summary = Some(s.clone());
                    }
                } else if pred.ends_with("#type") || pred.ends_with("/type") {
                    if let Value::Ref(tid) = &f.value
                        && let Ok(type_iri) = store.resolve(*tid)
                    {
                        labels.push(type_iri);
                    }
                } else if pred.ends_with("groupId")
                    && let Value::Str(s) = &f.value
                {
                    group_id = Some(s.clone());
                }
                if created_at.is_none() {
                    created_at = Some(f.valid_from.clone());
                }
            }

            // Text match: check IRI, label, and description against query.
            let iri_matches = iri.to_lowercase().contains(&query_lower);
            let label_matches = name
                .as_ref()
                .is_some_and(|n| n.to_lowercase().contains(&query_lower));
            let desc_matches = summary
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&query_lower));

            if !iri_matches && !label_matches && !desc_matches {
                continue;
            }

            // Filter by group_ids if specified.
            if let Some(ref gids) = group_ids {
                match &group_id {
                    Some(gid) if gids.contains(&gid.as_str()) => {}
                    _ => continue,
                }
            }

            nodes.push(serde_json::json!({
                "uuid": iri,
                "name": name.unwrap_or_else(|| iri.clone()),
                "group_id": group_id,
                "labels": labels,
                "summary": summary,
                "created_at": created_at,
            }));
        }
    }

    Ok(serde_json::json!({
        "nodes": nodes,
        "count": nodes.len()
    }))
}

/// Graphiti-compatible endpoint: flat episode ingestion.
///
/// Input: `{ "name": "...", "episode_body": "...", "group_id": "...", "source_description": "..." }`
/// Output: `{ "tx_id": N, "count": N, "episode": "..." }`
///
/// Converts the flat Graphiti episode format into Quipu's Episode struct and
/// ingests via the standard episode pipeline. The `episode_body` is stored as
/// provenance metadata; no nodes or edges are extracted (the caller provides raw
/// prose, not structured triples).
pub fn tool_episodes_complete(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;

    let episode_body = input.get("episode_body").and_then(|v| v.as_str());
    let group_id = input.get("group_id").and_then(|v| v.as_str());
    let source_description = input.get("source_description").and_then(|v| v.as_str());

    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    let ep = episode::Episode {
        name: name.to_string(),
        episode_body: episode_body.map(std::string::ToString::to_string),
        source: source_description.map(std::string::ToString::to_string),
        group_id: group_id.map(std::string::ToString::to_string),
        nodes: Vec::new(),
        edges: Vec::new(),
        shapes: None,
    };

    let (tx_id, count) =
        episode::ingest_episode(store, &ep, timestamp, crate::namespace::DEFAULT_BASE_NS)?;

    Ok(serde_json::json!({
        "tx_id": tx_id,
        "count": count,
        "episode": name
    }))
}
