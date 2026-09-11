#![cfg(feature = "shacl")]

use quipu::{store::Store, tool_knot};
use serde_json::json;

const SHAPES: &str = include_str!("../shapes/governance.ttl");
const DECLARATIONS: &str = r#"
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    <urn:example:DeclaredClass> a rdfs:Class ; rdfs:label "Declared class" .
    <urn:example:property> a rdf:Property ; rdfs:label "Property" ;
        rdfs:range <urn:example:DeclaredClass> .
"#;

fn governed_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("governance", SHAPES, "2026-01-01T00:00:00Z")
        .unwrap();
    store
}

#[test]
fn knot_accepts_standard_schema_declarations_under_loaded_governance() {
    let mut store = governed_store();
    let result = tool_knot(&mut store, &json!({"turtle": DECLARATIONS})).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 5, "{result}");
}

#[test]
fn data_class_declaration_does_not_authorize_instances() {
    let mut store = governed_store();
    tool_knot(&mut store, &json!({"turtle": DECLARATIONS})).unwrap();
    let err = tool_knot(
        &mut store,
        &json!({
            "turtle": "<urn:example:instance> a <urn:example:DeclaredClass> ."
        }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown rdf:type IRIs"), "{err}");
    assert!(
        err.to_string().contains("urn:example:DeclaredClass"),
        "{err}"
    );
}

#[test]
fn schema_declarations_require_named_iris() {
    for class in [
        "http://www.w3.org/2000/01/rdf-schema#Class",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#Property",
    ] {
        let turtle = format!("_:anonymous a <{class}> .");
        assert!(!quipu::validate_shapes(SHAPES, &turtle).unwrap().conforms);
    }
}
