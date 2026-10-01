//! `/update` must see the named graphs it writes (aegis-e9o5ci).
//!
//! The update dataset is built from the graph registry, and `/update` never
//! registered the graphs it wrote into. So data written by one update was
//! readable through `/query` and invisible to every later update's WHERE:
//! a must-not-exist guard on that graph passed every time and duplicates
//! landed. Measured on 0.9.1: two guarded writes to a fresh graph, 2 landed;
//! a third sequential one also landed.

use std::sync::{Arc, Barrier};

use quipu::Store;

use super::super::{SharedStore, StoreHandle};
use super::apply_update_as;

const G: &str = "http://ex.org/g/fresh";

fn fresh() -> SharedStore {
    Arc::new(StoreHandle::writer_only(Store::open_in_memory().unwrap()))
}

fn guarded(value: &str) -> String {
    format!(
        "INSERT {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"{value}\" }} }} \
         WHERE {{ FILTER NOT EXISTS {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> ?o }} }} }}"
    )
}

fn count(shared: &SharedStore, query: &str) -> usize {
    let store = shared.lock();
    quipu::sparql::query(&store, query).unwrap().rows().len()
}

fn claims(shared: &SharedStore) -> usize {
    count(
        shared,
        &format!(
            "SELECT ?o WHERE {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> ?o }} }}"
        ),
    )
}

#[test]
fn a_guard_on_a_fresh_graph_admits_exactly_one_write() {
    let shared = fresh();
    for v in ["a", "b", "c"] {
        apply_update_as(&shared, &guarded(v), false).unwrap();
    }
    assert_eq!(
        claims(&shared),
        1,
        "a must-not-exist guard passed on existing data"
    );
}

#[test]
fn concurrent_guarded_writes_to_a_fresh_graph_land_once() {
    let shared = fresh();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|v| {
            let (shared, barrier) = (Arc::clone(&shared), Arc::clone(&barrier));
            std::thread::spawn(move || {
                barrier.wait();
                apply_update_as(&shared, &guarded(v), false).unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(claims(&shared), 1);
}

#[test]
fn an_update_where_sees_a_graph_an_earlier_update_wrote() {
    let shared = fresh();
    apply_update_as(
        &shared,
        &format!(
            "INSERT DATA {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/v> \"x\" }} }}"
        ),
        false,
    )
    .unwrap();
    apply_update_as(
        &shared,
        &format!(
            "INSERT {{ <http://ex.org/e/copy> <http://ex.org/p/v> ?o }} \
             WHERE {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/v> ?o }} }}"
        ),
        false,
    )
    .unwrap();
    assert_eq!(
        count(
            &shared,
            "SELECT ?o WHERE { <http://ex.org/e/copy> <http://ex.org/p/v> ?o }"
        ),
        1
    );
}

#[test]
fn registering_a_written_graph_is_idempotent_and_keeps_existing_rows() {
    let store = Store::open_in_memory().unwrap();
    let g = store.graph_create(G).unwrap();
    store.graph_ensure_registered(g).unwrap();
    store.graph_ensure_registered(g).unwrap();
    assert_eq!(
        store
            .all_named_graph_ids()
            .unwrap()
            .iter()
            .filter(|x| **x == g)
            .count(),
        1
    );
    store
        .graph_ensure_registered(quipu::schema::ROOT_GRAPH)
        .unwrap();
}

#[test]
fn registering_never_rewrites_an_overlay_graph() {
    let store = Store::open_in_memory().unwrap();
    let parent = store.graph_create("http://ex.org/g/parent").unwrap();
    let overlay = store
        .overlay_create("http://ex.org/g/overlay", parent)
        .unwrap();
    store.graph_ensure_registered(overlay).unwrap();
    // Re-binding as an overlay of the same parent still succeeds only if the
    // row kept its class and parent.
    assert_eq!(
        store
            .overlay_create("http://ex.org/g/overlay", parent)
            .unwrap(),
        overlay
    );
}
