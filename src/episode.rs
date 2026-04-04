//! Episode ingestion — structured write path for agent-extracted knowledge.
//!
//! Episodes are the primary unit of knowledge ingestion from Gas Town agents.
//! An episode contains a set of nodes (entities) and edges (relationships)
//! extracted from operational events, bead work, or infrastructure observations.
//!
//! This module converts the structured JSON format used by Graphiti/Gas Town
//! into RDF Turtle and writes it through the existing `ingest_rdf` pipeline.

use serde::Deserialize;

use crate::error::Result;
use crate::rdf::ingest_rdf;
use crate::store::Store;

const BASE_NS: &str = "http://aegis.gastown.local/ontology/";

/// An episode — a unit of knowledge to ingest.
#[derive(Debug, Deserialize)]
pub struct Episode {
    pub name: String,
    #[serde(default)]
    pub episode_body: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

/// A node (entity) extracted from an episode.
#[derive(Debug, Deserialize)]
pub struct Node {
    pub name: String,
    #[serde(rename = "type", default)]
    pub node_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub properties: Option<serde_json::Map<String, serde_json::Value>>,
}

/// An edge (relationship) between two nodes.
#[derive(Debug, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: String,
}

/// Ingest an episode into the store.
///
/// Converts nodes and edges to Turtle and writes via `ingest_rdf`.
/// Returns `(tx_id, triple_count)`.
pub fn ingest_episode(
    store: &mut Store,
    episode: &Episode,
    timestamp: &str,
) -> Result<(i64, usize)> {
    let turtle = episode_to_turtle(episode);
    let actor = episode.source.as_deref();
    let source_str = format!("episode:{}", episode.name);

    ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        timestamp,
        actor,
        Some(&source_str),
    )
}

// ── Turtle generation ──────────────────────────────────────────

fn episode_to_turtle(episode: &Episode) -> String {
    let mut ttl = String::new();

    // Prefixes.
    ttl.push_str(&format!("@prefix aegis: <{BASE_NS}> .\n"));
    ttl.push_str("@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n");
    ttl.push_str("@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n");
    ttl.push_str("@prefix prov: <http://www.w3.org/ns/prov#> .\n");
    ttl.push_str("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\n");

    let ep_local = sanitize_iri_local(&episode.name);

    // Episode provenance entity.
    ttl.push_str(&format!("aegis:episode_{ep_local} a prov:Activity ;\n"));
    ttl.push_str(&format!(
        "    rdfs:label \"{}\"",
        escape_turtle(&episode.name)
    ));
    if let Some(body) = &episode.episode_body {
        ttl.push_str(&format!(
            " ;\n    rdfs:comment \"{}\"",
            escape_turtle(body)
        ));
    }
    if let Some(source) = &episode.source {
        ttl.push_str(&format!(
            " ;\n    prov:wasAssociatedWith \"{}\"",
            escape_turtle(source)
        ));
    }
    if let Some(gid) = &episode.group_id {
        ttl.push_str(&format!(
            " ;\n    aegis:groupId \"{}\"",
            escape_turtle(gid)
        ));
    }
    ttl.push_str(" .\n\n");

    // Nodes.
    for node in &episode.nodes {
        let local = sanitize_iri_local(&node.name);
        ttl.push_str(&format!("aegis:{local}"));

        if let Some(ntype) = &node.node_type {
            let type_local = sanitize_iri_local(ntype);
            ttl.push_str(&format!(" a aegis:{type_local}"));
        }

        ttl.push_str(&format!(
            " ;\n    rdfs:label \"{}\"",
            escape_turtle(&node.name)
        ));

        if let Some(desc) = &node.description {
            ttl.push_str(&format!(
                " ;\n    rdfs:comment \"{}\"",
                escape_turtle(desc)
            ));
        }

        // Link to episode provenance.
        ttl.push_str(&format!(
            " ;\n    prov:wasGeneratedBy aegis:episode_{ep_local}"
        ));

        // Optional properties as typed literals.
        if let Some(props) = &node.properties {
            for (key, val) in props {
                let pred = sanitize_iri_local(key);
                match val {
                    serde_json::Value::String(s) => {
                        ttl.push_str(&format!(
                            " ;\n    aegis:{pred} \"{}\"",
                            escape_turtle(s)
                        ));
                    }
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            ttl.push_str(&format!(
                                " ;\n    aegis:{pred} \"{i}\"^^xsd:integer"
                            ));
                        } else if let Some(f) = n.as_f64() {
                            ttl.push_str(&format!(
                                " ;\n    aegis:{pred} \"{f}\"^^xsd:double"
                            ));
                        }
                    }
                    serde_json::Value::Bool(b) => {
                        ttl.push_str(&format!(
                            " ;\n    aegis:{pred} \"{b}\"^^xsd:boolean"
                        ));
                    }
                    _ => {} // skip arrays/objects/null
                }
            }
        }

        ttl.push_str(" .\n\n");
    }

    // Edges.
    for edge in &episode.edges {
        let src = sanitize_iri_local(&edge.source);
        let tgt = sanitize_iri_local(&edge.target);
        let rel = sanitize_iri_local(&edge.relation);
        ttl.push_str(&format!("aegis:{src} aegis:{rel} aegis:{tgt} .\n"));
    }

    ttl
}

