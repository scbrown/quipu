//! Tests for MCP tool handlers.

use super::*;
use super::tools::*;

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
