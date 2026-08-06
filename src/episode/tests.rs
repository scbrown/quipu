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

    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));

    // Should contain prefixes.
    assert!(ttl.contains("@prefix aegis:"));
    assert!(ttl.contains("@prefix prov:"));

    // Should contain episode entity.
    assert!(ttl.contains("aegis:episode_test-episode a prov:Activity"));
    assert!(ttl.contains("rdfs:label \"test-episode\""));
    assert!(ttl.contains("rdfs:comment \"Test body\""));
    // Should carry the idempotency key (hq-fhc).
    assert!(ttl.contains("aegis:contentHash"));

    // Should contain node.
    assert!(ttl.contains("aegis:alpha a aegis:ServiceType"));
    assert!(ttl.contains("prov:wasGeneratedBy aegis:episode_test-episode"));
}

#[test]
#[cfg(feature = "shacl")]
fn write_validation_enforces_loaded_shapes() {
    // hq-c6s: persistently-loaded shapes must gate episode writes when
    // validate_on_write is set — not just episode-inline shapes.
    let mut store = Store::open_in_memory().unwrap();
    let shape = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix aegis: <http://aegis.gastown.local/ontology/> .
aegis:ServerShape a sh:NodeShape ;
    sh:targetClass aegis:Server ;
    sh:property [ sh:path aegis:hostname ; sh:minCount 1 ] .
"#;
    store
        .load_shapes("server-shape", shape, "2026-01-01")
        .unwrap();

    // A Server node with no aegis:hostname → violates the loaded shape.
    let ep = parse_episode(
        r#"{ "name": "ep", "nodes": [{"name": "Server1", "type": "Server"}], "edges": [] }"#,
    );

    // Toggle OFF: the violating write is accepted — the pre-fix gap.
    ingest_episode(&mut store, &ep, "2026-01-02T00:00:00Z", TEST_BASE_NS).unwrap();

    // Toggle ON: the same write is now rejected against the loaded shape.
    store.shacl_config_mut().validate_on_write = true;
    let err = ingest_episode(&mut store, &ep, "2026-01-03T00:00:00Z", TEST_BASE_NS).unwrap_err();
    assert!(
        matches!(err, crate::error::Error::ValidationFailed { .. }),
        "expected ValidationFailed, got: {err:?}"
    );
}

#[test]
fn resolution_fires_on_duplicate_node() {
    // hq-uye: with resolution enabled, ingesting an episode whose node matches
    // an existing entity (by rdfs:label) must surface a dedup hint — the engine
    // was previously dead code because nothing threaded the config through.
    let mut store = Store::open_in_memory().unwrap();

    let first = parse_episode(
        r#"{ "name": "ep-1", "nodes": [
            {"name": "Tapestry", "type": "WebApplication", "description": "Web UI"}
        ], "edges": [] }"#,
    );
    ingest_episode(&mut store, &first, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();

    let dup = parse_episode(
        r#"{ "name": "ep-2", "nodes": [
            {"name": "Tapestry", "type": "WebApplication", "description": "Web UI"}
        ], "edges": [] }"#,
    );

    // Resolution enabled → hint for the duplicate node.
    let enabled = IngestResolutionOpts {
        enabled: true,
        threshold: 0.85,
        top_k: 3,
        strict_mode: false,
    };
    let result = ingest_episode_with_resolution(
        &mut store,
        &dup,
        "2026-04-05T12:00:00Z",
        TEST_BASE_NS,
        Some(&enabled),
    )
    .unwrap();
    assert!(
        !result.resolution_hints.is_empty(),
        "expected a dedup hint for the duplicate 'Tapestry' node"
    );
    assert_eq!(result.resolution_hints[0].0, "Tapestry");

    // Control: resolution disabled → no hints (the pre-fix behaviour).
    let disabled = IngestResolutionOpts {
        enabled: false,
        ..enabled.clone()
    };
    let plain = ingest_episode_with_resolution(
        &mut store,
        &dup,
        "2026-04-06T12:00:00Z",
        TEST_BASE_NS,
        Some(&disabled),
    )
    .unwrap();
    assert!(
        plain.resolution_hints.is_empty(),
        "disabled resolution must not emit hints"
    );
}

