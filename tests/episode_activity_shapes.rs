#![cfg(feature = "shacl")]

use quipu::{store::Store, tool_knot, tool_shapes};
use serde_json::json;

const SHAPES: &str = include_str!("../shapes/provenance.ttl");
const PREFIX: &str = r#"
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn conforms(properties: &str) -> bool {
    let data = format!("{PREFIX}\n<urn:example:episode> a prov:Activity ; {properties} .");
    quipu::validate_shapes(SHAPES, &data).unwrap().conforms
}

#[test]
fn activity_is_governed_for_native_bulk_facts() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .load_shapes("provenance", SHAPES, "2026-01-01T00:00:00Z")
        .unwrap();
    let vocabulary = tool_shapes(&store, &json!({"action": "vocabulary"})).unwrap();
    assert!(
        vocabulary["classes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|class| { class.as_str() == Some("http://www.w3.org/ns/prov#Activity") })
    );
    let result = tool_knot(
        &mut store,
        &json!({"turtle": format!(
            "{PREFIX}\n<urn:example:episode> a prov:Activity ; rdfs:label \"example\" ."
        )}),
    );
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn legacy_stub_with_only_label_is_valid() {
    assert!(conforms(r#"rdfs:label "legacy episode""#));
}

#[test]
fn full_body_and_historical_comments_are_valid() {
    assert!(conforms(
        r#"rdfs:label "body episode" ; rdfs:comment "old stub", "full body""#
    ));
}

#[test]
fn missing_label_is_refused() {
    assert!(!conforms(r#"rdfs:comment "body""#));
}

#[test]
fn iri_label_is_refused() {
    assert!(!conforms("rdfs:label <urn:example:label>"));
}

#[test]
fn non_string_label_is_refused() {
    assert!(!conforms("rdfs:label 42"));
}
