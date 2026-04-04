//! MCP search tools: `quipu_search_nodes` and `quipu_search_facts`.
//!
//! These replace Graphiti's `search_nodes` and `search_memory_facts` MCP tools,
//! providing semantic search over the knowledge graph via case-insensitive
//! substring matching on entity IRIs, labels, and literal values.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::sparql;
use crate::store::Store;
use crate::types::Value;

/// MCP tool: `quipu_search_nodes` — Search entities by natural language query.
///
/// Fetches candidate entities via SPARQL (optionally filtered by type and group IDs),
/// then performs case-insensitive substring matching in Rust on entity IRIs and
/// literal values. The SPARQL engine doesn't support CONTAINS/LCASE/STR functions,
/// so text matching is done post-query.
///
/// Input: `{ "query": "...", "group_ids": ["..."], "max_results": N, "entity_type_filter": "..." }`
/// Output: `{ "nodes": [{ "iri", "label", "types", "summary", "score" }], "count": N }`
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

    let group_ids: Option<Vec<&str>> = input
        .get("group_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    // Build SPARQL to fetch candidate entities with optional type/group filters.
    // Oversample to allow for post-filter text matching reducing the set.
    let oversample = max_results * 10;
    let mut patterns = String::new();
    if let Some(type_iri) = entity_type_filter {
        let safe_type = type_iri.replace('>', "\\>");
        patterns.push_str(&format!("?s a <{safe_type}> . "));
    }
    if let Some(ref gids) = group_ids {
        patterns.push_str(
            "?s <http://www.w3.org/ns/prov#wasGeneratedBy> ?_episode . \
             ?_episode <http://aegis.gastown.local/ontology/groupId> ?_gid . ",
        );
        let gid_filters: Vec<String> = gids
            .iter()
            .map(|g| {
                let safe = g.replace('\\', "\\\\").replace('\'', "\\'");
                format!("?_gid = '{safe}'")
            })
            .collect();
        patterns.push_str(&format!("FILTER({}) ", gid_filters.join(" || ")));
    }

    let sparql = format!("SELECT DISTINCT ?s WHERE {{ ?s ?p ?o . {patterns}}} LIMIT {oversample}");

    let result = sparql::query(store, &sparql)?;

    let query_lower = query.to_lowercase();
    let mut nodes = Vec::new();

    for row in result.rows() {
        if nodes.len() >= max_results {
            break;
        }
        if let Some(Value::Ref(id)) = row.get("s") {
            let iri = store.resolve(*id)?;
            if entity_matches_query(store, *id, &iri, &query_lower)? {
                let entity = build_node_result(store, &iri, max_results)?;
                nodes.push(entity);
            }
        }
    }

    Ok(serde_json::json!({
        "nodes": nodes,
        "count": nodes.len()
    }))
}

/// MCP tool: `quipu_search_facts` — Search relationships/edges by natural language.
///
/// Fetches candidate triples via SPARQL (optionally filtered by group IDs),
/// then performs case-insensitive substring matching on predicate IRIs and
/// object values in Rust.
///
/// Input: `{ "query": "...", "group_ids": ["..."], "max_results": N }`
/// Output: `{ "facts": [{ "source", "predicate", "target", "provenance" }], "count": N }`
pub fn tool_search_facts(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'query' parameter".into()))?;

    let max_results = input
        .get("max_results")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10) as usize;

    let group_ids: Option<Vec<&str>> = input
        .get("group_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    let oversample = max_results * 10;
    let mut patterns = String::new();
    if let Some(ref gids) = group_ids {
        patterns.push_str(
            "?s <http://www.w3.org/ns/prov#wasGeneratedBy> ?_episode . \
             ?_episode <http://aegis.gastown.local/ontology/groupId> ?_gid . ",
        );
        let gid_filters: Vec<String> = gids
            .iter()
            .map(|g| {
                let safe = g.replace('\\', "\\\\").replace('\'', "\\'");
                format!("?_gid = '{safe}'")
            })
            .collect();
        patterns.push_str(&format!("FILTER({}) ", gid_filters.join(" || ")));
    }

    let sparql = format!("SELECT ?s ?p ?o WHERE {{ ?s ?p ?o . {patterns}}} LIMIT {oversample}");

    let result = sparql::query(store, &sparql)?;
    let query_lower = query.to_lowercase();

    let mut facts = Vec::new();
    for row in result.rows() {
        if facts.len() >= max_results {
            break;
        }
        let source = match row.get("s") {
            Some(Value::Ref(id)) => store.resolve(*id)?,
            _ => continue,
        };
        let predicate = match row.get("p") {
            Some(Value::Ref(id)) => store.resolve(*id)?,
            _ => continue,
        };
        let target = match row.get("o") {
            Some(Value::Ref(id)) => store.resolve(*id)?,
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Int(n)) => n.to_string(),
            Some(Value::Float(f)) => f.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => continue,
        };

        // Post-filter: check if predicate or target contains the query.
        if !predicate.to_lowercase().contains(&query_lower)
            && !target.to_lowercase().contains(&query_lower)
        {
            continue;
        }

        let provenance = resolve_provenance(store, &source);

        facts.push(serde_json::json!({
            "source": source,
            "predicate": predicate,
            "target": target,
            "provenance": provenance
        }));
    }

    Ok(serde_json::json!({
        "facts": facts,
        "count": facts.len()
    }))
}

