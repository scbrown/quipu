//! Stored named-query registry management: `quipu_queries` (quipu #79).

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::store::queries::{StoredParam, StoredQuery};

fn parse_query(input: &JsonValue) -> Result<StoredQuery> {
    let get = |k: &str| input.get(k).and_then(|v| v.as_str());
    let name = get("name").ok_or_else(|| Error::InvalidValue("missing 'name'".into()))?;
    let template =
        get("template").ok_or_else(|| Error::InvalidValue("missing 'template'".into()))?;
    let description = get("description").unwrap_or("");

    let mut params = Vec::new();
    if let Some(arr) = input.get("params").and_then(|v| v.as_array()) {
        for raw in arr {
            let obj = raw
                .as_object()
                .ok_or_else(|| Error::InvalidValue("each param must be an object".into()))?;
            let pname = obj
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("param needs 'name'".into()))?;
            params.push(StoredParam {
                name: pname.to_string(),
                kind: obj
                    .get("type")
                    .or_else(|| obj.get("kind"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("text")
                    .to_string(),
                required: obj
                    .get("required")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                default: obj
                    .get("default")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                description: obj
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }

    Ok(StoredQuery {
        name: name.to_string(),
        description: description.to_string(),
        template: template.to_string(),
        dataset: get("dataset").map(String::from),
        params,
    })
}

/// MCP/HTTP tool: `quipu_queries` — manage stored named queries.
///
/// Actions: `load` | `list` | `get` | `remove`. An unknown action ERRORS rather
/// than falling through to `list` — the recorded `tool_shapes` lesson: a typo'd
/// action that quietly lists is indistinguishable from one that did what you
/// asked.
///
/// # Errors
/// Unknown action, missing parameters, or a definition that fails load-time
/// validation (see [`StoredQuery::validate`]).
pub fn tool_queries(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map_or_else(crate::time::now_iso, String::from);

    match action {
        "load" => {
            let q = parse_query(input)?;
            store.query_load(&q, &timestamp)?;
            Ok(serde_json::json!({ "loaded": q.name, "params": q.params.len() }))
        }
        "list" => Ok(serde_json::json!({
            "queries": store
                .query_list()?
                .iter()
                .map(StoredQuery::to_catalog_json)
                .collect::<Vec<_>>(),
        })),
        "get" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name'".into()))?;
            let q = store
                .query_get(name)?
                .ok_or_else(|| Error::InvalidValue(format!("no such stored query: {name}")))?;
            Ok(serde_json::json!({
                "query": q.to_catalog_json(),
                "template": q.template,
            }))
        }
        "remove" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name'".into()))?;
            Ok(serde_json::json!({ "removed": store.query_remove(name, &timestamp)? }))
        }
        other => Err(Error::InvalidValue(format!(
            "unknown queries action '{other}' (load|list|get|remove)"
        ))),
    }
}