#[test]
fn resolution_opts_from_config_round_trip() {
    // The config→opts bridge that the handlers use must carry every field.
    let cfg = crate::config::ResolutionConfig {
        enabled: true,
        threshold: 0.7,
        top_k: 5,
        strict_mode: true,
    };
    let opts = IngestResolutionOpts::from_config(&cfg);
    assert!(opts.enabled);
    assert_eq!(opts.threshold, 0.7);
    assert_eq!(opts.top_k, 5);
    assert!(opts.strict_mode);
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
                    "hostname": "svc1.example",
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
fn array_valued_node_property_yields_one_triple_per_element() {
    // Regression guard: a JSON array in node properties was silently DROPPED
    // (no-op final match arm) while returning success — turning a multi-valued
    // trait into a silently incomplete role. It must now emit one triple per
    // element, matching what the Turtle path does for `a "x", "y" .`.
    let ep = parse_episode(
        r#"{
        "name": "arr-test",
        "nodes": [
            {
                "name": "worker",
                "type": "CrewRole",
                "properties": {
                    "traitScope": "domain-scoped",
                    "traitWorkIntake": ["self-directed", "escalations-only"]
                }
            }
        ],
        "edges": []
    }"#,
    );

    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));
    // both array elements present as their own object term (multi-value preserved)
    assert!(
        ttl.contains("aegis:traitWorkIntake \"self-directed\""),
        "first array element missing:\n{ttl}"
    );
    assert!(
        ttl.contains("aegis:traitWorkIntake \"escalations-only\""),
        "second array element missing (the exact z5mw3 silent-drop):\n{ttl}"
    );
    // the scalar sibling on the same node still survives unchanged
    assert!(
        ttl.contains("aegis:traitScope \"domain-scoped\""),
        "scalar property regressed:\n{ttl}"
    );
    // exactly two triples for the multi-valued predicate, not a joined blob
    assert_eq!(
        ttl.matches("aegis:traitWorkIntake ").count(),
        2,
        "expected 2 traitWorkIntake triples:\n{ttl}"
    );
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
    // Just the episode entity: type + label + comment + contentHash = 4
    assert_eq!(count, 4);
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
        graph: None,
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
        graph: None,
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
            graph: None,
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
            graph: None,
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

// ── Idempotent ingest (hq-fhc) ──────────────────────────────────────

