//! Named-dataset management: `quipu_datasets` (quipu #69).

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::store::datasets::DatasetMember;

/// MCP/HTTP tool: `quipu_datasets` — create / list / show / remove a named
/// graph set.
///
/// An unknown action errors rather than falling through to `list` — the
/// recorded `tool_shapes` silent-fall-through lesson: a typo'd action that
/// quietly lists is indistinguishable from one that did what you asked.
///
/// # Errors
/// Unknown action, missing/misshapen parameters, or a store refusal (duplicate
/// ranks in a declared ordering, no members, missing meta-graph authority).
pub fn tool_datasets(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    match action {
        "create" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name'".into()))?;
            let raw = input
                .get("members")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    Error::InvalidValue(
                        "missing 'members' — an array of graph IRIs, or of \
                         {\"graph\": \"<iri>\", \"ord\": N} for a declared ordering"
                            .into(),
                    )
                })?;

            let mut members = Vec::with_capacity(raw.len());
            for m in raw {
                if let Some(iri) = m.as_str() {
                    members.push(DatasetMember::new(iri));
                } else if let Some(obj) = m.as_object() {
                    let iri = obj
                        .get("graph")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Error::InvalidValue("member object needs 'graph'".into()))?;
                    match obj.get("ord").and_then(serde_json::Value::as_i64) {
                        Some(o) => members.push(DatasetMember::ranked(iri, o)),
                        None => members.push(DatasetMember::new(iri)),
                    }
                } else {
                    return Err(Error::InvalidValue(
                        "each member must be a graph IRI string or {\"graph\", \"ord\"}".into(),
                    ));
                }
            }

            let timestamp = input
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map_or_else(crate::time::now_iso, String::from);
            let actor = input.get("actor").and_then(|v| v.as_str());

            store.dataset_create(name, &members, &timestamp, actor)?;
            Ok(serde_json::json!({
                "created": name,
                "members": members.len(),
            }))
        }
        "list" => Ok(serde_json::json!({ "datasets": store.dataset_list()? })),
        "show" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name'".into()))?;
            if !store.is_dataset(name)? {
                return Err(Error::InvalidValue(format!("no such dataset: {name}")));
            }
            let members: Vec<JsonValue> = store
                .dataset_members(name)?
                .into_iter()
                .map(|m| serde_json::json!({ "graph": m.graph_iri, "ord": m.ord }))
                .collect();
            Ok(serde_json::json!({ "name": name, "members": members }))
        }
        "remove" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name'".into()))?;
            Ok(serde_json::json!({ "removed": store.dataset_remove(name)? }))
        }
        other => Err(Error::InvalidValue(format!(
            "unknown datasets action '{other}' (create|list|show|remove)"
        ))),
    }
}
