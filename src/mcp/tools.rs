//! Individual MCP tool implementations (cord, unravel, search, validate, episode, etc.)

use serde_json::Value as JsonValue;

use crate::episode::{self, Episode};
use crate::error::{Error, Result};
use crate::sparql;
use crate::store::{AsOf, Store};
use crate::types::Value;

use super::value_to_json;

/// MCP tool: `quipu_cord` -- List entities matching a pattern.
///
/// Input: `{ "type": "<optional IRI>", "predicate": "<optional IRI>", "limit": N }`
/// Output: `{ "entities": [{ "iri": "...", "facts": [...] }, ...] }`
pub fn tool_cord(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    let type_filter = input.get("type").and_then(|v| v.as_str());
    let pred_filter = input.get("predicate").and_then(|v| v.as_str());

    let query = if let Some(type_iri) = type_filter {
        format!(
            "SELECT DISTINCT ?s WHERE {{ ?s a <{type_iri}> }} LIMIT {limit}"
        )
    } else if let Some(pred_iri) = pred_filter {
        format!(
            "SELECT DISTINCT ?s WHERE {{ ?s <{pred_iri}> ?o }} LIMIT {limit}"
        )
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
        tx: input.get("tx").and_then(|v| v.as_i64()),
        valid_at: input
            .get("valid_at")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
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

/// MCP tool: `quipu_validate` -- Validate data against shapes.
///
/// Input: `{ "shapes": "<shapes turtle>", "data": "<data turtle>" }`
/// Output: validation feedback JSON
pub fn tool_validate(input: &JsonValue) -> Result<JsonValue> {
    let shapes = input
        .get("shapes")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'shapes' parameter".into()))?;
    let data = input
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'data' parameter".into()))?;

    let feedback = crate::shacl::validate_shapes(shapes, data)?;

    let issues: Vec<JsonValue> = feedback
        .results
        .iter()
        .map(|r| {
            serde_json::json!({
                "severity": r.severity,
                "focus_node": r.focus_node,
                "component": r.component,
                "path": r.path,
                "value": r.value,
                "source_shape": r.source_shape,
                "message": r.message
            })
        })
        .collect();

    Ok(serde_json::json!({
        "conforms": feedback.conforms,
        "violations": feedback.violations,
        "warnings": feedback.warnings,
        "issues": issues
    }))
}

/// MCP tool: `quipu_search` -- Semantic vector search over entity embeddings.
pub fn tool_search(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let embedding: Vec<f32> = input
        .get("embedding")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::InvalidValue("missing 'embedding' array parameter".into()))?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();

    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    let valid_at = input.get("valid_at").and_then(|v| v.as_str());

    let matches = store.vector_search(&embedding, limit, valid_at)?;

    let results: Vec<JsonValue> = matches
        .iter()
        .map(|m| {
            let iri = store.resolve(m.entity_id).unwrap_or_else(|_| format!("ref:{}", m.entity_id));
            serde_json::json!({
                "entity": iri,
                "text": m.text,
                "score": m.score,
                "valid_from": m.valid_from,
                "valid_to": m.valid_to
            })
        })
        .collect();

    Ok(serde_json::json!({
        "results": results,
        "count": results.len()
    }))
}

/// MCP tool: `quipu_shapes` -- Manage persistent SHACL shapes.
pub fn tool_shapes(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    match action {
        "load" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' for shape".into()))?;
            let turtle = input
                .get("turtle")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'turtle' for shape".into()))?;
            let timestamp = input
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("1970-01-01T00:00:00Z");

            crate::shacl::validate_shapes(turtle, "@prefix ex: <http://example.org/> .\n")?;

            store.load_shapes(name, turtle, timestamp)?;
            Ok(serde_json::json!({
                "action": "loaded",
                "name": name
            }))
        }
        "remove" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' for removal".into()))?;
            let removed = store.remove_shapes(name)?;
            Ok(serde_json::json!({
                "action": "removed",
                "name": name,
                "found": removed
            }))
        }
        _ => {
            let shapes = store.list_shapes()?;
            let items: Vec<JsonValue> = shapes
                .iter()
                .map(|(name, _, loaded_at)| {
                    serde_json::json!({
                        "name": name,
                        "loaded_at": loaded_at
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "shapes": items,
                "count": items.len()
            }))
        }
    }
}

/// MCP tool: `quipu_retract` -- Retract facts for an entity.
pub fn tool_retract(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let entity_iri = input
        .get("entity")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'entity' IRI parameter".into()))?;

    let entity_id = store
        .lookup(entity_iri)?
        .ok_or_else(|| Error::InvalidValue(format!("entity not found: {entity_iri}")))?;

    let predicate_id = if let Some(pred_iri) = input.get("predicate").and_then(|v| v.as_str()) {
        Some(
            store
                .lookup(pred_iri)?
                .ok_or_else(|| Error::InvalidValue(format!("predicate not found: {pred_iri}")))?,
        )
    } else {
        None
    };

    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    let actor = input.get("actor").and_then(|v| v.as_str());

    let (tx_id, count) = store.retract_entity(entity_id, predicate_id, timestamp, actor)?;

    Ok(serde_json::json!({
        "tx_id": tx_id,
        "retracted": count,
        "entity": entity_iri
    }))
}

/// MCP tool: `quipu_episode` -- Ingest structured knowledge from an agent episode.
pub fn tool_episode(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let ep: Episode = serde_json::from_value(input.clone())
        .map_err(|e| Error::InvalidValue(format!("invalid episode JSON: {e}")))?;

    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    let (tx_id, count) = episode::ingest_episode(store, &ep, timestamp)?;

    Ok(serde_json::json!({
        "tx_id": tx_id,
        "count": count,
        "episode": ep.name
    }))
}