/// Count the active object values of `subject pred ?v` in current state.
fn active_values(store: &Store, subject_iri: &str, pred_iri: &str) -> Vec<String> {
    let q = format!("SELECT ?v WHERE {{ <{subject_iri}> <{pred_iri}> ?v }}");
    crate::sparql::query(store, &q)
        .unwrap()
        .rows()
        .iter()
        .filter_map(|r| match r.get("v") {
            Some(crate::types::Value::Str(s)) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn reingest_identical_episode_is_noop() {
    let mut store = Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{
        "name": "dedup-test",
        "episode_body": "first body",
        "source": "crew/mayor",
        "nodes": [{"name": "alpha", "type": "Service", "description": "Alpha"}],
        "edges": []
    }"#,
    );

    let (tx1, count1) =
        ingest_episode(&mut store, &ep, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();
    assert!(tx1 > 0, "first ingest writes a real transaction");
    assert!(count1 > 0, "first ingest asserts triples");

    // Re-ingesting byte-identical content is a no-op.
    let (tx2, count2) =
        ingest_episode(&mut store, &ep, "2026-04-04T13:00:00Z", TEST_BASE_NS).unwrap();
    assert_eq!(
        tx2, NOOP_TX,
        "identical re-ingest returns the no-op sentinel"
    );
    assert_eq!(count2, 0, "identical re-ingest writes nothing");

    // The activity still has exactly one active label and one content hash —
    // no duplicate provenance accumulated.
    let ep_iri = format!("{TEST_BASE_NS}episode_dedup-test");
    let label = format!("{}label", namespace::RDFS);
    let chash = format!("{TEST_BASE_NS}contentHash");
    assert_eq!(active_values(&store, &ep_iri, &label).len(), 1);
    assert_eq!(active_values(&store, &ep_iri, &chash).len(), 1);
}

#[test]
fn reingest_changed_episode_updates_without_duplicating() {
    let mut store = Store::open_in_memory().unwrap();
    let v1 = parse_episode(
        r#"{
        "name": "dedup-test",
        "episode_body": "first body",
        "source": "crew/mayor",
        "nodes": [],
        "edges": []
    }"#,
    );
    ingest_episode(&mut store, &v1, "2026-04-04T12:00:00Z", TEST_BASE_NS).unwrap();

    let v2 = parse_episode(
        r#"{
        "name": "dedup-test",
        "episode_body": "second body",
        "source": "crew/mayor",
        "nodes": [],
        "edges": []
    }"#,
    );
    let (tx2, count2) =
        ingest_episode(&mut store, &v2, "2026-04-04T13:00:00Z", TEST_BASE_NS).unwrap();
    assert!(tx2 > 0, "changed content writes a real transaction");
    assert!(count2 > 0);

    // Exactly one active comment, and it is the updated value — the stale
    // "first body" was retracted, not left alongside the new one.
    let ep_iri = format!("{TEST_BASE_NS}episode_dedup-test");
    let comment = format!("{}comment", namespace::RDFS);
    let comments = active_values(&store, &ep_iri, &comment);
    assert_eq!(comments, vec!["second body".to_string()]);

    // And exactly one active content hash.
    let chash = format!("{TEST_BASE_NS}contentHash");
    assert_eq!(active_values(&store, &ep_iri, &chash).len(), 1);
}

#[test]
fn content_hash_is_order_independent_for_nodes() {
    let a =
        parse_episode(r#"{ "name": "h", "nodes": [{"name": "x"}, {"name": "y"}], "edges": [] }"#);
    let b =
        parse_episode(r#"{ "name": "h", "nodes": [{"name": "y"}, {"name": "x"}], "edges": [] }"#);
    assert_eq!(episode_content_hash(&a), episode_content_hash(&b));
}

// ── Edge confidence qualifier (hq-cug6, aegis-1p0 Gap 5) ──────────────────

/// An edge without a confidence field stays a bare triple — no reification,
/// fully back-compatible with pre-hq-cug6 episodes.
#[test]
fn edge_without_confidence_is_a_bare_triple() {
    let ep = parse_episode(
        r#"{ "name": "ep", "edges": [{"source": "a", "target": "b", "relation": "knows"}] }"#,
    );
    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));
    assert!(ttl.contains("aegis:a aegis:knows aegis:b ."));
    assert!(
        !ttl.contains("rdf:Statement"),
        "no reification without a confidence field"
    );
    assert!(!ttl.contains("quipu:confidence"));
}

/// A confidence enum grade reifies the statement and the qualifier is queryable
/// via SPARQL (the AC). The bare triple is still asserted alongside it.
#[test]
fn edge_confidence_enum_persists_and_is_sparql_queryable() {
    let mut store = Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{
        "name": "conf-test",
        "edges": [
            {"source": "svcA", "target": "svcB", "relation": "dependsOn", "confidence": "INFERRED"}
        ]
    }"#,
    );
    ingest_episode(&mut store, &ep, "2026-06-29T00:00:00Z", TEST_BASE_NS).unwrap();

    // Find the confidence grade by matching the reified statement on its triple.
    let q = format!(
        "SELECT ?c WHERE {{ \
           ?s <{rdf}subject> <{ns}svcA> ; \
              <{rdf}predicate> <{ns}dependsOn> ; \
              <{rdf}object> <{ns}svcB> ; \
              <{quipu}confidence> ?c }}",
        rdf = namespace::RDF,
        ns = TEST_BASE_NS,
        quipu = namespace::QUIPU,
    );
    let rows = crate::sparql::query(&store, &q).unwrap();
    let vals: Vec<String> = rows
        .rows()
        .iter()
        .filter_map(|r| match r.get("c") {
            Some(crate::types::Value::Str(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        vals,
        vec!["INFERRED".to_string()],
        "confidence is queryable"
    );

    // The plain triple still exists (the qualifier is additive, not a
    // replacement). The reification uses rdf:subject/predicate/object, so only
    // the bare assertion matches `svcA dependsOn ?o`.
    let bare = crate::sparql::query(
        &store,
        &format!("SELECT ?o WHERE {{ <{TEST_BASE_NS}svcA> <{TEST_BASE_NS}dependsOn> ?o }}"),
    )
    .unwrap();
    assert_eq!(
        bare.rows().len(),
        1,
        "bare triple asserted alongside the reified qualifier"
    );
}

/// A numeric 0–1 confidence is emitted as an xsd:decimal literal.
#[test]
fn edge_confidence_numeric_emits_decimal() {
    let ep = parse_episode(
        r#"{ "name": "n", "edges": [{"source": "a", "target": "b", "relation": "rel", "confidence": 0.75}] }"#,
    );
    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));
    assert!(ttl.contains("a rdf:Statement"));
    assert!(
        ttl.contains("quipu:confidence \"0.75\"^^xsd:decimal"),
        "numeric confidence becomes a typed decimal literal; got:\n{ttl}"
    );
}

/// Changing only an edge's confidence changes the content hash, so a re-ingest
/// is not mistaken for a no-op.
#[test]
fn confidence_participates_in_content_hash() {
    let a = parse_episode(
        r#"{ "name": "h", "edges": [{"source": "a", "target": "b", "relation": "r", "confidence": "EXTRACTED"}] }"#,
    );
    let b = parse_episode(
        r#"{ "name": "h", "edges": [{"source": "a", "target": "b", "relation": "r", "confidence": "AMBIGUOUS"}] }"#,
    );
    assert_ne!(episode_content_hash(&a), episode_content_hash(&b));
}

#[test]
fn untyped_node_is_rejected_with_a_clear_error_not_a_turtle_400() {
    // aegis-uqd8: an untyped node used to emit malformed Turtle ("aegis:foo ;
    // rdfs:label …") and 400 the WHOLE episode with a cryptic parse error,
    // discarding every well-formed node beside it. It must now fail loud and
    // specific, naming the offending node — and NOT ingest anything.
    let mut store = Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{
        "name": "uqd8-untyped",
        "source": "test",
        "nodes": [
            {"name": "well-typed-node", "type": "DatabaseService"},
            {"name": "the-untyped-one"}
        ]
    }"#,
    );
    let err = ingest_episode(&mut store, &ep, "2026-01-02T00:00:00Z", TEST_BASE_NS)
        .expect_err("an untyped node must be rejected");
    let msg = err.to_string();
    // Diagnosable: names the node and the cause, not a raw Turtle parse error.
    assert!(
        msg.contains("the-untyped-one"),
        "error must name the untyped node: {msg}"
    );
    assert!(
        msg.contains("type"),
        "error must explain it is a type problem: {msg}"
    );
    assert!(
        !msg.to_lowercase().contains("parse"),
        "must NOT be a cryptic Turtle parse error: {msg}"
    );
}

