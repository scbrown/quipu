//! MCP tool handlers for Quipu — the agent-facing API surface.
//!
//! Each function takes JSON input and returns JSON output, matching the
//! Model Context Protocol tool calling convention. Bobbin's MCP server
//! delegates knowledge graph operations to these handlers.

use serde_json::Value as JsonValue;

use crate::episode::{self, Episode};
use crate::error::{Error, Result};
use crate::rdf::ingest_rdf;
use crate::shacl;
use crate::sparql;
use crate::store::{AsOf, Store};
use crate::types::Value;

/// MCP tool: `quipu_query` — Execute a SPARQL SELECT query.
///
/// Input: `{ "query": "SELECT ..." }`
/// Output: `{ "variables": [...], "rows": [{ "var": "value", ... }, ...] }`
pub fn tool_query(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let query_str = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'query' parameter".into()))?;

    let result = sparql::query(store, query_str)?;

    let rows: Vec<JsonValue> = result
        .rows
        .iter()
        .map(|row| {
            let obj: serde_json::Map<String, JsonValue> = row
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(store, v)))
                .collect();
            JsonValue::Object(obj)
        })
        .collect();

    Ok(serde_json::json!({
        "variables": result.variables,
        "rows": rows,
        "count": rows.len()
    }))
}

/// MCP tool: `quipu_knot` — Assert facts with optional SHACL validation.
///
/// Input: `{ "turtle": "<data>", "timestamp": "...", "actor": "...",
///           "source": "...", "shapes": "<optional shapes turtle>" }`
/// Output: `{ "tx_id": N, "count": N }` or validation feedback on failure.
pub fn tool_knot(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let turtle = input
        .get("turtle")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'turtle' parameter".into()))?;

    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    let actor = input.get("actor").and_then(|v| v.as_str());
    let source = input.get("source").and_then(|v| v.as_str());

    // SHACL validation: combine per-request shapes with stored shapes.
    let request_shapes = input.get("shapes").and_then(|v| v.as_str());
    let stored_shapes = store.get_combined_shapes()?;

    let combined_shapes = match (request_shapes, &stored_shapes) {
        (Some(req), Some(stored)) => Some(format!("{stored}\n\n{req}")),
        (Some(req), None) => Some(req.to_string()),
        (None, Some(stored)) => Some(stored.clone()),
        (None, None) => None,
    };

    if let Some(shapes) = &combined_shapes {
        let feedback = shacl::validate_shapes(shapes, turtle)?;
        if !feedback.conforms {
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
            return Ok(serde_json::json!({
                "conforms": false,
                "violations": feedback.violations,
                "warnings": feedback.warnings,
                "issues": issues
            }));
        }
    }

    let (tx_id, count) = ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        timestamp,
        actor,
        source,
    )?;

    Ok(serde_json::json!({
        "conforms": true,
        "tx_id": tx_id,
        "count": count
    }))
}

