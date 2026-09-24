//! aegis-4cfnck: the change-feed reads (`/events`, `/changes`, `/transactions`)
//! must not wait on the WRITER. They served from the writer mutex, so every poll
//! of the feed serialised against every write in the store, and a consumer
//! polling it (the chaski reactor, the journal generator) is the store's
//! heaviest steady reader.
//!
//! Each arm holds the writer on another thread and requires the real HTTP
//! handler to answer within a second. Reverting any handler to `store.lock()`
//! makes its arm time out.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use serde_json::json;

use super::entity;

/// Run `call` while another thread holds the writer; true if it answered.
async fn answers_while_writer_held<F, Fut>(call: F) -> bool
where
    F: FnOnce(super::SharedStore) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let (_dir, handle) = super::tests::pooled_handle(2);
    let shared: super::SharedStore = Arc::new(handle);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let held = shared.clone();
    let holder = std::thread::spawn(move || {
        let _writer = held.lock();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    ready_rx.recv().unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(1), call(shared)).await;
    // Release the holder even when the timeout caught a regression.
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    matches!(outcome, Ok(true))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_poll_does_not_wait_on_the_writer() {
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"since": 0, "limit": 10})).unwrap();
        entity::events_get(State(s), Query(p)).await.is_ok()
    })
    .await;
    assert!(ok, "GET /events queued behind a held writer (aegis-4cfnck)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_poll_by_consumer_does_not_wait_on_the_writer() {
    // The consumer path reads the committed offset too; still read-only.
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"consumer": "chaski"})).unwrap();
        entity::events_get(State(s), Query(p)).await.is_ok()
    })
    .await;
    assert!(ok, "GET /events?consumer= queued behind a held writer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_poll_does_not_wait_on_the_writer() {
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"since": 0, "limit": 10})).unwrap();
        entity::changes_get(State(s), Query(p)).await.is_ok()
    })
    .await;
    assert!(
        ok,
        "GET /changes queued behind a held writer (aegis-4cfnck)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transactions_poll_does_not_wait_on_the_writer() {
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"since": 0, "limit": 10})).unwrap();
        entity::transactions(State(s), Query(p)).await.is_ok()
    })
    .await;
    assert!(
        ok,
        "GET /transactions queued behind a held writer (aegis-4cfnck)"
    );
}
