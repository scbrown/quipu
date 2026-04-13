//! Tests for episode ingestion.

use super::*;
use crate::namespace;

const TEST_BASE_NS: &str = namespace::DEFAULT_BASE_NS;

fn parse_episode(json: &str) -> Episode {
    serde_json::from_str(json).unwrap()
}

#[test]
fn deserialize_episode() {
    let ep = parse_episode(
        r#"{
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
    }"#,
    );

    assert_eq!(ep.name, "koror-discovery");
    assert_eq!(ep.nodes.len(), 2);
    assert_eq!(ep.edges.len(), 1);
    assert_eq!(ep.nodes[0].node_type.as_deref(), Some("ProxmoxNode"));
}

#[test]
fn episode_to_turtle_generates_valid_rdf() {
    let ep = parse_episode(
        r#"{
        "name": "test-episode",
        "episode_body": "Test body",
        "source": "unit-test",
        "nodes": [
            {"name": "alpha", "type": "ServiceType", "description": "Alpha service"}
        ],
        "edges": []
    }"#,
    );

    let ttl = episode_to_turtle(&ep, TEST_BASE_NS);

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

    let ep = parse_episode(
        r#"{
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
    }"#,
    );

    let (tx_id, count) =
        ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();

    assert!(tx_id > 0);
    // Episode (4: type + label + comment + wasAssociatedWith + groupId = 5)
    // + koror (4: type + label + comment + wasGeneratedBy = 4)
    // + ct-205 (4: type + label + comment + wasGeneratedBy = 4)
    // + 1 edge = 14 total
    assert!(count >= 10, "expected at least 10 triples, got {count}");

    // Verify entities are in the store.
    let koror = store.lookup(&format!("{TEST_BASE_NS}koror")).unwrap();
    assert!(koror.is_some(), "koror entity should exist");

    let ct205 = store.lookup(&format!("{TEST_BASE_NS}ct-205")).unwrap();
    assert!(ct205.is_some(), "ct-205 entity should exist");

    // Verify the episode provenance entity.
    let ep_ent = store
        .lookup(&format!("{TEST_BASE_NS}episode_infra-scan"))
        .unwrap();
    assert!(ep_ent.is_some(), "episode entity should exist");
}

#[test]
fn node_properties_become_triples() {
    let mut store = Store::open_in_memory().unwrap();

    let ep = parse_episode(
        r#"{
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
    }"#,
    );

    let (_, count) = ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();

    // Episode (2: type + label) + node (type + label + wasGeneratedBy + 3 props = 6) = 8
    assert!(count >= 7, "expected at least 7 triples, got {count}");

    let port_id = store.lookup(&format!("{TEST_BASE_NS}port")).unwrap();
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

    let ep = parse_episode(
        r#"{
        "name": "simple-note",
        "episode_body": "Koror was rebooted at 14:00 UTC"
    }"#,
    );

    let (tx_id, count) =
        ingest_episode(&mut store, &ep, "2026-04-04T14:00:00Z", TEST_BASE_NS).unwrap();
    assert!(tx_id > 0);
    // Just the episode entity: type + label + comment = 3
    assert_eq!(count, 3);
}

#[test]
#[cfg(feature = "shacl")]
fn shacl_validation_rejects_invalid_episode() {
    let mut store = Store::open_in_memory().unwrap();

    let shapes = concat!(
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
        "@prefix aegis: <http://aegis.gastown.local/ontology/> .\n",
        "aegis:WebServiceShape a sh:NodeShape ;\n",
        "    sh:targetClass aegis:WebService ;\n",
        "    sh:property [ sh:path aegis:port ; sh:minCount 1 ] .\n"
    );

    let ep = Episode {
        name: "bad-service".into(),
        episode_body: None,
        source: None,
        group_id: None,
        nodes: vec![Node {
            name: "broken-svc".into(),
            node_type: Some("WebService".into()),
            description: Some("Missing port".into()),
            properties: None,
        }],
        edges: vec![],
        shapes: Some(shapes.into()),
    };

    let err = ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap_err();
    match err {
        Error::ValidationFailed {
            violations,
            messages,
        } => {
            assert!(violations > 0);
            assert!(!messages.is_empty());
        }
        other => panic!("expected ValidationFailed, got: {other}"),
    }

    // Nothing should have been written.
    assert!(store.current_facts().unwrap().is_empty());
}

