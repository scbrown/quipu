//! API client — talks to the quipu-server REST endpoints.

use gloo_net::http::Request;
use serde_json::{Value, json};

use crate::components::graph_explorer::{EntityNode, Fact};

/// Base URL for the API. In dev, Trunk proxies to the backend.
fn base_url() -> String {
    let window = web_sys::window().unwrap();
    let location = window.location();
    let origin = location.origin().unwrap_or_else(|_| String::new());
    // If running via Trunk dev server, API is at same origin (proxied)
    // If served by quipu-server directly, also same origin
    origin
}

/// Fetch the initial graph via SPARQL — all entities with types and labels.
pub async fn fetch_initial_graph() -> Result<(Vec<EntityNode>, Vec<(String, String, String)>), String>
{
    let query = r#"
        SELECT ?s ?p ?o WHERE { ?s ?p ?o }
    "#;

    let url = format!("{}/query", base_url());
    let resp = Request::post(&url)
        .json(&json!({ "sparql": query }))
        .map_err(|e| format!("Request build error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.ok() {
        return Err(format!("API error: {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))?;

    parse_graph_response(&body)
}

/// Expand 1-hop neighborhood of a node via SPARQL.
pub async fn expand_neighborhood(
    iri: &str,
) -> Result<(Vec<EntityNode>, Vec<(String, String, String)>), String> {
    // Query outgoing and incoming edges
    let query = format!(
        r#"SELECT ?s ?p ?o WHERE {{
            {{ <{iri}> ?p ?o . BIND(<{iri}> AS ?s) }}
            UNION
            {{ ?s ?p <{iri}> . BIND(<{iri}> AS ?o) }}
        }}"#,
    );

    let url = format!("{}/query", base_url());
    let resp = Request::post(&url)
        .json(&json!({ "sparql": query }))
        .map_err(|e| format!("Request build error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.ok() {
        return Err(format!("API error: {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))?;

    parse_graph_response(&body)
}

/// Fetch facts for a specific entity.
pub async fn fetch_entity_facts(iri: &str) -> Result<Vec<Fact>, String> {
    let query = format!(
        r#"SELECT ?p ?o WHERE {{ <{iri}> ?p ?o }}"#,
    );

    let url = format!("{}/query", base_url());
    let resp = Request::post(&url)
        .json(&json!({ "sparql": query }))
        .map_err(|e| format!("Request build error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.ok() {
        return Err(format!("API error: {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))?;

    parse_facts_response(&body)
}

/// Execute a SPARQL query and return raw JSON results.
pub async fn sparql_query(sparql: &str) -> Result<Value, String> {
    let url = format!("{}/query", base_url());
    let resp = Request::post(&url)
        .json(&json!({ "sparql": sparql }))
        .map_err(|e| format!("Request build error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.ok() {
        return Err(format!("API error: {}", resp.status()));
    }

    resp.json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))
}

/// Fetch server stats.
pub async fn fetch_stats() -> Result<Value, String> {
    let url = format!("{}/stats", base_url());
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    resp.json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))
}

// ── Response parsing ──────────────────────────────────────────────────

/// Parse SPARQL query results into nodes and edges for the graph.
fn parse_graph_response(
    body: &Value,
) -> Result<(Vec<EntityNode>, Vec<(String, String, String)>), String> {
    let mut nodes = std::collections::HashMap::new();
    let mut edges = Vec::new();

    // The quipu query endpoint returns { columns: [...], rows: [...] }
    let rows = body
        .get("rows")
        .and_then(|r| r.as_array())
        .unwrap_or(&Vec::new())
        .clone();

    for row in &rows {
        let s = extract_value(row, "s");
        let p = extract_value(row, "p");
        let o = extract_value(row, "o");

        if let (Some(s_val), Some(p_val), Some(o_val)) = (&s, &p, &o) {
            // Add subject as node
            if is_iri(s_val) && !nodes.contains_key(s_val) {
                nodes.insert(
                    s_val.clone(),
                    EntityNode {
                        iri: s_val.clone(),
                        label: short_name(s_val),
                        entity_type: "default".to_string(),
                    },
                );
            }

            // If predicate is rdf:type, update the entity type
            if p_val.contains("type") || p_val.contains("rdf:type") || p_val.ends_with("#type") {
                if let Some(node) = nodes.get_mut(s_val) {
                    node.entity_type = short_name(o_val);
                }
            }

            // If object is an IRI, add as node and create edge
            if is_iri(o_val) {
                if !nodes.contains_key(o_val) {
                    nodes.insert(
                        o_val.clone(),
                        EntityNode {
                            iri: o_val.clone(),
                            label: short_name(o_val),
                            entity_type: "default".to_string(),
                        },
                    );
                }
                edges.push((s_val.clone(), o_val.clone(), short_name(p_val)));
            }
        }
    }

    Ok((nodes.into_values().collect(), edges))
}

/// Parse facts response for a single entity.
fn parse_facts_response(body: &Value) -> Result<Vec<Fact>, String> {
    let rows = body
        .get("rows")
        .and_then(|r| r.as_array())
        .unwrap_or(&Vec::new())
        .clone();

    let mut facts = Vec::new();
    for row in &rows {
        let p = extract_value(row, "p");
        let o = extract_value(row, "o");

        if let (Some(p_val), Some(o_val)) = (p, o) {
            facts.push(Fact {
                predicate: p_val,
                value: o_val.clone(),
                is_iri: is_iri(&o_val),
            });
        }
    }

    Ok(facts)
}

/// Extract a binding value from a SPARQL result row.
fn extract_value(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(|v| {
            // Handle both string values and {type, value} objects
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else if let Some(val) = v.get("value").and_then(|v| v.as_str()) {
                Some(val.to_string())
            } else {
                v.as_str().map(String::from)
            }
        })
}

/// Check if a value looks like an IRI.
fn is_iri(val: &str) -> bool {
    val.starts_with("http://")
        || val.starts_with("https://")
        || val.starts_with("urn:")
        || (val.starts_with('<') && val.ends_with('>'))
}

/// Extract the local name from an IRI.
fn short_name(iri: &str) -> String {
    let iri = iri.trim_start_matches('<').trim_end_matches('>');
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}
