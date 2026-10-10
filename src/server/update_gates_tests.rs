//! `/update` enforces the write gates `/knot` does (aegis-1hfyk5).
//!
//! Each refusal is judged by what the store HOLDS afterwards, not by the
//! response, the same way ian's C2 arms were measured.

use std::sync::Arc;

use quipu::Store;

use super::super::{SharedStore, StoreHandle};
use super::apply_update_as;

const PROFILE: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
     @prefix ex: <http://ex.org/> .\n\
     ex:ActionShape a sh:NodeShape ; sh:targetClass ex:Action ;\n\
       sh:property [ sh:path ex:sourceKind ; sh:minCount 1 ] .";

fn store(shapes: Option<&str>) -> SharedStore {
    let store = Store::open_in_memory().unwrap();
    if let Some(s) = shapes {
        store
            .load_shapes("profile", s, "2026-10-07T00:00:00Z")
            .unwrap();
    }
    Arc::new(StoreHandle::writer_only(store))
}

fn held(shared: &SharedStore, s: &str) -> usize {
    let store = shared.lock();
    quipu::sparql::query(&store, &format!("SELECT ?p ?o WHERE {{ <{s}> ?p ?o }}"))
        .unwrap()
        .rows()
        .len()
}

fn insert(subject: &str, class: &str, source_kind: bool) -> String {
    let sk = if source_kind {
        "; <http://ex.org/sourceKind> \"observed\""
    } else {
        ""
    };
    format!("INSERT DATA {{ <{subject}> a <{class}> {sk} }}")
}

#[test]
fn an_unknown_type_is_refused_and_nothing_is_stored() {
    // U1/K0: the only loaded shape targets ex:Action, so ex:Unknown is off-vocabulary.
    let shared = store(Some(PROFILE));
    let err = apply_update_as(
        &shared,
        &insert("http://ex.org/u1", "http://ex.org/Unknown", true),
        false,
    )
    .err()
    .expect("an off-vocabulary type must be refused");
    assert!(format!("{err:?}").contains("unknown rdf:type"), "{err:?}");
    assert_eq!(held(&shared, "http://ex.org/u1"), 0);
}

#[test]
fn a_node_missing_a_required_property_is_refused_and_nothing_is_stored() {
    // U2/U7: K5's refusal, now on /update.
    let shared = store(Some(PROFILE));
    let err = apply_update_as(
        &shared,
        &insert("http://ex.org/u7", "http://ex.org/Action", false),
        false,
    )
    .err()
    .expect("a non-conforming node must be refused");
    assert!(
        format!("{err:?}").contains("SHACL validation failed"),
        "{err:?}"
    );
    assert_eq!(held(&shared, "http://ex.org/u7"), 0);
}

#[test]
fn a_conforming_update_is_accepted() {
    // U6, the positive control: the gates do not refuse what /knot accepts.
    let shared = store(Some(PROFILE));
    apply_update_as(
        &shared,
        &insert("http://ex.org/u6", "http://ex.org/Action", true),
        false,
    )
    .unwrap();
    assert_eq!(held(&shared, "http://ex.org/u6"), 2);
}

#[test]
fn a_conforming_update_into_a_named_graph_is_accepted_and_a_bad_one_refused() {
    let shared = store(Some(PROFILE));
    let good = "INSERT DATA { GRAPH <http://ex.org/board> { <http://ex.org/g1> a <http://ex.org/Action> ; <http://ex.org/sourceKind> \"observed\" } }";
    apply_update_as(&shared, good, false).unwrap();
    let bad = "INSERT DATA { GRAPH <http://ex.org/board> { <http://ex.org/g2> a <http://ex.org/Action> } }";
    assert!(apply_update_as(&shared, bad, false).is_err());
    let store = shared.lock();
    let rows = quipu::sparql::query(
        &store,
        "SELECT ?s WHERE { GRAPH <http://ex.org/board> { ?s a ?t } }",
    )
    .unwrap()
    .rows()
    .len();
    assert_eq!(
        rows, 1,
        "only the conforming node landed in the board graph"
    );
}

#[test]
fn with_no_shapes_loaded_the_gates_are_inactive() {
    // No vocabulary authority exists yet, exactly as on /knot.
    let shared = store(None);
    apply_update_as(
        &shared,
        &insert("http://ex.org/n", "http://ex.org/Anything", false),
        false,
    )
    .unwrap();
    assert_eq!(held(&shared, "http://ex.org/n"), 1);
}

#[test]
fn a_pure_deletion_is_not_gated() {
    let shared = store(Some(PROFILE));
    apply_update_as(
        &shared,
        &insert("http://ex.org/d", "http://ex.org/Action", true),
        false,
    )
    .unwrap();
    apply_update_as(
        &shared,
        "DELETE DATA { <http://ex.org/d> <http://ex.org/sourceKind> \"observed\" }",
        false,
    )
    .unwrap();
    assert_eq!(held(&shared, "http://ex.org/d"), 1);
}
