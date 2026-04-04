use super::*;
use crate::rdf::ingest_rdf;
use oxrdfio::RdfFormat;

fn setup_test_store() -> Store {
    let store = Store::open_in_memory().unwrap();

    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

ex:traefik rdf:type ex:WebApplication ;
    rdfs:label "Traefik" ;
    ex:runsOn ex:kota ;
    ex:port "443" .

ex:kota rdf:type ex:ProxmoxNode ;
    rdfs:label "Kota" ;
    ex:hostname "kota.lan" ;
    ex:manages ex:traefik .

ex:forgejo rdf:type ex:WebApplication ;
    rdfs:label "Forgejo" ;
    ex:runsOn ex:koror ;
    ex:port "3000" .

ex:koror rdf:type ex:ProxmoxNode ;
    rdfs:label "Koror" ;
    ex:hostname "koror.lan" .
    "#;

    let mut store = store;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        Some("test.ttl"),
    )
    .unwrap();
    store
}

#[test]
fn direct_text_search() {
    let store = setup_test_store();
    let pipeline = ContextPipeline::new(
        &store,
        ContextPipelineConfig {
            expand_links: false,
            ..Default::default()
        },
    );

    let ctx = pipeline.query("traefik").unwrap();
    assert!(!ctx.entities.is_empty());
    assert!(ctx.entities.iter().any(|e| e.iri.contains("traefik")));
    assert_eq!(ctx.entities[0].relevance, KnowledgeRelevance::Direct);
}

#[test]
fn link_expansion() {
    let store = setup_test_store();
    let pipeline = ContextPipeline::new(
        &store,
        ContextPipelineConfig {
            expand_links: true,
            ..Default::default()
        },
    );

    let ctx = pipeline.query("traefik").unwrap();
    // Should find traefik (direct) and kota (linked via runsOn/manages).
    assert!(ctx.entities.len() >= 2);
    assert!(ctx.entities.iter().any(|e| e.iri.contains("traefik")));
    assert!(ctx.entities.iter().any(|e| e.iri.contains("kota")));
    assert!(ctx.summary.linked_additions > 0);
}

#[test]
fn entity_has_label_and_types() {
    let store = setup_test_store();
    let pipeline = ContextPipeline::new(
        &store,
        ContextPipelineConfig {
            expand_links: false,
            ..Default::default()
        },
    );

    let ctx = pipeline.query("traefik").unwrap();
    let traefik = ctx
        .entities
        .iter()
        .find(|e| e.iri.contains("traefik"))
        .unwrap();
    assert_eq!(traefik.label, Some("Traefik".to_string()));
    assert!(traefik.types.iter().any(|t| t.contains("WebApplication")));
}

#[test]
fn max_entities_respected() {
    let store = setup_test_store();
    let pipeline = ContextPipeline::new(
        &store,
        ContextPipelineConfig {
            max_entities: 1,
            expand_links: false,
            ..Default::default()
        },
    );

    let ctx = pipeline.query("kota").unwrap();
    assert!(ctx.entities.len() <= 1);
}

#[test]
fn tool_context_handler() {
    let store = setup_test_store();
    let input = serde_json::json!({
        "query": "traefik",
        "expand_links": true,
    });

    let result = tool_context(&store, &input).unwrap();
    assert!(!result["entities"].as_array().unwrap().is_empty());
    assert!(result["summary"]["direct_hits"].as_u64().unwrap() >= 1);
}

#[test]
fn no_match_returns_empty() {
    let store = Store::open_in_memory().unwrap();
    let pipeline = ContextPipeline::new(&store, ContextPipelineConfig::default());

    let ctx = pipeline.query("anything").unwrap();
    assert_eq!(ctx.entities.len(), 0);
    assert_eq!(ctx.summary.direct_hits, 0);
}

#[test]
fn facts_have_correct_types() {
    let store = setup_test_store();
    let pipeline = ContextPipeline::new(
        &store,
        ContextPipelineConfig {
            expand_links: false,
            ..Default::default()
        },
    );

    let ctx = pipeline.query("traefik").unwrap();
    let traefik = ctx
        .entities
        .iter()
        .find(|e| e.iri.contains("traefik"))
        .unwrap();

    // runsOn should be an Entity reference.
    let runs_on = traefik
        .facts
        .iter()
        .find(|f| f.predicate.contains("runsOn"));
    assert!(runs_on.is_some());
    assert_eq!(runs_on.unwrap().value_type, FactValueType::Entity);

    // port should be a Literal.
    let port = traefik.facts.iter().find(|f| f.predicate.contains("port"));
    assert!(port.is_some());
    assert_eq!(port.unwrap().value_type, FactValueType::Literal);
}