#[test]
fn episode_graph_field_writes_to_named_graph_not_root() {
    // aegis-g1al / #36 write API: an episode with a `graph` field lands its
    // facts in that named graph (g = the graph IRI's term id); an episode
    // without one writes ROOT (g=0). Verified by reading the g column back.
    use crate::store::Store;
    let mut store = Store::open_in_memory().unwrap();

    // Episode into an overlay graph.
    let ov = parse_episode(
        r#"{ "name": "ov-ep", "source": "t",
             "graph": "http://example.org/graph/tenant-1",
             "nodes": [{"name": "svc-x", "type": "DatabaseService"}] }"#,
    );
    ingest_episode(&mut store, &ov, "2026-01-01T00:00:00Z", TEST_BASE_NS).unwrap();

    // Episode into ROOT (no graph field).
    let root = parse_episode(
        r#"{ "name": "root-ep", "source": "t",
             "nodes": [{"name": "svc-y", "type": "DatabaseService"}] }"#,
    );
    ingest_episode(&mut store, &root, "2026-01-02T00:00:00Z", TEST_BASE_NS).unwrap();

    let overlay_g = store.intern("http://example.org/graph/tenant-1").unwrap();
    // svc-x's facts must be in the overlay graph, none in ROOT.
    let svc_x = store.intern(&format!("{TEST_BASE_NS}svc-x")).unwrap();
    let in_overlay: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND g=?2",
            [svc_x, overlay_g],
            |r| r.get(0),
        )
        .unwrap();
    let in_root: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND g=0",
            [svc_x],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        in_overlay > 0,
        "overlay episode's facts must land in the named graph"
    );
    assert_eq!(
        in_root, 0,
        "overlay episode must write NOTHING to ROOT (base un-mutated)"
    );

    // svc-y (no graph field) must be in ROOT.
    let svc_y = store.intern(&format!("{TEST_BASE_NS}svc-y")).unwrap();
    let y_in_root: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND g=0",
            [svc_y],
            |r| r.get(0),
        )
        .unwrap();
    assert!(y_in_root > 0, "a no-graph episode must write ROOT (g=0)");
}

