use super::*;
use axum::{extract::State, response::IntoResponse};

#[test]
fn unknown_counts_are_omitted_and_failures_preserve_the_last_snapshot() {
    let cache = GraphMetrics::new(":memory:");
    let unknown = cache.render();
    assert!(unknown.contains("quipu_graph_counts_ready 0\n"));
    assert!(!unknown.contains("quipu_graph_facts "));
    assert!(!unknown.contains("quipu_graph_counts_age_seconds "));
    cache.record(
        Err(quipu::Error::InvalidValue("fixture".into())),
        Instant::now(),
    );
    assert!(!cache.render().contains("quipu_graph_facts "));
    cache.record(Ok((2, 5, 3)), Instant::now());
    let before = cache.state.lock().snapshot.unwrap().completed;
    cache.record(
        Err(quipu::Error::InvalidValue("fixture".into())),
        Instant::now(),
    );
    let body = cache.render();
    assert!(body.contains("quipu_graph_facts 5\n"));
    assert!(body.contains("quipu_graph_counts_ready 1\n"));
    assert!(body.contains("quipu_graph_counts_refresh_failures_total 2\n"));
    assert_eq!(cache.state.lock().snapshot.unwrap().completed, before);
    assert!(body.contains("quipu_graph_counts_age_seconds "));
    assert!(
        !GraphMetrics::new(":memory:")
            .render()
            .contains("quipu_graph_facts ")
    );
}

#[test]
fn wal_stat_stays_current_between_count_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("fixture.db");
    let wal = dir.path().join("fixture.db-wal");
    let cache = GraphMetrics::new(db.to_str().unwrap());
    assert!(!cache.render().contains("quipu_wal_bytes "));
    std::fs::write(&wal, b"first").unwrap();
    assert!(cache.render().contains("quipu_wal_bytes 5\n"));
    std::fs::write(&wal, b"longer fixture").unwrap();
    assert!(cache.render().contains("quipu_wal_bytes 14\n"));
}

/// A held writer AND every held reader discriminate against both old paths.
/// Repeated scrapes must return the cached counts while no SQL can run.
#[tokio::test(flavor = "current_thread")]
async fn scrapes_do_not_acquire_any_store_connection() {
    let (_dir, handle) = crate::tests::pooled_handle(2);
    let shared = Arc::new(handle);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let held = shared.clone();
    let thread = std::thread::spawn(move || {
        let _writer = held.lock();
        let _readers: Vec<_> = held.readers.conns.iter().map(|r| r.lock()).collect();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    ready_rx.recv().unwrap();
    // Release the holder even when the timeout catches a regression.
    let outcome = tokio::time::timeout(Duration::from_secs(1), async {
        let cold = crate::base::metrics_handler(State(shared.clone()))
            .await
            .unwrap()
            .into_response();
        let bytes = axum::body::to_bytes(cold.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("quipu_graph_counts_ready 0\n"));
        assert!(!body.contains("quipu_graph_facts "));
        shared.graph_metrics.record(Ok((2, 5, 3)), Instant::now());
        for _ in 0..10 {
            let response = crate::base::metrics_handler(State(shared.clone()))
                .await
                .unwrap()
                .into_response();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(
                std::str::from_utf8(&bytes)
                    .unwrap()
                    .contains("quipu_graph_facts 5\n")
            );
        }
    })
    .await;
    release_tx.send(()).unwrap();
    thread.join().unwrap();
    outcome.expect("a metrics scrape acquired a database connection");
}

#[tokio::test]
async fn startup_refresh_runs_without_a_scrape() {
    let shared = Arc::new(crate::StoreHandle::writer_only(
        quipu::Store::open_in_memory().unwrap(),
    ));
    quipu::tool_knot(&mut shared.lock(), &serde_json::json!({"turtle":"<http://example.test/s> <http://example.test/p> <http://example.test/o> ."})).unwrap();
    spawn_refresh(&shared);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if shared.graph_metrics.state.lock().snapshot.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let counts = shared.graph_metrics.state.lock().snapshot.unwrap().counts;
    assert!(counts.1 > 0, "control: the seeded graph must contain facts");
    assert_eq!(counts, shared.read().graph_counts().unwrap());
}
