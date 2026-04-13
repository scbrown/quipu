//! MCP tool implementation for OWL ontology management.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::owl::Ontology;
use crate::store::Store;

/// MCP tool: `quipu_load_ontology` -- Load, list, or remove OWL ontologies.
///
/// Actions:
/// - `load`: Parse and store an OWL ontology from Turtle, then materialize.
/// - `list`: List stored ontologies.
/// - `remove`: Remove a stored ontology by name.
pub fn tool_load_ontology(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    match action {
        "load" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;
            let turtle = input
                .get("turtle")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'turtle' parameter".into()))?;
            let timestamp = input
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("1970-01-01T00:00:00Z");

            // Parse and validate the ontology.
            let ontology = Ontology::from_turtle(turtle)?;
            let summary = ontology.axiom_summary();

            // Persist.
            store.load_ontology(name, turtle, timestamp)?;

            // Materialize entailments.
            let report = ontology.materialize(store, timestamp)?;

            Ok(serde_json::json!({
                "action": "load",
                "name": name,
                "axioms": summary,
                "materialized": {
                    "subclass_inferences": report.subclass_inferences,
                    "inverse_inferences": report.inverse_inferences,
                    "symmetric_inferences": report.symmetric_inferences,
                    "equivalent_class_inferences": report.equivalent_class_inferences,
                    "domain_range_inferences": report.domain_range_inferences,
                    "total": report.total,
                }
            }))
        }
        "list" => {
            let ontologies = store.list_ontologies()?;
            let items: Vec<JsonValue> = ontologies
                .iter()
                .map(|(name, turtle, loaded_at)| {
                    let axiom_summary = Ontology::from_turtle(turtle).map_or_else(
                        |_| serde_json::json!({"error": "parse failed"}),
                        |o| o.axiom_summary(),
                    );
                    serde_json::json!({
                        "name": name,
                        "loaded_at": loaded_at,
                        "axioms": axiom_summary,
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "action": "list",
                "ontologies": items,
                "count": items.len(),
            }))
        }
        "remove" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;
            let removed = store.remove_ontology(name)?;
            Ok(serde_json::json!({
                "action": "remove",
                "name": name,
                "removed": removed,
            }))
        }
        _ => Err(Error::InvalidValue(format!(
            "unknown action '{action}'; use 'load', 'list', or 'remove'"
        ))),
    }
}