#[test]
fn prefixed_edge_relation_is_emitted_verbatim_not_forced_into_aegis() {
    // Regression guard for aegis-kuotp. /episode used to force EVERY relation
    // into aegis: and then sanitize it, so `rdfs:subClassOf` was stored as
    // `aegis:rdfs_subClassOf` — a predicate that resembles the intended one,
    // matches nothing, and is inert — behind HTTP 200 with a healthy count.
    let ep = parse_episode(
        r#"{
        "name": "prefix-test",
        "nodes": [
            {"name": "child", "type": "FailureMode"},
            {"name": "parent", "type": "FailureMode"}
        ],
        "edges": [
            {"source": "child", "target": "parent", "relation": "rdfs:subClassOf"},
            {"source": "child", "target": "parent", "relation": "owl:sameAs"},
            {"source": "child", "target": "parent", "relation": "related_to"}
        ]
    }"#,
    );

    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));

    assert!(
        ttl.contains("aegis:child rdfs:subClassOf aegis:parent"),
        "prefixed relation must be emitted verbatim:\n{ttl}"
    );
    assert!(
        ttl.contains("aegis:child owl:sameAs aegis:parent"),
        "owl: relation must be emitted verbatim (a real instance existed in the \
         live graph as the inert aegis:owl_sameAs):\n{ttl}"
    );
    // The exact mangled forms that were the defect.
    assert!(
        !ttl.contains("aegis:rdfs_subClassOf"),
        "the mangled predicate must be gone:\n{ttl}"
    );
    assert!(
        !ttl.contains("aegis:owl_sameAs"),
        "the mangled predicate must be gone:\n{ttl}"
    );
    // A bare relation still lands in the aegis: domain vocabulary, unchanged.
    assert!(
        ttl.contains("aegis:child aegis:related_to aegis:parent"),
        "bare relations must keep working:\n{ttl}"
    );
    // Every prefix the resolver accepts must actually be declared, or the
    // emitted Turtle does not parse.
    assert!(ttl.contains("@prefix owl:"), "owl: undeclared:\n{ttl}");
    assert!(ttl.contains("@prefix skos:"), "skos: undeclared:\n{ttl}");
    assert!(ttl.contains("@prefix sh:"), "sh: undeclared:\n{ttl}");
}

