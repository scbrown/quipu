//! Shape and subscription management: `quipu_validate`, `quipu_shapes`,
//! `quipu_subscriptions`.

use std::collections::BTreeSet;

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// Resolve the shape source for a `/validate` request (quipu #71).
///
/// Returns `None` when the request carries its own `shapes` — the existing
/// contract, unchanged. Otherwise falls back to the STORED shapes, optionally
/// as they stood at `valid_at` / `as_of_tx`, defaulting to now.
///
/// Split out from [`tool_validate`] so the caller can take the store lock ONLY
/// to fetch the turtle and drop it before validation, which is CPU-bound and
/// unbounded in the size of the payload. Validating under the lock would
/// serialize every other request behind an arbitrary caller's data.
///
/// # Errors
/// Store errors while reading the registry.
pub fn resolve_validation_shapes(
    store: &crate::store::Store,
    input: &JsonValue,
) -> Result<Option<String>> {
    if input.get("shapes").and_then(|v| v.as_str()).is_some() {
        return Ok(None);
    }
    let as_of = crate::store::AsOf {
        tx: input.get("as_of_tx").and_then(serde_json::Value::as_i64),
        valid_at: input
            .get("valid_at")
            .and_then(|v| v.as_str())
            .map(String::from),
    };
    if as_of.tx.is_none() && as_of.valid_at.is_none() {
        return store.get_combined_shapes();
    }
    store.get_combined_shapes_as_of(&as_of)
}

/// MCP tool: `quipu_validate` -- Validate data against shapes.
///
/// Input: `{ "shapes": "<shapes turtle>", "data": "<data turtle>" }`
/// Output: validation feedback JSON
///
/// ⚠️ The `#[cfg]` below must stay ADJACENT to this fn. It was separated from it
/// once (quipu #71 inserted `resolve_validation_shapes` between the two), which
/// silently moved the gate onto the new function and left this one ungated —
/// three compile errors under `--no-default-features`, and a red CI nobody was
/// subscribed to for six hours. Do not insert anything between here and the fn.
///
/// # Errors
/// Missing `shapes`/`data` parameters, or a SHACL validation error.
#[cfg(feature = "shacl")]
pub fn tool_validate(input: &JsonValue) -> Result<JsonValue> {
    let shapes = input
        .get("shapes")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::InvalidValue(
                "missing 'shapes' parameter, and the store has no shapes loaded for the \
             requested window — pass 'shapes', or load some and retry"
                    .into(),
            )
        })?;
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

#[cfg(not(feature = "shacl"))]
pub fn tool_validate(_input: &JsonValue) -> Result<JsonValue> {
    Err(Error::InvalidValue(
        "SHACL validation requires the 'shacl' feature".into(),
    ))
}

