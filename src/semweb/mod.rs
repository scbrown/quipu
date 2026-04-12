//! Semantic web API logic — entity conneg, Spotlight, TPF, Reconciliation, Preview.
//!
//! Pure functions that operate on a `Store` and return structured data.
//! The server module wraps these in thin HTTP handlers.

mod conneg;

pub use conneg::{entity_json_ld, entity_turtle, preview_card};

use serde_json::{Value as JsonValue, json};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

/// Extract the local name from an IRI (after last `#` or `/`).
pub fn short_name(iri: &str) -> String {
    let iri = iri.trim_start_matches('<').trim_end_matches('>');
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}

/// Minimal HTML escaping.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Decode percent-encoded IRI components.
pub fn decode_iri(iri: &str) -> String {
    iri.replace("%23", "#")
        .replace("%20", " ")
        .replace("%3A", ":")
}

/// An entity with label and type for matching operations.
struct LabeledEntity {
    iri: String,
    label: String,
    entity_type: String,
}

/// Fetch all entities with labels from the store.
fn fetch_labeled_entities(store: &Store) -> Result<Vec<LabeledEntity>> {
    let result = crate::sparql_query(
        store,
        "SELECT ?s ?label ?type WHERE { \
         ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . \
         OPTIONAL { ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type } \
         }",
    )?;

    let mut entities = Vec::new();
    for row in result.rows() {
        let iri = match row.get("s") {
            Some(Value::Ref(id)) => store.resolve(*id).unwrap_or_default(),
            _ => continue,
        };
        let label = match row.get("label") {
            Some(Value::Str(s)) => s.clone(),
            _ => continue,
        };
        let entity_type = match row.get("type") {
            Some(Value::Ref(id)) => store.resolve(*id).unwrap_or_default(),
            _ => String::new(),
        };
        entities.push(LabeledEntity {
            iri,
            label,
            entity_type,
        });
    }
    Ok(entities)
}

