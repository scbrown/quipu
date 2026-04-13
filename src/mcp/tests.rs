//! Tests for MCP tool handlers.

use std::sync::Arc;

use super::graphiti::*;
use super::tools::*;
use super::*;
use crate::embedding::EmbeddingProvider;
use crate::error::Result as QResult;
use crate::vector::KnowledgeVectorStore;

fn test_store_with_data() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ; ex:name "Alice" ; ex:age "30"^^xsd:integer .
ex:bob a ex:Person ; ex:name "Bob" ; ex:age "25"^^xsd:integer .
"#;
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

    crate::rdf::ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:a ex:v \"1\" .".as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    crate::rdf::ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:b ex:v \"2\" .".as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-02-01",
        None,
        None,
    )
    .unwrap();

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
    crate::rdf::ingest_rdf(
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
    crate::rdf::ingest_rdf(
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

    let input = serde_json::json!({
        "entity": "http://example.org/bob",
        "predicate": "http://example.org/name",
        "timestamp": "2026-02-01"
    });
    let result = tool_retract(&mut store, &input).unwrap();
    assert_eq!(result["retracted"], 1);

    assert_eq!(store.current_facts().unwrap().len(), 2);
}

#[test]
fn test_tool_shapes_load_and_enforce() {
    let mut store = Store::open_in_memory().unwrap();

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

    let list_input = serde_json::json!({ "action": "list" });
    let list_result = tool_shapes(&store, &list_input).unwrap();
    assert_eq!(list_result["count"], 1);

    let good_input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
        "timestamp": "2026-04-04T01:00:00Z"
    });
    let good_result = tool_knot(&mut store, &good_input).unwrap();
    assert_eq!(good_result["conforms"], true);

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
    assert_eq!(defs.len(), 19);
    let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"quipu_query"));
    assert!(names.contains(&"quipu_knot"));
    assert!(names.contains(&"quipu_cord"));
    assert!(names.contains(&"quipu_unravel"));
    assert!(names.contains(&"quipu_impact"));
    assert!(names.contains(&"quipu_validate"));
    assert!(names.contains(&"quipu_search"));
    assert!(names.contains(&"quipu_hybrid_search"));
    assert!(names.contains(&"quipu_unified_search"));
    assert!(names.contains(&"quipu_episode"));
    assert!(names.contains(&"quipu_retract"));
    assert!(names.contains(&"quipu_shapes"));
    assert!(names.contains(&"quipu_search_nodes"));
    assert!(names.contains(&"quipu_search_facts"));
    assert!(names.contains(&"quipu_episodes_complete"));
    assert!(names.contains(&"quipu_propose_schema_change"));
    assert!(names.contains(&"quipu_list_proposals"));
    assert!(names.contains(&"quipu_accept_proposal"));
    assert!(names.contains(&"quipu_reject_proposal"));
}

// ── Schema evolution proposal tool tests ────────────────────────────

#[test]
fn test_propose_and_list_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "kind": "shape",
        "target": "PersonShape",
        "diff": "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:PersonShape a sh:NodeShape .",
        "rationale": "Need to add email property",
        "proposer": "agent/test",
        "trigger_ref": "val-fail-1",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &input).unwrap();
    assert_eq!(result["status"], "pending");
    let id = result["proposal_id"].as_i64().unwrap();
    assert!(id > 0);

    // List all proposals — should find exactly one.
    let list_result = super::proposal::tool_list_proposals(&store, &serde_json::json!({})).unwrap();
    assert_eq!(list_result["count"], 1);
    assert_eq!(list_result["proposals"][0]["id"], id);
    assert_eq!(list_result["proposals"][0]["kind"], "shape");
    assert_eq!(list_result["proposals"][0]["target"], "PersonShape");
    assert_eq!(list_result["proposals"][0]["proposer"], "agent/test");
    assert_eq!(list_result["proposals"][0]["status"], "pending");

    // Filter by status.
    let pending =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "pending" }))
            .unwrap();
    assert_eq!(pending["count"], 1);

    let accepted =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "accepted" }))
            .unwrap();
    assert_eq!(accepted["count"], 0);
}