/// MCP tool: `quipu_shapes` -- Manage persistent SHACL shapes.
/// MCP/HTTP tool: `quipu_subscriptions` — event-push subscription registry
/// (event-log P2). Actions: create / list / delete. An unknown action errors
/// (the `tool_shapes` silent-fall-through lesson).
pub fn tool_subscriptions(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");
    match action {
        "create" => {
            let consumer = input
                .get("consumer_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'consumer_id'".into()))?;
            let url = input
                .get("webhook_url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'webhook_url'".into()))?;
            let types: Option<Vec<String>> =
                input.get("types").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                });
            let mode = input
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("realtime");
            let ask = input.get("sparql_ask").and_then(|v| v.as_str());
            let batch_size = input
                .get("batch_size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(50) as usize;
            let batch_window = input
                .get("batch_window_s")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(30);
            let now = crate::time::now_iso();
            let id = store.subscription_create(
                consumer,
                types.as_deref(),
                ask,
                mode,
                url,
                batch_size,
                batch_window,
                &now,
            )?;
            Ok(serde_json::json!({"action": "created", "id": id, "consumer_id": consumer}))
        }
        "list" => {
            let subs = store.subscription_list()?;
            Ok(serde_json::json!({
                "count": subs.len(),
                "subscriptions": subs.iter().map(|s| serde_json::json!({
                    "id": s.id, "consumer_id": s.consumer_id, "types": s.types,
                    "mode": s.mode, "webhook_url": s.webhook_url,
                    "batch_size": s.batch_size, "batch_window_s": s.batch_window_s,
                })).collect::<Vec<_>>()
            }))
        }
        "delete" => {
            let consumer = input
                .get("consumer_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'consumer_id'".into()))?;
            let found = store.subscription_delete(consumer)?;
            Ok(serde_json::json!({"action": "deleted", "consumer_id": consumer, "found": found}))
        }
        other => Err(Error::InvalidValue(format!(
            "unknown subscriptions action '{other}' (expected: create, list, delete)"
        ))),
    }
}

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
            let now = crate::time::now_iso();
            let timestamp = input
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or(&now);

            // Validate the shapes parse correctly (requires shacl feature).
            #[cfg(feature = "shacl")]
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
        // Returns the stored turtle, so a caller can verify WHICH shapes are
        // loaded rather than only their names. Without this, `list` proves a
        // shape set exists under a name and nothing about its content: a load
        // of a stale or wrong .ttl is indistinguishable from the right one, and
        // every deploy-time assertion we have could only ever check names
        // (aegis-1y3q). The turtle was already in the tuple and discarded.
        "get" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' for get".into()))?;
            let shapes = store.list_shapes()?;
            let found = shapes.iter().find(|(n, _, _)| n == name);
            match found {
                Some((n, turtle, loaded_at)) => Ok(serde_json::json!({
                    "name": n,
                    "turtle": turtle,
                    "loaded_at": loaded_at
                })),
                None => Err(Error::InvalidValue(format!("no shape set named '{name}'"))),
            }
        }
        "list" => list_shapes_json(store),
        "vocabulary" => vocabulary_json(store),
        // An unrecognized action USED TO FALL THROUGH TO `list`, so a typo
        // ("laod") returned HTTP 200 and a plausible shape list — success by
        // every signal a caller can see, having done nothing. Same silent-no-op
        // class as `bd label` exiting 0 (aegis-oe10). A missing action still
        // defaults to "list" above, which keeps a bare `{}` probe working.
        other => Err(Error::InvalidValue(format!(
            "unknown shapes action '{other}' (expected: load, list, get, remove, vocabulary)"
        ))),
    }
}

/// Return the class IRIs sanctioned by the shapes currently loaded in the
/// server.  `list` only names shape sets; it cannot answer whether a proposed
/// rdf:type is governed.  Target classes plus declared superclass objects form
/// the accepted vocabulary: abstract parents such as Service and Host are
/// query-only classes without their own target shape, but remain valid IRIs.
fn vocabulary_json(store: &Store) -> Result<JsonValue> {
    use oxttl::TurtleParser;

    const TARGET_CLASS: &str = "http://www.w3.org/ns/shacl#targetClass";
    const SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

    let mut classes = BTreeSet::new();
    for (name, turtle, _) in store.list_shapes()? {
        let parser = TurtleParser::new()
            .with_base_iri("http://example.org/")
            .map_err(|e| Error::InvalidValue(format!("shape set '{name}' base IRI: {e}")))?;
        for result in parser.for_reader(turtle.as_bytes()) {
            let triple = result.map_err(|e| {
                Error::InvalidValue(format!("shape set '{name}' Turtle parse error: {e}"))
            })?;
            let predicate = triple.predicate.as_str();
            if (predicate == TARGET_CLASS || predicate == SUBCLASS_OF)
                && let oxrdf::Term::NamedNode(class) = triple.object
            {
                classes.insert(class.as_str().to_owned());
            }
        }
    }
    Ok(serde_json::json!({
        "classes": classes,
        "count": classes.len(),
        "basis": ["sh:targetClass", "rdfs:subClassOf object"]
    }))
}

fn list_shapes_json(store: &Store) -> Result<JsonValue> {
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