#[test]
fn every_known_prefix_resolves_and_is_declared() {
    // The test above names three prefixes by hand out of the eight in
    // KNOWN_PREFIXES, so a ninth entry added without a matching `@prefix` line
    // would pass it and emit Turtle that does not parse. The two lists are only
    // kept in lockstep by a comment; this asserts the invariant instead.
    // Found while confirming owl:sameAs was reachable from /episode at all.
    let edges: String = KNOWN_PREFIXES
        .iter()
        .map(|(p, _)| format!(r#"{{"source":"a","target":"b","relation":"{p}:probe"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let ep = parse_episode(&format!(
        r#"{{"name":"prefix-lockstep",
             "nodes":[{{"name":"a","type":"Thing"}},{{"name":"b","type":"Thing"}}],
             "edges":[{edges}]}}"#
    ));

    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));

    for (prefix, _ns) in KNOWN_PREFIXES {
        // Resolves rather than being refused or mangled into aegis:.
        assert_eq!(
            resolve_edge_predicate(&format!("{prefix}:probe")).unwrap(),
            format!("{prefix}:probe"),
            "{prefix}: must resolve to itself"
        );
        assert!(
            ttl.contains(&format!("aegis:a {prefix}:probe aegis:b")),
            "{prefix}: relation not emitted verbatim:\n{ttl}"
        );
        // ...and the prefix it emits is actually declared in the same document.
        assert!(
            ttl.contains(&format!("@prefix {prefix}:")),
            "KNOWN_PREFIXES has '{prefix}' but episode_to_turtle declares no \
             `@prefix {prefix}:` — the emitted Turtle would not parse:\n{ttl}"
        );
        assert!(
            !ttl.contains(&format!("aegis:{prefix}_probe")),
            "{prefix}: was mangled into the aegis: namespace:\n{ttl}"
        );
    }
}

#[test]
fn full_iri_edge_relation_is_emitted_verbatim() {
    let ep = parse_episode(
        r#"{
        "name": "iri-test",
        "nodes": [{"name": "a", "type": "Thing"}, {"name": "b", "type": "Thing"}],
        "edges": [
            {"source": "a", "target": "b", "relation": "<http://example.org/ns#custom>"}
        ]
    }"#,
    );
    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));
    assert!(
        ttl.contains("aegis:a <http://example.org/ns#custom> aegis:b"),
        "full-IRI relation must be emitted verbatim:\n{ttl}"
    );
}

#[test]
fn unrepresentable_edge_relation_is_refused_not_silently_rewritten() {
    // The whole point of aegis-kuotp: a predicate we cannot represent faithfully
    // must produce an ERROR that names the right path, never a 200 plus a dead
    // triple. Each case below is one that used to return success.
    let mut store = crate::store::Store::open_in_memory().unwrap();

    // (a) undeclared prefix -> refused, and the error must name /set.
    let ep = parse_episode(
        r#"{
        "name": "bad-prefix",
        "nodes": [{"name": "a", "type": "Thing"}, {"name": "b", "type": "Thing"}],
        "edges": [{"source": "a", "target": "b", "relation": "skynet:controls"}]
    }"#,
    );
    let err = ingest_episode(&mut store, &ep, "2026-08-04T00:00:00Z", TEST_BASE_NS)
        .expect_err("an undeclared prefix must be refused, not stored mangled");
    let msg = err.to_string();
    assert!(
        msg.contains("/set"),
        "the error must name the write path that DOES take a foreign IRI: {msg}"
    );
    assert!(
        msg.contains("skynet"),
        "the error must name the offending prefix: {msg}"
    );

    // (b) a relation that would be silently rewritten -> refused.
    let ep = parse_episode(
        r#"{
        "name": "bad-chars",
        "nodes": [{"name": "a", "type": "Thing"}, {"name": "b", "type": "Thing"}],
        "edges": [{"source": "a", "target": "b", "relation": "runs on"}]
    }"#,
    );
    let err = ingest_episode(&mut store, &ep, "2026-08-04T00:00:00Z", TEST_BASE_NS)
        .expect_err("a relation that cannot round-trip must be refused");
    assert!(
        err.to_string().contains("runs_on"),
        "the error must teach the working form: {err}"
    );

    // (c) NOTHING from a refused episode may land — the gate runs before the
    // write, so the well-formed nodes beside the bad edge must not be stored.
    assert!(
        store.lookup(&format!("{TEST_BASE_NS}a")).unwrap().is_none(),
        "a refused episode must write nothing at all"
    );
}