#[test]
fn test_accept_proposal_roundtrip() {
    let store = Store::open_in_memory().unwrap();

    // Create a valid shape proposal.
    let shape_turtle = r#"@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .
"#;
    let propose = serde_json::json!({
        "kind": "shape",
        "target": "PersonShape",
        "diff": shape_turtle,
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Accept it.
    let accept = serde_json::json!({
        "id": id,
        "decided_by": "aegis/crew/braino",
        "note": "Approved — looks correct",
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let accept_result = super::proposal::tool_accept_proposal(&store, &accept).unwrap();
    assert_eq!(accept_result["status"], "accepted");
    assert_eq!(accept_result["proposal_id"], id);

    // Verify the shape was loaded.
    let shapes = store.list_shapes().unwrap();
    assert!(shapes.iter().any(|(name, _, _)| name == "PersonShape"));
}

#[test]
fn test_reject_proposal_roundtrip() {
    let store = Store::open_in_memory().unwrap();

    let propose = serde_json::json!({
        "kind": "property",
        "target": "ex:dangerousField",
        "diff": "ex:dangerousField rdfs:range xsd:string .",
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Reject it.
    let reject = serde_json::json!({
        "id": id,
        "note": "Too permissive — needs cardinality constraints",
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let reject_result = super::proposal::tool_reject_proposal(&store, &reject).unwrap();
    assert_eq!(reject_result["status"], "rejected");
    assert_eq!(reject_result["proposal_id"], id);

    // Verify it shows as rejected in listing.
    let list =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "rejected" }))
            .unwrap();
    assert_eq!(list["count"], 1);
}

#[test]
fn test_accept_invalid_turtle_stays_pending() {
    let store = Store::open_in_memory().unwrap();

    // Create a shape proposal with invalid Turtle.
    let propose = serde_json::json!({
        "kind": "shape",
        "target": "BadShape",
        "diff": "this is not valid turtle {{{{",
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Accepting should fail — invalid Turtle.
    let accept = serde_json::json!({
        "id": id,
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let err = super::proposal::tool_accept_proposal(&store, &accept).unwrap_err();
    assert!(err.to_string().contains("invalid"));

    // Proposal should still be pending.
    let list =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "pending" }))
            .unwrap();
    assert_eq!(list["count"], 1);
}

#[test]
fn test_reject_missing_note_errors() {
    let store = Store::open_in_memory().unwrap();

    let propose = serde_json::json!({
        "kind": "class",
        "target": "ex:NewClass",
        "diff": "ex:NewClass a rdfs:Class .",
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Reject without note should error.
    let reject = serde_json::json!({ "id": id, "timestamp": "2026-04-13T01:00:00Z" });
    let err = super::proposal::tool_reject_proposal(&store, &reject).unwrap_err();
    assert!(err.to_string().contains("note"));
}

#[test]
fn test_propose_missing_required_fields_errors() {
    let store = Store::open_in_memory().unwrap();

    // Missing 'kind'.
    let input = serde_json::json!({
        "target": "X", "diff": "x", "proposer": "test"
    });
    assert!(super::proposal::tool_propose_schema_change(&store, &input).is_err());

    // Missing 'proposer'.
    let input = serde_json::json!({
        "kind": "shape", "target": "X", "diff": "x"
    });
    assert!(super::proposal::tool_propose_schema_change(&store, &input).is_err());
}

#[test]
fn test_knot_validation_failure_includes_proposal_hint() {
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
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = tool_knot(&mut store, &input).unwrap();
    assert_eq!(result["conforms"], false);
    assert_eq!(
        result["hint"],
        "propose a schema change via quipu_propose_schema_change"
    );
}

#[test]
fn test_extract_type_filter_simple() {
    let sparql = "SELECT ?s WHERE { ?s a <http://example.org/Person> }";
    let filter = super::tools::extract_type_filter(sparql);
    assert_eq!(
        filter,
        Some("entity_type = 'http://example.org/Person'".into())
    );
}

#[test]
fn test_extract_type_filter_rdf_type() {
    let sparql = "SELECT ?s WHERE { ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Bot> }";
    let filter = super::tools::extract_type_filter(sparql);
    assert_eq!(
        filter,
        Some("entity_type = 'http://example.org/Bot'".into())
    );
}

#[test]
fn test_extract_type_filter_complex_returns_none() {
    // FILTER makes this too complex for pushdown
    let sparql = "SELECT ?s WHERE { ?s a <http://example.org/Person> . FILTER(?s != <http://example.org/bob>) }";
    let filter = super::tools::extract_type_filter(sparql);
    assert!(filter.is_none());
}

#[test]
fn test_extract_type_filter_no_type_returns_none() {
    let sparql = "SELECT ?s WHERE { ?s <http://example.org/name> \"Alice\" }";
    let filter = super::tools::extract_type_filter(sparql);
    assert!(filter.is_none());
}

#[test]
fn test_hybrid_search_includes_pushdown_filter() {
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let alice_id = store.intern("http://example.org/alice").unwrap();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(alice_id, "Alice", &emb, "2026-01-01")
        .unwrap();

    let input = serde_json::json!({
        "embedding": emb,
        "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        "limit": 5
    });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();

    // Result should include the pushdown_filter field.
    assert_eq!(
        result["pushdown_filter"],
        "entity_type = 'http://example.org/Person'"
    );
    assert_eq!(result["count"], 1);
}

#[test]
fn test_hybrid_search_vector_only() {
    let store = test_store_with_data();

    // Embed an entity for vector search.
    let eid = store.intern("http://example.org/alice").unwrap();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(eid, "Alice the person", &emb, "2026-01-01")
        .unwrap();

    // Hybrid search with no SPARQL filter — behaves like plain vector search.
    let input = serde_json::json!({
        "embedding": emb,
        "limit": 5
    });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
    assert!(result["sparql_candidates"].is_null());
}

#[test]
fn test_hybrid_search_with_sparql_filter() {
    let mut store = Store::open_in_memory().unwrap();

    // Ingest two entities.
    let ttl = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .\nex:bob a ex:Bot ; ex:name \"Bob\" .";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // Embed both.
    let alice_id = store.intern("http://example.org/alice").unwrap();
    let bob_id = store.intern("http://example.org/bob").unwrap();
    let emb_a: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    let emb_b: Vec<f32> = (0..8).map(|i| (1.1 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(alice_id, "Alice", &emb_a, "2026-01-01")
        .unwrap();
    store
        .embed_entity(bob_id, "Bob", &emb_b, "2026-01-01")
        .unwrap();

    // Hybrid search: SPARQL filters to only Person, vector ranks.
    let input = serde_json::json!({
        "embedding": emb_a,
        "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        "limit": 5
    });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();
    assert_eq!(result["count"], 1); // Only Alice (Person), not Bob (Bot).
    assert_eq!(result["sparql_candidates"], 1);
    let entity = result["results"][0]["entity"].as_str().unwrap();
    assert!(entity.contains("alice"));
}

#[test]
fn test_search_results_include_source_field() {
    let store = test_store_with_data();
    let eid = store.intern("http://example.org/alice").unwrap();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(eid, "Alice the person", &emb, "2026-01-01")
        .unwrap();

    // tool_search results should have source: "knowledge"
    let input = serde_json::json!({ "embedding": emb, "limit": 5 });
    let result = super::tools::tool_search(&store, &input).unwrap();
    assert_eq!(result["results"][0]["source"], "knowledge");

    // tool_hybrid_search results should also have source: "knowledge"
    let input = serde_json::json!({ "embedding": emb, "limit": 5 });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();
    assert_eq!(result["results"][0]["source"], "knowledge");
}

/// Deterministic embedding provider for testing query-text auto-embedding.
struct TestProvider;

impl EmbeddingProvider for TestProvider {
    fn embed_text(&self, text: &str) -> QResult<Vec<f32>> {
        let seed = text.len() as f32;
        Ok((0..8).map(|i| (seed + i as f32 * 0.1).sin()).collect())
    }

    fn dimension(&self) -> usize {
        8
    }
}

#[test]
fn test_search_with_query_text() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" ; rdfs:comment "A software engineer" .
ex:bob rdfs:label "Bob" ; rdfs:comment "A data scientist" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Search using query text (auto-embedded by provider).
    let input = serde_json::json!({ "query": "software engineer", "limit": 5 });
    let result = tool_search(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    assert_eq!(result["results"][0]["source"], "knowledge");
}

#[test]
fn test_search_query_text_without_provider_errors() {
    let store = Store::open_in_memory().unwrap();

    // No embedding provider → query-text search should fail with a clear message.
    let input = serde_json::json!({ "query": "software engineer" });
    let err = tool_search(&store, &input).unwrap_err();
    assert!(err.to_string().contains("no embedding provider"));
}

#[test]
fn test_search_missing_both_params_errors() {
    let store = Store::open_in_memory().unwrap();

    // Neither query nor embedding → error.
    let input = serde_json::json!({ "limit": 5 });
    let err = tool_search(&store, &input).unwrap_err();
    assert!(err.to_string().contains("missing"));
}

#[test]
fn test_search_explicit_embedding_preferred_over_query() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
ex:alice rdfs:label "Alice" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // When both embedding and query are provided, embedding wins.
    let emb: Vec<f32> = (0..8).map(|i| (5.0 + i as f32 * 0.1).sin()).collect();
    let input = serde_json::json!({
        "embedding": emb,
        "query": "ignored because embedding takes precedence",
        "limit": 5
    });
    let result = tool_search(&store, &input).unwrap();
    // Should succeed (uses explicit embedding).
    assert!(result["count"].as_u64().is_some());
}

#[test]
fn test_hybrid_search_with_query_text() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; rdfs:label "Alice" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Hybrid search with query text + SPARQL filter.
    let input = serde_json::json!({
        "query": "Alice",
        "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        "limit": 5
    });
    let result = tool_hybrid_search(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_hybrid_search_query_text_without_provider_errors() {
    let store = Store::open_in_memory().unwrap();

    let input = serde_json::json!({ "query": "test" });
    let err = tool_hybrid_search(&store, &input).unwrap_err();
    assert!(err.to_string().contains("no embedding provider"));
}

// ── search module tests (text-matching search_nodes / search_facts) ──

#[test]
fn test_tool_search_nodes_basic() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let nodes = result["nodes"].as_array().unwrap();
    // At least one node should have "alice" in its IRI.
    assert!(
        nodes
            .iter()
            .any(|n| n["iri"].as_str().unwrap().contains("alice"))
    );
}

#[test]
fn test_tool_search_nodes_with_type_filter() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "entity_type_filter": "http://example.org/Person",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_tool_search_nodes_no_match() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "zzz_nonexistent_entity",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn test_tool_search_nodes_returns_label_and_types() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    let nodes = result["nodes"].as_array().unwrap();
    let alice = nodes
        .iter()
        .find(|n| n["iri"].as_str().unwrap().contains("alice"))
        .unwrap();
    // Should have types populated.
    assert!(!alice["types"].as_array().unwrap().is_empty());
}

#[test]
fn test_tool_search_nodes_with_group_ids() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "test-ep",
        "source": "test",
        "group_id": "my-group",
        "timestamp": "2026-04-04T12:00:00Z",
        "nodes": [
            {"name": "ServerAlpha", "type": "Server", "description": "Production server"}
        ],
        "edges": []
    });
    super::tools::tool_episode(&mut store, &input).unwrap();

    // Search with matching group_id.
    let search_input = serde_json::json!({
        "query": "ServerAlpha",
        "group_ids": ["my-group"],
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &search_input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);

    // Search with non-matching group_id.
    let search_input = serde_json::json!({
        "query": "ServerAlpha",
        "group_ids": ["wrong-group"],
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &search_input).unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn test_tool_search_facts_basic() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "name",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let facts = result["facts"].as_array().unwrap();
    // Should find name predicates.
    assert!(
        facts
            .iter()
            .any(|f| f["predicate"].as_str().unwrap().contains("name"))
    );
}

#[test]
fn test_tool_search_facts_by_value() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let facts = result["facts"].as_array().unwrap();
    assert!(
        facts
            .iter()
            .any(|f| f["target"].as_str().unwrap() == "Alice")
    );
}

#[test]
fn test_tool_search_facts_no_match() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "zzz_nonexistent_predicate",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &input).unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn test_tool_search_facts_with_provenance() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "deploy-ep",
        "source": "test",
        "group_id": "ops-group",
        "timestamp": "2026-04-04T12:00:00Z",
        "nodes": [
            {"name": "AppBeta", "type": "Application"},
            {"name": "HostGamma", "type": "Host"}
        ],
        "edges": [
            {"source": "AppBeta", "target": "HostGamma", "relation": "deployed_on"}
        ]
    });
    super::tools::tool_episode(&mut store, &input).unwrap();

    let search_input = serde_json::json!({
        "query": "deployed_on",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &search_input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let facts = result["facts"].as_array().unwrap();
    let deploy_fact = facts
        .iter()
        .find(|f| f["predicate"].as_str().unwrap().contains("deployed_on"))
        .unwrap();
    // Should have provenance from the episode.
    assert!(!deploy_fact["provenance"].is_null());
}

// ── Graphiti-compatible endpoint tests ────────────────────────────

#[test]
fn test_search_nodes_sparql_fallback() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
@prefix aegis: <https://aegis.dev/ns/> .

ex:tapestry a aegis:WebApplication ;
    rdfs:label "tapestry" ;
    rdfs:comment "Web UI for Gas Town" .
ex:quipu a aegis:KnowledgeGraph ;
    rdfs:label "quipu" ;
    rdfs:comment "AI-native knowledge graph" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Search by label text — no embedding provider, uses SPARQL fallback.
    let input = serde_json::json!({ "query": "tapestry", "max_results": 5 });
    let result = tool_search_nodes(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["nodes"][0]["name"], "tapestry");
}

#[test]
fn test_search_nodes_with_type_filter() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix aegis: <https://aegis.dev/ns/> .

aegis:tapestry a aegis:WebApplication ;
    rdfs:label "tapestry" ;
    rdfs:comment "Web UI" .
aegis:quipu a aegis:KnowledgeGraph ;
    rdfs:label "quipu" ;
    rdfs:comment "Knowledge graph" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Filter to only WebApplication entities.
    let input = serde_json::json!({
        "query": "tapestry",
        "entity_type_filter": "WebApplication",
        "max_results": 5
    });
    let result = tool_search_nodes(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
    assert!(
        result["nodes"][0]["type"]
            .as_str()
            .unwrap()
            .contains("WebApplication")
    );
}

#[test]
fn test_search_nodes_missing_query_errors() {
    let store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({ "max_results": 5 });
    let err = tool_search_nodes(&store, &input).unwrap_err();
    assert!(err.to_string().contains("missing 'query'"));
}

#[test]
fn test_search_nodes_with_vector_search() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" ; rdfs:comment "A software engineer" .
ex:bob rdfs:label "Bob" ; rdfs:comment "A data scientist" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let input = serde_json::json!({ "query": "engineer", "max_results": 5 });
    let result = tool_search_nodes(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_episodes_complete_basic() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "meeting-notes-2026-04",
        "episode_body": "Discussed the new auth middleware requirements",
        "group_id": "aegis-ontology",
        "source_description": "crew/ellie",
        "timestamp": "2026-04-04T14:00:00Z"
    });
    let result = tool_episodes_complete(&mut store, &input).unwrap();
    assert_eq!(result["episode"], "meeting-notes-2026-04");
    assert!(result["tx_id"].as_i64().unwrap() > 0);
    assert!(result["count"].as_i64().unwrap() >= 1);
}

#[test]
fn test_episodes_complete_minimal() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "quick-note"
    });
    let result = tool_episodes_complete(&mut store, &input).unwrap();
    assert_eq!(result["episode"], "quick-note");
    assert!(result["tx_id"].as_i64().unwrap() > 0);
}

#[test]
fn test_episodes_complete_missing_name_errors() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "episode_body": "some text"
    });
    let err = tool_episodes_complete(&mut store, &input).unwrap_err();
    assert!(err.to_string().contains("missing 'name'"));
}

#[test]
fn test_episodes_complete_provenance_queryable() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "deploy-v2",
        "episode_body": "Deployed version 2 to production",
        "source_description": "ci/pipeline",
        "timestamp": "2026-04-04T15:00:00Z"
    });
    tool_episodes_complete(&mut store, &input).unwrap();

    // The episode provenance entity should be queryable via SPARQL.
    let q = serde_json::json!({
        "query": "SELECT ?label WHERE { ?s a <http://www.w3.org/ns/prov#Activity> ; <http://www.w3.org/2000/01/rdf-schema#label> ?label }"
    });
    let result = tool_query(&store, &q).unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["rows"][0]["label"], "deploy-v2");
}