/// Sanitize a name into a valid IRI local name.
fn sanitize_iri_local(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Escape special characters for Turtle string literals.
fn escape_turtle(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_episode(json: &str) -> Episode {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn deserialize_episode() {
        let ep = parse_episode(r#"{
            "name": "koror-discovery",
            "episode_body": "Discovered koror runs ct-205",
            "source": "crew/mayor",
            "group_id": "aegis-ontology",
            "nodes": [
                {"name": "koror", "type": "ProxmoxNode", "description": "Primary Proxmox node"},
                {"name": "ct-205", "type": "LXCContainer"}
            ],
            "edges": [
                {"source": "koror", "target": "ct-205", "relation": "runs_on"}
            ]
        }"#);

        assert_eq!(ep.name, "koror-discovery");
        assert_eq!(ep.nodes.len(), 2);
        assert_eq!(ep.edges.len(), 1);
        assert_eq!(ep.nodes[0].node_type.as_deref(), Some("ProxmoxNode"));
    }

    #[test]
    fn episode_to_turtle_generates_valid_rdf() {
        let ep = parse_episode(r#"{
            "name": "test-episode",
            "episode_body": "Test body",
            "source": "unit-test",
            "nodes": [
                {"name": "alpha", "type": "ServiceType", "description": "Alpha service"}
            ],
            "edges": []
        }"#);

        let ttl = episode_to_turtle(&ep);

        // Should contain prefixes.
        assert!(ttl.contains("@prefix aegis:"));
        assert!(ttl.contains("@prefix prov:"));

        // Should contain episode entity.
        assert!(ttl.contains("aegis:episode_test-episode a prov:Activity"));
        assert!(ttl.contains("rdfs:label \"test-episode\""));
        assert!(ttl.contains("rdfs:comment \"Test body\""));

        // Should contain node.
        assert!(ttl.contains("aegis:alpha a aegis:ServiceType"));
        assert!(ttl.contains("prov:wasGeneratedBy aegis:episode_test-episode"));
    }

    #[test]
    fn ingest_episode_writes_to_store() {
        let mut store = Store::open_in_memory().unwrap();

        let ep = parse_episode(r#"{
            "name": "infra-scan",
            "episode_body": "Infrastructure scan results",
            "source": "crew/mayor",
            "group_id": "aegis-ontology",
            "nodes": [
                {"name": "koror", "type": "ProxmoxNode", "description": "Proxmox host"},
                {"name": "ct-205", "type": "LXCContainer", "description": "Dolt container"}
            ],
            "edges": [
                {"source": "koror", "target": "ct-205", "relation": "runs"}
            ]
        }"#);

        let (tx_id, count) = ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z").unwrap();

        assert!(tx_id > 0);
        // Episode (4: type + label + comment + wasAssociatedWith + groupId = 5)
        // + koror (4: type + label + comment + wasGeneratedBy = 4)
        // + ct-205 (4: type + label + comment + wasGeneratedBy = 4)
        // + 1 edge = 14 total
        assert!(count >= 10, "expected at least 10 triples, got {count}");

        // Verify entities are in the store.
        let koror = store
            .lookup("http://aegis.gastown.local/ontology/koror")
            .unwrap();
        assert!(koror.is_some(), "koror entity should exist");

        let ct205 = store
            .lookup("http://aegis.gastown.local/ontology/ct-205")
            .unwrap();
        assert!(ct205.is_some(), "ct-205 entity should exist");

        // Verify the episode provenance entity.
        let ep_ent = store
            .lookup("http://aegis.gastown.local/ontology/episode_infra-scan")
            .unwrap();
        assert!(ep_ent.is_some(), "episode entity should exist");
    }

    #[test]
    fn node_properties_become_triples() {
        let mut store = Store::open_in_memory().unwrap();

        let ep = parse_episode(r#"{
            "name": "prop-test",
            "nodes": [
                {
                    "name": "svc1",
                    "type": "WebService",
                    "properties": {
                        "port": 8080,
                        "hostname": "svc1.svc",
                        "active": true
                    }
                }
            ],
            "edges": []
        }"#);

        let (_, count) = ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z").unwrap();

        // Episode (2: type + label) + node (type + label + wasGeneratedBy + 3 props = 6) = 8
        assert!(count >= 7, "expected at least 7 triples, got {count}");

        let port_id = store
            .lookup("http://aegis.gastown.local/ontology/port")
            .unwrap();
        assert!(port_id.is_some(), "port predicate should exist");
    }

    #[test]
    fn sanitize_iri_local_handles_special_chars() {
        assert_eq!(sanitize_iri_local("ct-205"), "ct-205");
        assert_eq!(sanitize_iri_local("hello world"), "hello_world");
        assert_eq!(sanitize_iri_local("a/b:c"), "a_b_c");
        assert_eq!(sanitize_iri_local("node.name"), "node.name");
    }

    #[test]
    fn escape_turtle_handles_quotes() {
        assert_eq!(escape_turtle(r#"say "hello""#), r#"say \"hello\""#);
        assert_eq!(escape_turtle("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn minimal_episode_with_body_only() {
        let mut store = Store::open_in_memory().unwrap();

        let ep = parse_episode(r#"{
            "name": "simple-note",
            "episode_body": "Koror was rebooted at 14:00 UTC"
        }"#);

        let (tx_id, count) = ingest_episode(&mut store, &ep, "2026-04-04T14:00:00Z").unwrap();
        assert!(tx_id > 0);
        // Just the episode entity: type + label + comment = 3
        assert_eq!(count, 3);
    }
}
