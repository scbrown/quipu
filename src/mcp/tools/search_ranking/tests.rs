use crate::vector::KnowledgeVectorStore;
use crate::{Store, tool_search};
use serde_json::json;

fn fixture(kind: &str, extra: &str) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let base = store.base_ns().to_string();
    let ttl = format!(
        "@prefix ex: <http://example.org/> .\n\
         @prefix model: <{base}> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         ex:artifact a model:{kind}; rdfs:label \"Hybrid Search\"; {extra} .\n\
         ex:rule a model:FailureMode; rdfs:label \"Wrong quantity\";\
         rdfs:comment \"An accurate number may be labelled with a different quantity.\" ."
    );
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    for (iri, vector) in [
        ("artifact", [0.49_f32, (1.0 - 0.49_f32.powi(2)).sqrt()]),
        ("rule", [0.36_f32, (1.0 - 0.36_f32.powi(2)).sqrt()]),
    ] {
        let id = store.intern(&format!("http://example.org/{iri}")).unwrap();
        store.embed_entity(id, iri, &vector, "2026-01-01").unwrap();
    }
    store
}

#[test]
fn contentless_artifacts_lose_to_explanatory_knowledge_before_limit() {
    for kind in ["Section", "Chunk", "CodeSymbol"] {
        let store = fixture(
            kind,
            "model:heading \"Hybrid Search\"; model:filePath \"src/search.rs\"; model:headingDepth 3",
        );
        let result = tool_search(
            &store,
            &json!({"embedding":[1,0],"ranking":"content","limit":1,"verbose":true}),
        )
        .unwrap();
        assert_eq!(
            result["results"][0]["entity"], "http://example.org/rule",
            "{kind}"
        );
        let raw = tool_search(
            &store,
            &json!({"embedding":[1,0],"ranking":"semantic","limit":1,"verbose":true}),
        )
        .unwrap();
        assert_eq!(raw["results"][0]["entity"], "http://example.org/artifact");
        assert_eq!(raw["results"][0]["score"], raw["results"][0]["similarity"]);
    }
}

#[test]
fn useful_artifacts_and_exact_name_queries_keep_their_rank() {
    for kind in ["Section", "Chunk", "CodeSymbol"] {
        for extra in [
            "rdfs:comment \"Combine vector similarity with structured filters\"",
            "model:content \"SELECT entities using structured filters before ranking\"@en",
        ] {
            let store = fixture(kind, extra);
            let result = tool_search(
                &store,
                &json!({"embedding":[1,0],"ranking":"content","limit":1,"verbose":true}),
            )
            .unwrap();
            assert_eq!(
                result["results"][0]["entity"], "http://example.org/artifact",
                "{kind}: {extra}"
            );
            assert_eq!(
                result["results"][0]["score"],
                result["results"][0]["similarity"]
            );
        }
        let store = fixture(kind, "rdfs:comment \"Hybrid Search\"");
        let result = tool_search(
            &store,
            &json!({"embedding":[1,0],"ranking":"content","query":" hybrid SEARCH ","limit":1,"verbose":true}),
        )
        .unwrap();
        assert_eq!(
            result["results"][0]["entity"],
            "http://example.org/artifact"
        );
    }
}

#[test]
fn historical_search_does_not_use_a_later_description() {
    let mut store = fixture("Section", "model:heading \"Hybrid Search\"");
    crate::rdf::ingest_rdf(&mut store,
        &b"<http://example.org/artifact> <http://www.w3.org/2000/01/rdf-schema#comment> \"A later explanatory body\" ."[..],
        oxrdfio::RdfFormat::Turtle, None, "2026-02-01", None, None).unwrap();
    let result = tool_search(
        &store,
        &json!({"embedding":[1,0],"ranking":"content","limit":1,"verbose":true,"valid_at":"2026-01-15"}),
    )
    .unwrap();
    assert_eq!(result["results"][0]["entity"], "http://example.org/rule");
    let current = tool_search(
        &store,
        &json!({"embedding":[1,0],"ranking":"content","limit":1,"verbose":true}),
    )
    .unwrap();
    assert_eq!(
        current["results"][0]["entity"],
        "http://example.org/artifact"
    );
}

#[test]
fn invalid_ranking_is_rejected() {
    let store = fixture("Section", "model:heading \"Hybrid Search\"");
    assert!(tool_search(&store, &json!({"embedding":[1,0],"ranking":"governed"})).is_err());
}

#[test]
fn general_search_does_not_demote_a_correct_bare_symbol() {
    let store = fixture("CodeSymbol", "model:name \"Hybrid Search\"");
    let result = tool_search(
        &store,
        &json!({"embedding":[1,0],
        "query":"which symbol implements hybrid searching", "limit":1,"verbose":true}),
    )
    .unwrap();
    assert_eq!(result["ranking"], "semantic");
    assert_eq!(
        result["results"][0]["entity"],
        "http://example.org/artifact"
    );
    assert_eq!(
        result["results"][0]["score"],
        result["results"][0]["similarity"]
    );
}

#[test]
fn content_in_a_named_graph_does_not_change_root_ranking() {
    let mut store = fixture("Section", "model:heading \"Hybrid Search\"");
    let entity = store
        .lookup("http://example.org/artifact")
        .unwrap()
        .unwrap();
    let comment = store
        .intern("http://www.w3.org/2000/01/rdf-schema#comment")
        .unwrap();
    let graph = store
        .overlay_create("http://example.org/private", 0)
        .unwrap();
    store
        .overlay_write(
            graph,
            crate::types::Op::Assert,
            entity,
            comment,
            crate::types::Value::Str("An explanation outside ROOT".into()),
            "2026-01-01",
        )
        .unwrap();
    assert_eq!(store.entity_facts_in_graph(entity, graph).unwrap().len(), 1);
    assert!(
        !store
            .entity_facts(entity)
            .unwrap()
            .iter()
            .any(|f| f.attribute == comment)
    );
    for valid_at in [None, Some("2026-01-15")] {
        let result = tool_search(
            &store,
            &json!({
                "embedding": [1,0], "ranking": "content", "limit": 1,
                "verbose": true, "valid_at": valid_at
            }),
        )
        .unwrap();
        assert_eq!(result["results"][0]["entity"], "http://example.org/rule");
    }
}

#[test]
fn negative_similarity_is_demoted_and_explained_without_changing_raw_score() {
    let store = fixture("Section", "model:heading \"Hybrid Search\"");
    let raw = tool_search(
        &store,
        &json!({
            "embedding": [-1,0], "ranking": "semantic", "verbose": true
        }),
    )
    .unwrap();
    let ranked = tool_search(
        &store,
        &json!({
            "embedding": [-1,0], "ranking": "content", "verbose": true
        }),
    )
    .unwrap();
    let artifact = |result: &serde_json::Value| {
        result["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["entity"] == "http://example.org/artifact")
            .unwrap()
            .clone()
    };
    let original = artifact(&raw);
    let demoted = artifact(&ranked);
    assert_eq!(demoted["ranking_reason"], "contentless_artifact");
    assert_eq!(demoted["similarity"], original["score"]);
    assert!(demoted["score"].as_f64().unwrap() < original["score"].as_f64().unwrap());
}
