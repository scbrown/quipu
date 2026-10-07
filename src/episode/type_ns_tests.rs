//! `/episode` node types in the public Quechua vocabulary (aegis-kpy8ec).
//!
//! A bare type stays the legacy `base_ns` class, byte-for-byte. A `quechua:`
//! type names `<QUECHUA>Local`, behind the same closed-vocabulary gate, and
//! never moves the instance IRI.

use super::*;
use crate::{tool_episode, tool_query, tool_shapes};
use serde_json::json;

const LEGACY: &str = namespace::DEFAULT_BASE_NS;

fn store_with_shapes(turtle: &str) -> Store {
    let store = Store::open_in_memory().unwrap();
    tool_shapes(
        &store,
        &json!({"action": "load", "name": "type-ns", "turtle": turtle}),
    )
    .unwrap();
    store
}

const QUECHUA_SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
     @prefix q: <https://scbrown.github.io/quechua/ns#> .\n\
     q:WorkItemShape a sh:NodeShape ; sh:targetClass q:WorkItem .";

const LEGACY_SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
     @prefix aegis: <http://aegis.gastown.local/ontology/> .\n\
     aegis:WorkItemShape a sh:NodeShape ; sh:targetClass aegis:WorkItem .";

fn episode(node_type: &str) -> serde_json::Value {
    json!({
        "name": "type-ns-episode",
        "episode_body": "type namespace",
        "source": "unit-test",
        "group_id": "test",
        "nodes": [{"name": "aegis-kpy8ec", "type": node_type}]
    })
}

/// Asserted-only: the FILTER form does not let inference answer for it.
fn asserted(store: &Store, class: &str) -> bool {
    let query = format!("ASK {{ <{LEGACY}aegis-kpy8ec> a ?t . FILTER(?t = <{class}>) }}");
    tool_query(store, &json!({"query": query})).unwrap()["result"] == true
}

#[test]
fn a_bare_type_emits_the_legacy_turtle_unchanged() {
    let ep: Episode = serde_json::from_value(episode("WorkItem")).unwrap();
    let ttl = episode_to_turtle(&ep, "2026-10-07T00:00:00Z", LEGACY, "h");
    assert!(
        ttl.contains("aegis:aegis-kpy8ec a aegis:WorkItem ;"),
        "{ttl}"
    );
    assert!(!ttl.contains(namespace::QUECHUA), "{ttl}");
    assert_eq!(
        node_type_iri("WorkItem", LEGACY),
        format!("{LEGACY}WorkItem")
    );
}

#[test]
fn a_quechua_type_is_written_in_the_quechua_namespace() {
    let mut store = store_with_shapes(QUECHUA_SHAPES);
    tool_episode(&mut store, &episode("quechua:WorkItem")).unwrap();

    let quechua = format!("{}WorkItem", namespace::QUECHUA);
    assert!(asserted(&store, &quechua), "quechua class must be asserted");
    // The instance IRI did not move; only the class did.
    assert!(!asserted(&store, &format!("{LEGACY}WorkItem")));
}

#[test]
fn a_quechua_type_is_refused_until_its_shapes_are_loaded() {
    let mut store = store_with_shapes(LEGACY_SHAPES);
    let err = tool_episode(&mut store, &episode("quechua:WorkItem")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unknown rdf:type IRIs"), "{msg}");
    assert!(
        msg.contains(&format!("{}WorkItem", namespace::QUECHUA)),
        "{msg}"
    );
    let any = format!("ASK {{ <{LEGACY}aegis-kpy8ec> ?p ?o }}");
    assert_eq!(
        tool_query(&store, &json!({"query": any})).unwrap()["result"],
        false
    );
}

#[test]
fn a_loaded_quechua_class_does_not_govern_the_bare_name() {
    let mut store = store_with_shapes(QUECHUA_SHAPES);
    let err = tool_episode(&mut store, &episode("WorkItem")).unwrap_err();
    assert!(
        err.to_string().contains(&format!("{LEGACY}WorkItem")),
        "{err}"
    );
}

#[test]
fn malformed_and_foreign_prefixed_types_are_refused() {
    for bad in [
        "quechua:",
        "quechua:Work Item",
        "quechua:a:b",
        "foo:WorkItem",
    ] {
        let mut store = store_with_shapes(QUECHUA_SHAPES);
        assert!(
            tool_episode(&mut store, &episode(bad)).is_err(),
            "type '{bad}' must be refused"
        );
    }
}