#[test]
fn comma_separated_type_is_refused_not_minted_as_a_junk_class() {
    // Regression guard for aegis-vngta. `{"type": "Feature, Concept"}` used to mint
    // ONE class `aegis:Feature__Concept` behind HTTP 200 — the node present, correctly
    // described and edged, and absent from `?s a Feature`, the query anyone runs.
    let mut store = crate::store::Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{
        "name": "comma-type",
        "nodes": [{"name": "governor-burndown", "type": "Feature, Concept"}],
        "edges": []
    }"#,
    );
    let err = ingest_episode(&mut store, &ep, "2026-08-04T00:00:00Z", TEST_BASE_NS)
        .expect_err("a comma-separated type must be refused, not minted as one class");
    let msg = err.to_string();
    assert!(
        msg.contains("Feature__Concept"),
        "the error must show the junk class that WOULD have been minted: {msg}"
    );
    assert!(
        msg.contains("ONE ENTRY PER TYPE"),
        "the error must teach the working form: {msg}"
    );
    assert!(
        msg.contains("governor-burndown"),
        "the error must name the offending node: {msg}"
    );
    // Nothing from a refused episode may land.
    assert!(
        store
            .lookup(&format!("{TEST_BASE_NS}governor-burndown"))
            .unwrap()
            .is_none(),
        "a refused episode must write nothing at all"
    );
}

#[test]
fn one_entry_per_type_still_yields_one_entity_with_both_types() {
    // The CONVERSE, and it is the half that makes the guard a policy rather than a
    // filter: the documented working form must keep working, or the refusal above
    // just breaks multi-typing instead of fixing it.
    let mut store = crate::store::Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{
        "name": "multi-type-ok",
        "nodes": [
            {"name": "governor-burndown", "type": "Feature"},
            {"name": "governor-burndown", "type": "Concept"}
        ],
        "edges": []
    }"#,
    );
    ingest_episode(&mut store, &ep, "2026-08-04T00:00:00Z", TEST_BASE_NS)
        .expect("one-entry-per-type is the documented working form and must be accepted");

    let ttl = episode_to_turtle(&ep, TEST_BASE_NS, &episode_content_hash(&ep));
    assert!(
        ttl.contains("aegis:governor-burndown a aegis:Feature"),
        "first type missing:\n{ttl}"
    );
    assert!(
        ttl.contains("aegis:governor-burndown a aegis:Concept"),
        "second type missing:\n{ttl}"
    );
    assert!(
        !ttl.contains("Feature__Concept"),
        "no junk class may appear:\n{ttl}"
    );
}

#[test]
fn type_that_would_be_silently_rewritten_is_refused() {
    let mut store = crate::store::Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{
        "name": "bad-type-chars",
        "nodes": [{"name": "thing", "type": "Web Service"}],
        "edges": []
    }"#,
    );
    let err = ingest_episode(&mut store, &ep, "2026-08-04T00:00:00Z", TEST_BASE_NS)
        .expect_err("a type that cannot round-trip must be refused");
    assert!(
        err.to_string().contains("Web_Service"),
        "the error must show what it would have become: {err}"
    );
}

