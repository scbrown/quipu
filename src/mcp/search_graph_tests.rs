use super::*;

/// 40 ROOT entities that are all MORE similar to the query than the one
/// entity living only in a named graph, so a filter applied after a top-N cut
/// would never see the named-graph entity (aegis-rcz5ib.10).
fn root_and_named_graph_store() -> (Store, Vec<f32>) {
    let mut store = Store::open_in_memory().unwrap();
    store.search_config_mut().named_graphs = true;
    let mut turtle = String::new();
    for i in 0..40 {
        turtle.push_str(&format!(
            "<http://example.org/root{i:02}> <http://www.w3.org/2000/01/rdf-schema#label> \"Root {i:02}\" .\n"
        ));
    }
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let graph = store.graph_create("urn:test:graph:knowledge").unwrap();
    crate::rdf::ingest_rdf_to_graph(
        &mut store,
        r#"<https://example.org/issues/5877> <http://www.w3.org/2000/01/rdf-schema#label> "beads#5877: Proposal: Memory Beads" ."#.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
        graph,
    )
    .unwrap();
    for i in 0..40 {
        let id = store
            .lookup(&format!("http://example.org/root{i:02}"))
            .unwrap()
            .unwrap();
        #[allow(clippy::cast_precision_loss)]
        let v = vec![1.0, 0.01 * i as f32, 0.0];
        store
            .embed_entity(id, &format!("Root {i:02}"), &v, "2026-04-04T00:00:00Z")
            .unwrap();
    }
    let named = store
        .lookup("https://example.org/issues/5877")
        .unwrap()
        .unwrap();
    store
        .embed_entity(
            named,
            "beads#5877: Proposal: Memory Beads",
            &[0.0, 0.0, 1.0],
            "2026-04-04T00:00:00Z",
        )
        .unwrap();
    (store, vec![1.0, 0.0, 0.1])
}

fn result_entities(result: &serde_json::Value) -> Vec<String> {
    result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["entity"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_tool_search_graph_reaches_a_named_graph_entity_behind_root_matches() {
    let (store, query) = root_and_named_graph_store();
    let result = tool_search(
        &store,
        &serde_json::json!({
            "embedding": query, "limit": 1, "graph": "urn:test:graph:knowledge", "verbose": true
        }),
    )
    .unwrap();
    assert_eq!(
        result_entities(&result),
        vec!["https://example.org/issues/5877".to_string()],
        "a graph scope must not be starved by 40 more-similar ROOT vectors"
    );
    assert_eq!(
        result["results"][0]["graph"], "urn:test:graph:knowledge",
        "a scoped result carries its graph IRI"
    );
}

#[test]
fn test_tool_search_without_graph_is_root_only_and_unchanged() {
    let (store, query) = root_and_named_graph_store();
    let result = tool_search(
        &store,
        &serde_json::json!({"embedding": query, "limit": 50, "verbose": true}),
    )
    .unwrap();
    let entities = result_entities(&result);
    assert_eq!(entities.len(), 40, "every ROOT entity, and nothing else");
    assert!(
        !entities.iter().any(|e| e.contains("5877")),
        "a named-graph-only entity must not leak into an unscoped (ROOT) search"
    );
    assert!(entities[0].ends_with("root00"), "ROOT ranking is unchanged");
    assert!(result["results"][0].get("graph").is_none());
}

#[test]
fn test_tool_search_refuses_an_unknown_graph() {
    let (store, query) = root_and_named_graph_store();
    for graph in [
        serde_json::json!("urn:test:graph:missing"),
        serde_json::json!(7),
    ] {
        let err = tool_search(
            &store,
            &serde_json::json!({"embedding": query, "limit": 5, "graph": graph}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("graph"), "{err}");
    }
}
