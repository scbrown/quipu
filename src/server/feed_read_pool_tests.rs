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

use super::feed;

/// Run `call` while another thread holds the writer; true if it answered.
async fn answers_while_writer_held<F, Fut>(call: F) -> bool
where
    F: FnOnce(super::SharedStore) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let (_dir, handle) = super::tests::pooled_handle(2);
    {
        let mut writer = handle.lock();
        quipu::rdf::ingest_rdf(
            &mut writer,
            b"<urn:feed:item> <urn:feed:value> \"present\" .".as_slice(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();
    }
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
        feed::events_get(State(s), headers("feed-events"), Query(p))
            .await
            .is_ok_and(|response| {
                response.0["events"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            })
    })
    .await;
    assert!(ok, "GET /events queued behind a held writer (aegis-4cfnck)");
    recorded_wait("feed-events", "/events");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_poll_by_consumer_does_not_wait_on_the_writer() {
    // The consumer path reads the committed offset too; still read-only.
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"consumer": "chaski"})).unwrap();
        feed::events_get(State(s), headers("feed-consumer"), Query(p))
            .await
            .is_ok_and(|response| {
                response.0["events"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            })
    })
    .await;
    assert!(ok, "GET /events?consumer= queued behind a held writer");
    recorded_wait("feed-consumer", "/events");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_poll_does_not_wait_on_the_writer() {
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"since": 0, "limit": 10})).unwrap();
        feed::changes_get(State(s), headers("feed-changes"), Query(p))
            .await
            .is_ok_and(|response| {
                response.0["records"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            })
    })
    .await;
    assert!(
        ok,
        "GET /changes queued behind a held writer (aegis-4cfnck)"
    );
    recorded_wait("feed-changes", "/changes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transactions_poll_does_not_wait_on_the_writer() {
    let ok = answers_while_writer_held(|s| async move {
        let p = serde_json::from_value(json!({"since": 0, "limit": 10})).unwrap();
        feed::transactions(State(s), headers("feed-transactions"), Query(p))
            .await
            .is_ok_and(|response| {
                response.0["transactions"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            })
    })
    .await;
    assert!(
        ok,
        "GET /transactions queued behind a held writer (aegis-4cfnck)"
    );
    recorded_wait("feed-transactions", "/transactions");
}

fn headers(client: &'static str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "x-quipu-client",
        axum::http::HeaderValue::from_static(client),
    );
    headers
}

#[tokio::test]
async fn feed_error_still_records_caller_store_time() {
    let (_dir, handle) = super::tests::pooled_handle(1);
    let state = Arc::new(handle);
    let params = serde_json::from_value(json!({"capture": "invalid"})).unwrap();
    assert!(
        feed::changes_get(State(state), headers("feed-error"), Query(params))
            .await
            .is_err()
    );
    let rendered = quipu::metrics::metrics().render(0, 0, 0, None);
    for metric in [
        "quipu_store_wait_seconds_total",
        "quipu_store_held_seconds_total",
    ] {
        let key = format!("{metric}{{client=\"feed-error\",endpoint=\"/changes\"}}");
        assert!(
            rendered.lines().any(|line| line.starts_with(&key)),
            "missing {key}"
        );
    }
}

fn recorded_wait(client: &str, endpoint: &str) -> f64 {
    let rendered = quipu::metrics::metrics().render(0, 0, 0, None);
    let key =
        format!("quipu_store_wait_seconds_total{{client=\"{client}\",endpoint=\"{endpoint}\"}} ");
    let seconds = rendered
        .lines()
        .find_map(|line| line.strip_prefix(&key))
        .expect("caller/endpoint wait metric must exist")
        .parse::<f64>()
        .unwrap();
    eprintln!("{client} {endpoint} wait_seconds={seconds}");
    seconds
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feed_wait_metric_detects_real_writer_fallback_wait() {
    // Positive control: an empty pool must fall back to the writer, and the
    // metric must see that contention instead of reporting a vacuous zero.
    let (_dir, handle) = super::tests::pooled_handle(0);
    let state = Arc::new(handle);
    let held = state.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = held.lock();
        ready_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(250));
    });
    ready_rx.recv().unwrap();
    let p = serde_json::from_value(json!({"since": 0, "limit": 1})).unwrap();
    let response = feed::events_get(State(state), headers("feed-fallback"), Query(p))
        .await
        .unwrap();
    holder.join().unwrap();
    assert!(response.0["events"].is_array());
    assert!(recorded_wait("feed-fallback", "/events") >= 0.1);
}