/// MCP tool: `quipu_cord` — List entities matching a pattern.
///
/// Input: `{ "type": "<optional IRI>", "predicate": "<optional IRI>",
///           "limit": N }`
/// Output: `{ "entities": [{ "iri": "...", "facts": [...] }, ...] }`
pub fn tool_cord(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    // Build a SPARQL query based on the filters.
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
    for row in &result.rows {
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

/// MCP tool: `quipu_unravel` — Time-travel query.
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

/// MCP tool: `quipu_validate` — Validate data against shapes.
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

    let feedback = shacl::validate_shapes(shapes, data)?;

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

/// MCP tool: `quipu_search` — Semantic vector search over entity embeddings.
///
/// Input: `{ "embedding": [f32...], "limit": N, "valid_at": "..." }`
/// Output: `{ "results": [{ "entity": "...", "text": "...", "score": N }, ...] }`
///
/// Note: The caller must provide pre-computed embeddings. When integrated with
/// Bobbin, the MCP server can embed the query text using the ONNX pipeline
/// before calling this tool.
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

/// MCP tool: `quipu_shapes` — Manage persistent SHACL shapes.
///
/// Input: `{ "action": "load|list|remove", "name": "...", "turtle": "...", "timestamp": "..." }`
/// Output depends on action.
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

            // Validate the shapes parse correctly.
            shacl::validate_shapes(turtle, "@prefix ex: <http://example.org/> .\n")?;

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
        "list" | _ => {
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

/// MCP tool: `quipu_retract` — Retract facts for an entity.
///
/// Input: `{ "entity": "<IRI>", "predicate": "<optional IRI>", "timestamp": "..." }`
/// Output: `{ "tx_id": N, "retracted": N }`
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

/// MCP tool: `quipu_episode` — Ingest structured knowledge from an agent episode.
///
/// Input: `{ "name": "...", "episode_body": "...", "source": "...",
///           "group_id": "...", "timestamp": "...",
///           "nodes": [{ "name": "...", "type": "...", "description": "..." }],
///           "edges": [{ "source": "...", "target": "...", "relation": "..." }] }`
/// Output: `{ "tx_id": N, "count": N, "episode": "..." }`
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

/// MCP tool definitions as JSON schemas for registration with Bobbin.
pub fn tool_definitions() -> Vec<JsonValue> {
    vec![
        serde_json::json!({
            "name": "quipu_query",
            "description": "Execute a SPARQL SELECT query against the knowledge graph",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "SPARQL SELECT query"
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_knot",
            "description": "Assert facts into the knowledge graph (with optional SHACL validation)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turtle": {
                        "type": "string",
                        "description": "RDF data in Turtle format to assert"
                    },
                    "timestamp": {
                        "type": "string",
                        "description": "ISO-8601 timestamp for the assertion"
                    },
                    "actor": {
                        "type": "string",
                        "description": "Who is making the assertion"
                    },
                    "source": {
                        "type": "string",
                        "description": "Provenance source (episode, file, etc.)"
                    },
                    "shapes": {
                        "type": "string",
                        "description": "Optional SHACL shapes in Turtle for validation"
                    }
                },
                "required": ["turtle"]
            }
        }),
        serde_json::json!({
            "name": "quipu_cord",
            "description": "List entities in the knowledge graph, optionally filtered by type or predicate",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "description": "Filter by rdf:type IRI"
                    },
                    "predicate": {
                        "type": "string",
                        "description": "Filter by predicate IRI"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entities (default: 100)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_unravel",
            "description": "Time-travel query: see facts as they were at a given point",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tx": {
                        "type": "integer",
                        "description": "Maximum transaction ID to consider"
                    },
                    "valid_at": {
                        "type": "string",
                        "description": "Point-in-time for valid-time filtering (ISO-8601)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_validate",
            "description": "Validate RDF data against SHACL shapes without writing",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "shapes": {
                        "type": "string",
                        "description": "SHACL shapes in Turtle format"
                    },
                    "data": {
                        "type": "string",
                        "description": "RDF data in Turtle format to validate"
                    }
                },
                "required": ["shapes", "data"]
            }
        }),
        serde_json::json!({
            "name": "quipu_shapes",
            "description": "Manage persistent SHACL shapes (load, list, remove). Loaded shapes auto-validate on writes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["load", "list", "remove"],
                        "description": "Action to perform (default: list)"
                    },
                    "name": {
                        "type": "string",
                        "description": "Shape graph name (required for load/remove)"
                    },
                    "turtle": {
                        "type": "string",
                        "description": "SHACL shapes in Turtle format (required for load)"
                    },
                    "timestamp": {
                        "type": "string",
                        "description": "ISO-8601 timestamp"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_retract",
            "description": "Retract facts for an entity (all facts, or filtered by predicate)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": {
                        "type": "string",
                        "description": "IRI of the entity to retract"
                    },
                    "predicate": {
                        "type": "string",
                        "description": "Optional: only retract facts with this predicate IRI"
                    },
                    "timestamp": {
                        "type": "string",
                        "description": "ISO-8601 timestamp for the retraction"
                    },
                    "actor": {
                        "type": "string",
                        "description": "Who is performing the retraction"
                    }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "quipu_episode",
            "description": "Ingest structured knowledge from an agent episode (nodes + edges)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Episode name/identifier"
                    },
                    "episode_body": {
                        "type": "string",
                        "description": "Natural language description of the knowledge"
                    },
                    "source": {
                        "type": "string",
                        "description": "Who/what produced this episode"
                    },
                    "group_id": {
                        "type": "string",
                        "description": "Knowledge graph group (e.g. aegis-ontology)"
                    },
                    "timestamp": {
                        "type": "string",
                        "description": "ISO-8601 timestamp for the assertion"
                    },
                    "nodes": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "type": { "type": "string" },
                                "description": { "type": "string" },
                                "properties": { "type": "object" }
                            },
                            "required": ["name"]
                        },
                        "description": "Entity nodes to create"
                    },
                    "edges": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "source": { "type": "string" },
                                "target": { "type": "string" },
                                "relation": { "type": "string" }
                            },
                            "required": ["source", "target", "relation"]
                        },
                        "description": "Relationship edges between nodes"
                    }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_search",
            "description": "Semantic vector search over entity embeddings (requires pre-computed embedding)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "embedding": {
                        "type": "array",
                        "items": { "type": "number" },
                        "description": "Query embedding vector (f32 array)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results (default: 10)"
                    },
                    "valid_at": {
                        "type": "string",
                        "description": "Point-in-time for temporal filtering (ISO-8601)"
                    }
                },
                "required": ["embedding"]
            }
        }),
    ]
}

