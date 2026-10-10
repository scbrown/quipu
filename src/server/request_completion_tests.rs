use super::*;
use std::future::Future;
use std::sync::Arc;

fn completion<'a>(
    metrics: &'a quipu::metrics::Metrics,
    counters: &'a Cancellations,
    logs: Arc<Mutex<Vec<String>>>,
) -> Completion<'a> {
    Completion {
        id: 7,
        client: "test-client".into(),
        task: "unattributed".into(),
        method: "GET".into(),
        path: "/pending".into(),
        endpoint: "/pending".into(),
        declared_host: None,
        declared_agent: None,
        started: Instant::now(),
        metrics,
        cancellations: counters,
        emit: Box::new(move |log| logs.lock().unwrap().push(log)),
        finished: false,
    }
}

#[tokio::test]
async fn dropping_polled_pending_future_accounts_once() {
    let metrics = quipu::metrics::Metrics::default();
    let counters = Cancellations::default();
    let logs = Arc::new(Mutex::new(Vec::new()));
    let guard = completion(&metrics, &counters, logs.clone());
    let mut request = Box::pin(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    });
    std::future::poll_fn(|cx| {
        assert!(request.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(logs.lock().unwrap().is_empty());
    drop(request);
    let logs = logs.lock().unwrap();
    assert_eq!(logs.len(), 1);
    let event: serde_json::Value = serde_json::from_str(&logs[0]).unwrap();
    assert_eq!(event["event"], "request_complete");
    assert_eq!(event["request_id"], 7);
    assert_eq!(event["status"], 499);
    assert_eq!(event["completion_outcome"], "cancelled");
    assert!(event["duration_ms"].is_u64());
    assert_eq!(event["auth_outcome"], "pending");
    assert!(event.get("result_size").is_none());
    let text = metrics.render(0, 0, 0, None);
    assert!(text.contains("quipu_http_requests_total{endpoint=\"/pending\",status=\"499\"} 1"));
    assert!(text.contains("quipu_http_request_duration_seconds_count{endpoint=\"/pending\"} 1"));
    assert!(text.contains("quipu_http_client_requests_total{client=\"test-client\",task=\"unattributed\",endpoint=\"/pending\"} 1"));
    assert_eq!(
        *counters
            .0
            .lock()
            .unwrap()
            .get(&("test-client".into(), "/pending".into()))
            .unwrap(),
        1
    );
}

#[test]
fn completed_response_disarms_drop_and_retains_metadata() {
    let metrics = quipu::metrics::Metrics::default();
    let counters = Cancellations::default();
    let logs = Arc::new(Mutex::new(Vec::new()));
    let mut guard = completion(&metrics, &counters, logs.clone());
    guard.finish(
        200,
        quipu::request_usage::AuthOutcome::NotRequired,
        None,
        false,
    );
    guard.finish(500, quipu::request_usage::AuthOutcome::Pending, None, false);
    drop(guard);
    let logs = logs.lock().unwrap();
    assert_eq!(logs.len(), 1);
    let event: serde_json::Value = serde_json::from_str(&logs[0]).unwrap();
    assert_eq!(event["status"], 200);
    assert_eq!(event["completion_outcome"], "response");
    assert_eq!(event["auth_outcome"], "not_required");
    assert!(counters.0.lock().unwrap().is_empty());
    let text = metrics.render(0, 0, 0, None);
    assert!(text.contains("quipu_http_request_duration_seconds_count{endpoint=\"/pending\"} 1"));
    assert!(!text.contains("status=\"499\""));
}

#[test]
fn cancellation_cardinality_reserves_overflow_and_escapes_labels() {
    let counters = Cancellations::default();
    for i in 0..100 {
        counters.observe(&format!("client-{i}"), "/pending");
    }
    assert_eq!(counters.0.lock().unwrap().len(), 32);
    counters.observe("new-client", "/route/\"quoted\"");
    let mut text = String::new();
    counters.render(&mut text);
    assert!(text.contains("client=\"other\",endpoint=\"/pending\"} 69"));
    assert!(text.contains("endpoint=\"/route/\\\"quoted\\\"\""));
}
