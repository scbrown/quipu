//! aegis-11rwfs: the whole-store copy stops at its ceiling.

use std::sync::{Arc, atomic::Ordering};

use oxigraph::{model::GraphName, store::Store as OxStore};
use quipu::Store;

use super::super::super::{SharedStore, StoreHandle};
use super::super::apply_update_as;
use super::{REFUSED, copy};

fn seeded(n: usize) -> SharedStore {
    let shared: SharedStore = Arc::new(StoreHandle::writer_only(Store::open_in_memory().unwrap()));
    let triples: String = (0..n)
        .map(|i| format!("<http://ex.org/s{i}> <http://ex.org/p> \"v{i}\" . "))
        .collect();
    apply_update_as(&shared, &format!("INSERT DATA {{ {triples} }}"), false).unwrap();
    shared
}

fn copy_with(shared: &SharedStore, limit: usize) -> (Result<(), String>, usize) {
    let store = shared.lock();
    let ox = OxStore::new().unwrap();
    let graphs = vec![(0, GraphName::DefaultGraph)];
    let result =
        copy(&store, &ox, &graphs, "variable-predicate", limit).map_err(|e| format!("{e:?}"));
    (result, ox.len().unwrap())
}

#[test]
fn a_copy_past_the_ceiling_is_refused_before_it_finishes() {
    let shared = seeded(10);
    let refused_before = REFUSED.load(Ordering::Relaxed);
    let (result, copied) = copy_with(&shared, 5);
    let error = result.expect_err("10 facts over a ceiling of 5 must be refused");
    assert!(
        error.contains("cannot be sliced (variable-predicate)"),
        "{error}"
    );
    assert!(
        error.contains("DELETE DATA"),
        "the refusal names the rewrite: {error}"
    );
    assert!(
        copied <= 5,
        "the copy stops at the ceiling, not after the store: {copied}"
    );
    assert!(REFUSED.load(Ordering::Relaxed) > refused_before);
}

#[test]
fn at_the_ceiling_or_unbounded_the_copy_completes() {
    let shared = seeded(10);
    assert_eq!(copy_with(&shared, 10), (Ok(()), 10));
    assert_eq!(copy_with(&shared, 0), (Ok(()), 10), "0 means unbounded");
}

#[test]
fn an_unsliceable_cleanup_under_the_default_ceiling_still_works() {
    // The exact shape that recycled production: open subject AND predicate.
    let shared = seeded(3);
    apply_update_as(
        &shared,
        "INSERT DATA { GRAPH <urn:scratch> { <urn:a> <urn:p> 1 . <urn:b> <urn:q> 2 } }",
        false,
    )
    .unwrap();
    apply_update_as(
        &shared,
        "DELETE WHERE { GRAPH <urn:scratch> { ?s ?p ?o } }",
        false,
    )
    .unwrap();
    let store = shared.lock();
    let left = quipu::sparql::query(
        &store,
        "SELECT ?s WHERE { GRAPH <urn:scratch> { ?s ?p ?o } }",
    )
    .unwrap()
    .rows()
    .len();
    let kept = quipu::sparql::query(&store, "SELECT ?s WHERE { ?s <http://ex.org/p> ?o }")
        .unwrap()
        .rows()
        .len();
    assert_eq!(
        (left, kept),
        (0, 3),
        "scratch emptied, default graph untouched"
    );
}

#[test]
fn the_full_copy_metric_names_both_outcomes() {
    let mut out = String::new();
    super::render(&mut out);
    assert!(out.contains("# TYPE quipu_sparql_update_full_copy_total counter"));
    assert!(out.contains("quipu_sparql_update_full_copy_total{outcome=\"started\"}"));
    assert!(out.contains("quipu_sparql_update_full_copy_total{outcome=\"refused\"}"));
}