// ── Helpers ──────────────────────────────────────────────────────

fn value_to_json(store: &Store, val: &Value) -> JsonValue {
    match val {
        Value::Ref(id) => {
            let iri = store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}"));
            JsonValue::String(iri)
        }
        Value::Str(s) => JsonValue::String(s.clone()),
        Value::Int(n) => serde_json::json!(n),
        Value::Float(f) => serde_json::json!(f),
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Bytes(b) => JsonValue::String(format!("<{} bytes>", b.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store_with_data() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ; ex:name "Alice" ; ex:age "30"^^xsd:integer .
ex:bob a ex:Person ; ex:name "Bob" ; ex:age "25"^^xsd:integer .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    #[test]
    fn test_tool_query() {
        let store = test_store_with_data();
        let input = serde_json::json!({
            "query": "SELECT ?name WHERE { ?s <http://example.org/name> ?name }"
        });
        let result = tool_query(&store, &input).unwrap();
        assert_eq!(result["count"], 2);
        assert_eq!(result["variables"], serde_json::json!(["name"]));
    }

    #[test]
    fn test_tool_knot() {
        let mut store = Store::open_in_memory().unwrap();
        let input = serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:carol a ex:Person ; ex:name \"Carol\" .",
            "timestamp": "2026-04-04T01:00:00Z",
            "actor": "test",
            "source": "unit-test"
        });
        let result = tool_knot(&mut store, &input).unwrap();
        assert_eq!(result["conforms"], true);
        assert_eq!(result["count"], 2);
        assert!(result["tx_id"].as_i64().unwrap() > 0);
    }

    #[test]
    fn test_tool_knot_with_validation_failure() {
        let mut store = Store::open_in_memory().unwrap();
        let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
"#;
        let input = serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:bad a ex:Person .",
            "shapes": shapes,
            "timestamp": "2026-04-04T01:00:00Z"
        });
        let result = tool_knot(&mut store, &input).unwrap();
        assert_eq!(result["conforms"], false);
        assert!(result["violations"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_tool_cord() {
        let store = test_store_with_data();
        let input = serde_json::json!({
            "type": "http://example.org/Person"
        });
        let result = tool_cord(&store, &input).unwrap();
        assert_eq!(result["count"], 2);
    }

    #[test]
    fn test_tool_cord_all() {
        let store = test_store_with_data();
        let input = serde_json::json!({ "limit": 10 });
        let result = tool_cord(&store, &input).unwrap();
        assert!(result["count"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn test_tool_unravel() {
        let mut store = Store::open_in_memory().unwrap();

        // TX 1
        ingest_rdf(
            &mut store,
            "@prefix ex: <http://example.org/> .\nex:a ex:v \"1\" .".as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // TX 2
        ingest_rdf(
            &mut store,
            "@prefix ex: <http://example.org/> .\nex:b ex:v \"2\" .".as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-02-01",
            None,
            None,
        )
        .unwrap();

        // As of TX 1, should only see first triple.
        let input = serde_json::json!({ "tx": 1 });
        let result = tool_unravel(&store, &input).unwrap();
        assert_eq!(result["count"], 1);
    }

    #[test]
    fn test_tool_validate() {
        let input = serde_json::json!({
            "shapes": "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:property [ sh:path ex:name ; sh:minCount 1 ] .",
            "data": "@prefix ex: <http://example.org/> .\nex:x a ex:T ; ex:name \"ok\" ."
        });
        let result = tool_validate(&input).unwrap();
        assert_eq!(result["conforms"], true);
    }

    #[test]
    fn test_tool_episode() {
        let mut store = Store::open_in_memory().unwrap();
        let input = serde_json::json!({
            "name": "deploy-event",
            "episode_body": "Deployed new version of tapestry to ct-236",
            "source": "crew/mayor",
            "group_id": "aegis-ontology",
            "timestamp": "2026-04-04T12:00:00Z",
            "nodes": [
                {"name": "tapestry", "type": "WebApplication", "description": "Web UI"},
                {"name": "ct-236", "type": "LXCContainer"}
            ],
            "edges": [
                {"source": "tapestry", "target": "ct-236", "relation": "deployed_on"}
            ]
        });
        let result = tool_episode(&mut store, &input).unwrap();
        assert_eq!(result["episode"], "deploy-event");
        assert!(result["tx_id"].as_i64().unwrap() > 0);
        assert!(result["count"].as_i64().unwrap() >= 10);
    }

    #[test]
    fn test_tool_retract_entity() {
        let mut store = Store::open_in_memory().unwrap();
        let turtle = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .";
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        assert_eq!(store.current_facts().unwrap().len(), 2);

        let input = serde_json::json!({
            "entity": "http://example.org/alice",
            "timestamp": "2026-02-01"
        });
        let result = tool_retract(&mut store, &input).unwrap();
        assert_eq!(result["retracted"], 2);
        assert!(result["tx_id"].as_i64().unwrap() > 0);

        assert_eq!(store.current_facts().unwrap().len(), 0);
    }

    #[test]
    fn test_tool_retract_predicate() {
        let mut store = Store::open_in_memory().unwrap();
        let turtle = "@prefix ex: <http://example.org/> .\nex:bob a ex:Person ; ex:name \"Bob\" ; ex:age \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        assert_eq!(store.current_facts().unwrap().len(), 3);

        // Retract only the name predicate.
        let input = serde_json::json!({
            "entity": "http://example.org/bob",
            "predicate": "http://example.org/name",
            "timestamp": "2026-02-01"
        });
        let result = tool_retract(&mut store, &input).unwrap();
        assert_eq!(result["retracted"], 1);

        // 2 facts remain (type + age).
        assert_eq!(store.current_facts().unwrap().len(), 2);
    }

    #[test]
    fn test_tool_shapes_load_and_enforce() {
        let mut store = Store::open_in_memory().unwrap();

        // Load a shape that requires ex:name on ex:Person.
        let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
"#;
        let load_input = serde_json::json!({
            "action": "load",
            "name": "person-rules",
            "turtle": shapes,
            "timestamp": "2026-04-04"
        });
        tool_shapes(&store, &load_input).unwrap();

        // Verify it's listed.
        let list_input = serde_json::json!({ "action": "list" });
        let list_result = tool_shapes(&store, &list_input).unwrap();
        assert_eq!(list_result["count"], 1);

        // Write valid data — should succeed.
        let good_input = serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
            "timestamp": "2026-04-04T01:00:00Z"
        });
        let good_result = tool_knot(&mut store, &good_input).unwrap();
        assert_eq!(good_result["conforms"], true);

        // Write invalid data — should fail validation.
        let bad_input = serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:bob a ex:Person .",
            "timestamp": "2026-04-04T02:00:00Z"
        });
        let bad_result = tool_knot(&mut store, &bad_input).unwrap();
        assert_eq!(bad_result["conforms"], false);
    }

    #[test]
    fn test_tool_definitions() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 9);
        let names: Vec<&str> = defs
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"quipu_query"));
        assert!(names.contains(&"quipu_knot"));
        assert!(names.contains(&"quipu_cord"));
        assert!(names.contains(&"quipu_unravel"));
        assert!(names.contains(&"quipu_validate"));
        assert!(names.contains(&"quipu_search"));
        assert!(names.contains(&"quipu_episode"));
        assert!(names.contains(&"quipu_retract"));
        assert!(names.contains(&"quipu_shapes"));
    }
}
