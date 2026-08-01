//! Read-only graph tools: `quipu_cord` (pattern listing) and
//! `quipu_unravel` (time-travel query).

use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::sparql;
use crate::store::{AsOf, Store};
use crate::types::Value;

use crate::mcp::value_to_json;

/// MCP tool: `quipu_cord` -- List entities matching a pattern.
///
/// Input: `{ "type": "<optional IRI>", "predicate": "<optional IRI>", "limit": N }`
/// Output: `{ "entities": [{ "iri": "...", "facts": [...] }, ...] }`
pub fn tool_cord(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(100) as usize;

    let type_filter = input.get("type").and_then(|v| v.as_str());
    let pred_filter = input.get("predicate").and_then(|v| v.as_str());

    let query = if let Some(type_iri) = type_filter {
        format!("SELECT DISTINCT ?s WHERE {{ ?s a <{type_iri}> }} LIMIT {limit}")
    } else if let Some(pred_iri) = pred_filter {
        format!("SELECT DISTINCT ?s WHERE {{ ?s <{pred_iri}> ?o }} LIMIT {limit}")
    } else {
        format!("SELECT DISTINCT ?s WHERE {{ ?s ?p ?o }} LIMIT {limit}")
    };

    let result = sparql::query(store, &query)?;

    let mut entities = Vec::new();
    for row in result.rows() {
        if let Some(Value::Ref(id)) = row.get("s") {
            let iri = store.resolve(*id)?;
            let facts = store.entity_facts(*id)?;
            let fact_list: Vec<JsonValue> = facts
                .iter()
                .map(|f| {
                    let pred = store.resolve(f.attribute).unwrap_or_default();
                    serde_json::json!({
                        "predicate": pred,
                        "value": value_to_json(store, &f.value)
                    })
                })
                .collect();
            entities.push(serde_json::json!({
                "iri": iri,
                "facts": fact_list
            }));
        }
    }

    Ok(serde_json::json!({
        "entities": entities,
        "count": entities.len()
    }))
}

/// MCP tool: `quipu_unravel` -- Time-travel query.
///
/// Input: `{ "tx": N, "valid_at": "..." }`
/// Output: `{ "facts": [...], "count": N }`
pub fn tool_unravel(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let as_of = AsOf {
        tx: input.get("tx").and_then(serde_json::Value::as_i64),
        valid_at: input
            .get("valid_at")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
    };

    let facts = store.facts_as_of(&as_of)?;

    let fact_list: Vec<JsonValue> = facts
        .iter()
        .map(|f| {
            let entity = store.resolve(f.entity).unwrap_or_default();
            let pred = store.resolve(f.attribute).unwrap_or_default();
            serde_json::json!({
                "entity": entity,
                "predicate": pred,
                "value": value_to_json(store, &f.value),
                "valid_from": f.valid_from,
                "valid_to": f.valid_to,
                "tx": f.tx
            })
        })
        .collect();

    Ok(serde_json::json!({
        "facts": fact_list,
        "count": fact_list.len()
    }))
}
