//! Cross-cutting HTTP request middleware.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST_STARTS: AtomicU64 = AtomicU64::new(0);

/// Arrivals include pending/cancelled requests and the metrics scrape itself.
/// Completion counters cannot distinguish low traffic from stalled handlers.
pub(crate) fn render_request_starts(out: &mut String) {
    out.push_str(
        "# HELP quipu_http_requests_started_total HTTP requests received, including pending and cancelled requests.\n\
         # TYPE quipu_http_requests_started_total counter\n",
    );
    let _ = writeln!(
        out,
        "quipu_http_requests_started_total {}",
        REQUEST_STARTS.load(Ordering::Relaxed)
    );
}

pub(crate) async fn log_request(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    log_request_with_sequence(req, next, &REQUEST_STARTS).await
}

async fn log_request_with_sequence(
    req: axum::extract::Request,
    next: axum::middleware::Next,
    sequence: &AtomicU64,
) -> axum::response::Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    // Use the route template, not the raw path, to bound metric cardinality.
    let endpoint = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or_else(|| "unmatched".to_string(), |m| m.as_str().to_string());
    let client = quipu::metrics::normalize_client(
        req.headers()
            .get("x-quipu-client")
            .and_then(|v| v.to_str().ok()),
        req.headers()
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
    );
    let task = quipu::metrics::normalize_task(
        req.headers()
            .get("x-quipu-task")
            .and_then(|v| v.to_str().ok()),
    );
    // Log before dispatch so a request that never completes is still visible.
    let id = sequence.fetch_add(1, Ordering::Relaxed);
    eprintln!(
        "{}",
        quipu::request_usage::structured_request_log(
            "request_start",
            id,
            &client,
            &task,
            method.as_str(),
            &path,
            &endpoint,
            None,
            None,
            quipu::request_usage::AuthOutcome::Pending,
            None,
        )
    );
    let started = std::time::Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    let elapsed = started.elapsed().as_secs_f64();
    quipu::metrics::metrics().observe_request(&endpoint, status, elapsed);
    quipu::metrics::metrics().observe_client(&client, &task, &endpoint, elapsed);
    let auth = resp
        .extensions()
        .get::<quipu::request_usage::AuthOutcome>()
        .copied()
        .unwrap_or(quipu::request_usage::AuthOutcome::Pending);
    let usage = resp
        .extensions()
        .get::<quipu::request_usage::RequestUsage>()
        .copied();
    eprintln!(
        "{}",
        quipu::request_usage::structured_request_log(
            "request_complete",
            id,
            &client,
            &task,
            method.as_str(),
            &path,
            &endpoint,
            Some(status),
            Some(started.elapsed().as_millis()),
            auth,
            usage,
        )
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn arrivals_count_before_completion_and_survive_cancellation() {
        let sequence = Arc::new(AtomicU64::new(0));
        let entered = Arc::new(tokio::sync::Notify::new());
        let handler_entered = entered.clone();
        let middleware_sequence = sequence.clone();
        let app = axum::Router::new()
            .route(
                "/pending",
                axum::routing::get(move || async move {
                    handler_entered.notify_one();
                    std::future::pending::<()>().await;
                    "never returned"
                }),
            )
            .route("/done", axum::routing::get(|| async { "done" }))
            .layer(axum::middleware::from_fn(move |req, next| {
                let sequence = middleware_sequence.clone();
                async move { log_request_with_sequence(req, next, &sequence).await }
            }));
        assert_eq!(sequence.load(Ordering::Relaxed), 0);
        let pending = tokio::spawn(
            app.clone().oneshot(
                axum::http::Request::builder()
                    .uri("/pending")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            ),
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified())
            .await
            .expect("handler was entered");
        assert_eq!(sequence.load(Ordering::Relaxed), 1);
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        assert_eq!(sequence.load(Ordering::Relaxed), 1);
        app.oneshot(
            axum::http::Request::builder()
                .uri("/done")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(sequence.load(Ordering::Relaxed), 2);
    }
}