#[test]
fn shacl_validation_passes_valid_episode() {
    let mut store = Store::open_in_memory().unwrap();

    let shapes = concat!(
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
        "@prefix aegis: <http://aegis.gastown.local/ontology/> .\n",
        "aegis:WebServiceShape a sh:NodeShape ;\n",
        "    sh:targetClass aegis:WebService ;\n",
        "    sh:property [ sh:path aegis:port ; sh:minCount 1 ] .\n"
    );

    let mut props = serde_json::Map::new();
    props.insert("port".into(), serde_json::json!(8080));

    let ep = Episode {
        name: "good-service".into(),
        episode_body: None,
        source: None,
        group_id: None,
        nodes: vec![Node {
            name: "valid-svc".into(),
            node_type: Some("WebService".into()),
            description: None,
            properties: Some(props),
        }],
        edges: vec![],
        shapes: Some(shapes.into()),
    };

    let (tx_id, count) =
        ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();
    assert!(tx_id > 0);
    assert!(count > 0);
}

#[test]
fn batch_ingestion() {
    let mut store = Store::open_in_memory().unwrap();

    let episodes: Vec<Episode> = vec![
        parse_episode(
            r#"{"name": "batch-1", "nodes": [{"name": "a1", "type": "Thing"}], "edges": []}"#,
        ),
        parse_episode(
            r#"{"name": "batch-2", "nodes": [{"name": "b1", "type": "Thing"}], "edges": []}"#,
        ),
        parse_episode(
            r#"{"name": "batch-3", "nodes": [{"name": "c1", "type": "Thing"}], "edges": []}"#,
        ),
    ];
    let timestamps = vec![
        "2026-04-04T12:00:00Z",
        "2026-04-04T12:01:00Z",
        "2026-04-04T12:02:00Z",
    ];

    let results = ingest_batch(&mut store, &episodes, &timestamps, TEST_BASE_NS).unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].0 < results[1].0);
    assert!(results[1].0 < results[2].0);
}

#[test]
fn provenance_query() {
    let mut store = Store::open_in_memory().unwrap();

    let ep = parse_episode(
        r#"{
        "name": "prov-test",
        "nodes": [
            {"name": "server1", "type": "Host"},
            {"name": "server2", "type": "Host"}
        ],
        "edges": []
    }"#,
    );

    ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();

    let entities = episode_provenance(&store, "prov-test", TEST_BASE_NS).unwrap();
    let iris: Vec<&str> = entities.iter().map(|(iri, _)| iri.as_str()).collect();
    let expected_server1 = format!("{TEST_BASE_NS}server1");
    let expected_server2 = format!("{TEST_BASE_NS}server2");
    assert!(iris.contains(&expected_server1.as_str()));
    assert!(iris.contains(&expected_server2.as_str()));
}

#[test]
#[cfg(feature = "shacl")]
fn batch_stops_on_validation_failure() {
    let mut store = Store::open_in_memory().unwrap();

    let shapes = concat!(
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
        "@prefix aegis: <http://aegis.gastown.local/ontology/> .\n",
        "aegis:S a sh:NodeShape ;\n",
        "    sh:targetClass aegis:Thing ;\n",
        "    sh:property [ sh:path aegis:label ; sh:minCount 1 ] .\n"
    );

    let mut good_props = serde_json::Map::new();
    good_props.insert("label".into(), serde_json::json!("ok"));

    let episodes = vec![
        Episode {
            name: "ok-ep".into(),
            episode_body: None,
            source: None,
            group_id: None,
            nodes: vec![Node {
                name: "good".into(),
                node_type: Some("Thing".into()),
                description: None,
                properties: Some(good_props),
            }],
            edges: vec![],
            shapes: Some(shapes.into()),
        },
        Episode {
            name: "bad-ep".into(),
            episode_body: None,
            source: None,
            group_id: None,
            nodes: vec![Node {
                name: "bad".into(),
                node_type: Some("Thing".into()),
                description: None,
                properties: None,
            }],
            edges: vec![],
            shapes: Some(shapes.into()),
        },
    ];
    let timestamps = vec!["2026-04-04T12:00:00Z", "2026-04-04T12:01:00Z"];

    let err = ingest_batch(&mut store, &episodes, &timestamps, TEST_BASE_NS);
    assert!(err.is_err());

    // First episode should have been ingested before failure.
    let prov = episode_provenance(&store, "ok-ep", TEST_BASE_NS).unwrap();
    assert_eq!(prov.len(), 1);
}