/// A retry after a lost response must be reported as SUCCESS, not as a write
/// that achieved nothing.
///
/// `/episode` has been idempotent since hq-fhc, so retrying is safe. The defect
/// was that it did not SAY so: the no-op returned `count: 0, tx_id: 0`, which is
/// exactly what a failed write returns, while every caller's documented success
/// check is "HTTP 200 with count > 0". The safe mechanism reported itself as a
/// failure, and the natural recovery from "it didn't land" is to re-post under a
/// different name — the entity fragmentation this store already has beads about.
#[test]
fn an_identical_repost_reports_unchanged_rather_than_an_empty_write() {
    let mut store = Store::open_in_memory().unwrap();
    let ep = parse_episode(
        r#"{"name": "retry-probe", "episode_body": "b", "source": "s",
            "nodes": [{"name": "alpha", "type": "Probe", "description": "d"}],
            "edges": []}"#,
    );

    let first =
        ingest_episode_outcome(&mut store, &ep, "2026-01-01T00:00:00Z", TEST_BASE_NS).unwrap();
    assert_eq!(first.2, IngestOutcome::Created);
    assert!(first.1 > 0, "the first ingest must actually write");

    // The retry: byte-identical payload, as a caller re-sending after a timeout.
    let retry =
        ingest_episode_outcome(&mut store, &ep, "2026-01-02T00:00:00Z", TEST_BASE_NS).unwrap();
    assert_eq!(
        retry.2,
        IngestOutcome::Unchanged,
        "an identical re-post must report `unchanged`, so a caller can tell it \
         from a write that did nothing"
    );
    assert_eq!(retry.1, 0, "and it must still write nothing");
    assert_eq!(retry.0, NOOP_TX);

    // The facts are present exactly ONCE — idempotency is real, not just reported.
    let alpha = store
        .lookup(&format!("{TEST_BASE_NS}alpha"))
        .unwrap()
        .expect("alpha exists");
    let comments = store
        .entity_facts(alpha)
        .unwrap()
        .into_iter()
        .filter(|f| store.resolve(f.attribute).unwrap().ends_with("comment"))
        .count();
    assert_eq!(comments, 1, "the retry duplicated a comment");
}

/// Changed content on an existing episode is `updated`, not `created` — so a
/// caller can tell a genuine revision from a first write, which is the other
/// half of what `count` alone cannot express.
#[test]
fn a_changed_repost_reports_updated() {
    let mut store = Store::open_in_memory().unwrap();
    let first = parse_episode(
        r#"{"name": "rev", "episode_body": "one",
            "nodes": [{"name": "n", "type": "Probe"}], "edges": []}"#,
    );
    let second = parse_episode(
        r#"{"name": "rev", "episode_body": "two",
            "nodes": [{"name": "n", "type": "Probe"}], "edges": []}"#,
    );
    let a =
        ingest_episode_outcome(&mut store, &first, "2026-01-01T00:00:00Z", TEST_BASE_NS).unwrap();
    let b =
        ingest_episode_outcome(&mut store, &second, "2026-01-02T00:00:00Z", TEST_BASE_NS).unwrap();
    assert_eq!(a.2, IngestOutcome::Created);
    assert_eq!(b.2, IngestOutcome::Updated);
    assert!(b.1 > 0, "a changed episode must write");
}

/// The three outcomes must have distinct wire strings, or a caller branching on
/// the field is back where it started.
#[test]
fn outcome_wire_strings_are_distinct() {
    let all = [
        IngestOutcome::Created.as_str(),
        IngestOutcome::Updated.as_str(),
        IngestOutcome::Unchanged.as_str(),
    ];
    let uniq: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(uniq.len(), 3, "outcome strings collide: {all:?}");
}