/// Check if an entity's IRI or any of its literal values match the query (case-insensitive).
fn entity_matches_query(
    store: &Store,
    entity_id: i64,
    iri: &str,
    query_lower: &str,
) -> Result<bool> {
    if iri.to_lowercase().contains(query_lower) {
        return Ok(true);
    }
    let facts = store.entity_facts(entity_id)?;
    for fact in &facts {
        let val_str = match &fact.value {
            Value::Str(s) => s.to_lowercase(),
            Value::Int(n) => n.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Ref(id) => store.resolve(*id).unwrap_or_default().to_lowercase(),
            Value::Bytes(_) => continue,
        };
        if val_str.contains(query_lower) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Build a node result object for an entity IRI, including label, types, and summary.
fn build_node_result(store: &Store, iri: &str, max_facts: usize) -> Result<JsonValue> {
    let safe_iri = iri
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('>', "\\>");

    let sparql = format!("SELECT ?p ?o WHERE {{ <{safe_iri}> ?p ?o }} LIMIT {max_facts}");
    let result = sparql::query(store, &sparql)?;

    let mut label: Option<String> = None;
    let mut types = Vec::new();
    let mut summary: Option<String> = None;

    for row in result.rows() {
        let pred_str = match row.get("p") {
            Some(Value::Ref(id)) => store.resolve(*id)?,
            _ => continue,
        };
        let val_str = match row.get("o") {
            Some(Value::Ref(id)) => store.resolve(*id)?,
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Int(n)) => n.to_string(),
            Some(Value::Float(f)) => f.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => continue,
        };

        if pred_str.ends_with("#label") || pred_str.ends_with("/label") {
            label = Some(val_str.clone());
        }
        if pred_str.ends_with("#type") || pred_str.ends_with("/type") {
            types.push(val_str.clone());
        }
        if pred_str.ends_with("#comment") || pred_str.ends_with("/comment") {
            summary = Some(val_str.clone());
        }
    }

    Ok(serde_json::json!({
        "iri": iri,
        "label": label,
        "types": types,
        "summary": summary,
        "score": 1.0
    }))
}

/// Resolve provenance for an entity — find the episode it was generated by.
fn resolve_provenance(store: &Store, entity_iri: &str) -> Option<JsonValue> {
    let safe_iri = entity_iri
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('>', "\\>");

    let sparql_simple = format!(
        "SELECT ?ep WHERE {{ \
            <{safe_iri}> <http://www.w3.org/ns/prov#wasGeneratedBy> ?ep \
        }} LIMIT 1"
    );

    if let Ok(result) = sparql::query(store, &sparql_simple) {
        for row in result.rows() {
            if let Some(Value::Ref(id)) = row.get("ep")
                && let Ok(ep_iri) = store.resolve(*id)
            {
                return Some(serde_json::json!({ "episode": ep_iri }));
            }
        }
    }

    None
}