/// Annotate text with entity mentions from the knowledge graph.
pub fn spotlight(store: &Store, text: &str, confidence: f64) -> Result<JsonValue> {
    let entities = fetch_labeled_entities(store)?;
    let text_lower = text.to_lowercase();
    let mut annotations = Vec::new();

    for entity in &entities {
        let label_lower = entity.label.to_lowercase();
        let iri_short = short_name(&entity.iri).to_lowercase();
        let entity_type_short = if entity.entity_type.is_empty() {
            "Thing".to_string()
        } else {
            short_name(&entity.entity_type)
        };

        for (surface, score_base) in [(&label_lower, 0.95), (&iri_short, 0.85)] {
            if surface.is_empty() {
                continue;
            }
            let mut search_from = 0;
            while let Some(pos) = text_lower[search_from..].find(surface.as_str()) {
                let abs_pos = search_from + pos;
                let surface_text = &text[abs_pos..abs_pos + surface.len()];
                if score_base >= confidence {
                    annotations.push(json!({
                        "surface": surface_text,
                        "iri": entity.iri,
                        "type": entity_type_short,
                        "confidence": score_base,
                        "offset": abs_pos,
                    }));
                }
                search_from = abs_pos + surface.len();
            }
        }
    }

    // Deduplicate by offset (keep highest confidence).
    annotations.sort_by(|a, b| {
        let ao = a["offset"].as_u64().unwrap_or(0);
        let bo = b["offset"].as_u64().unwrap_or(0);
        ao.cmp(&bo).then_with(|| {
            let ac = a["confidence"].as_f64().unwrap_or(0.0);
            let bc = b["confidence"].as_f64().unwrap_or(0.0);
            bc.partial_cmp(&ac).unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    annotations.dedup_by(|a, b| a["offset"] == b["offset"] && a["iri"] == b["iri"]);

    Ok(json!({ "text": text, "annotations": annotations }))
}

/// Query parameters for the TPF endpoint.
pub struct FragmentQuery {
    /// Filter by subject IRI.
    pub subject: Option<String>,
    /// Filter by predicate IRI.
    pub predicate: Option<String>,
    /// Filter by object (IRI or literal).
    pub object: Option<String>,
    /// Page number (1-indexed).
    pub page: usize,
    /// Results per page.
    pub page_size: usize,
}

/// Query matching triples with pagination.
pub fn fragments(store: &Store, query: &FragmentQuery) -> Result<JsonValue> {
    let s_bind = query.subject.as_deref().map(|s| format!("<{s}>"));
    let p_bind = query.predicate.as_deref().map(|p| format!("<{p}>"));
    let o_bind = query.object.as_deref();

    let s_part = s_bind.as_deref().unwrap_or("?s");
    let p_part = p_bind.as_deref().unwrap_or("?p");

    let o_part = match o_bind {
        Some(o)
            if o.starts_with("http://") || o.starts_with("https://") || o.starts_with("urn:") =>
        {
            format!("<{o}>")
        }
        Some(o) => format!("\"{o}\""),
        None => "?o".to_string(),
    };

    let sparql = format!("SELECT ?s ?p ?o WHERE {{ {s_part} {p_part} {o_part} }}");
    let result = crate::sparql_query(store, &sparql)?;

    let total = result.rows().len();
    let start = (query.page - 1) * query.page_size;
    let page_rows: Vec<JsonValue> = result
        .rows()
        .iter()
        .skip(start)
        .take(query.page_size)
        .map(|row| {
            let s = match row.get("s") {
                Some(Value::Ref(id)) => store.resolve(*id).unwrap_or_default(),
                _ => query.subject.clone().unwrap_or_default(),
            };
            let p = match row.get("p") {
                Some(Value::Ref(id)) => store.resolve(*id).unwrap_or_default(),
                _ => query.predicate.clone().unwrap_or_default(),
            };
            let o = match row.get("o") {
                Some(val) => crate::value_to_json(store, val),
                _ => json!(query.object.clone().unwrap_or_default()),
            };
            json!({"subject": s, "predicate": p, "object": o})
        })
        .collect();

    let total_pages = total.div_ceil(query.page_size);
    let has_next = query.page < total_pages;

    let mut response = json!({
        "triples": page_rows,
        "totalCount": total,
        "page": query.page,
        "pageSize": query.page_size,
    });

    // Hypermedia controls.
    if has_next {
        let mut params = Vec::new();
        if let Some(ref s) = query.subject {
            params.push(format!("subject={s}"));
        }
        if let Some(ref p) = query.predicate {
            params.push(format!("predicate={p}"));
        }
        if let Some(ref o) = query.object {
            params.push(format!("object={o}"));
        }
        params.push(format!("page={}", query.page + 1));
        params.push(format!("pageSize={}", query.page_size));
        response["controls"] = json!({"next": format!("/fragments?{}", params.join("&"))});
    } else {
        response["controls"] = json!({});
    }

    Ok(response)
}

/// Service manifest for the `OpenRefine` Reconciliation API.
pub fn reconcile_manifest() -> JsonValue {
    json!({
        "name": "Quipu Knowledge Graph",
        "identifierSpace": "https://quipu.dev/entity/",
        "schemaSpace": "https://quipu.dev/ontology/",
        "view": {"url": "/entity/{{id}}"},
        "preview": {"url": "/preview/{{id}}", "width": 400, "height": 300},
        "defaultTypes": [],
    })
}

/// Match entity queries against the knowledge graph.
pub fn reconcile(store: &Store, queries: &serde_json::Map<String, JsonValue>) -> Result<JsonValue> {
    let entities = fetch_labeled_entities(store)?;
    let mut results = serde_json::Map::new();

    for (query_id, query_val) in queries {
        let query_text = query_val
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let limit = query_val
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5) as usize;
        let type_filter = query_val.get("type").and_then(serde_json::Value::as_str);

        let query_lower = query_text.to_lowercase();
        let mut candidates: Vec<JsonValue> = entities
            .iter()
            .filter(|e| {
                if let Some(tf) = type_filter {
                    return e.entity_type.contains(tf);
                }
                true
            })
            .filter_map(|e| {
                let label_lower = e.label.to_lowercase();
                let short_lower = short_name(&e.iri).to_lowercase();
                let type_short = short_name(&e.entity_type);

                let score = if label_lower == query_lower || short_lower == query_lower {
                    100.0
                } else if label_lower.contains(&query_lower) || short_lower.contains(&query_lower) {
                    let max_len = label_lower.len().max(short_lower.len());
                    let ratio = query_lower.len() as f64 / max_len as f64;
                    (ratio * 80.0).min(95.0)
                } else {
                    return None;
                };

                Some(json!({
                    "id": e.iri,
                    "name": e.label,
                    "type": [{"id": e.entity_type, "name": type_short}],
                    "score": score,
                    "match": score >= 100.0,
                }))
            })
            .collect();

        candidates.sort_by(|a, b| {
            let sa = a["score"].as_f64().unwrap_or(0.0);
            let sb = b["score"].as_f64().unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(limit);

        results.insert(query_id.clone(), json!({"result": candidates}));
    }

    Ok(serde_json::Value::Object(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_name_extracts_local() {
        assert_eq!(short_name("http://example.org/foo"), "foo");
        assert_eq!(short_name("http://example.org/ns#Bar"), "Bar");
        assert_eq!(short_name("plain"), "plain");
    }

    #[test]
    fn html_escape_covers_special_chars() {
        assert_eq!(
            html_escape("<b>\"hi\"&</b>"),
            "&lt;b&gt;&quot;hi&quot;&amp;&lt;/b&gt;"
        );
    }

    #[test]
    fn spotlight_empty_store() {
        let store = Store::open_in_memory().unwrap();
        let result = spotlight(&store, "hello world", 0.5).unwrap();
        let annotations = result["annotations"].as_array().unwrap();
        assert!(annotations.is_empty());
    }

    #[test]
    fn fragments_empty_store() {
        let store = Store::open_in_memory().unwrap();
        let q = FragmentQuery {
            subject: None,
            predicate: None,
            object: None,
            page: 1,
            page_size: 10,
        };
        let result = fragments(&store, &q).unwrap();
        assert_eq!(result["totalCount"], 0);
        assert!(result["triples"].as_array().unwrap().is_empty());
    }

    #[test]
    fn reconcile_manifest_has_required_fields() {
        let m = reconcile_manifest();
        assert!(m.get("name").is_some());
        assert!(m.get("identifierSpace").is_some());
        assert!(m.get("schemaSpace").is_some());
    }

    #[test]
    fn reconcile_empty_store() {
        let store = Store::open_in_memory().unwrap();
        let queries = serde_json::Map::new();
        let result = reconcile(&store, &queries).unwrap();
        assert!(result.as_object().unwrap().is_empty());
    }
}
