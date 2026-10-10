//! Cross-cutting HTTP request middleware.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "request_completion.rs"]
mod request_completion;

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
    request_completion::cancellations().render(out);
}

tokio::task_local! {
    /// The write code path of this request (aegis-gwkd76), from its route.
    static REQUEST_WRITE_KIND: Option<quipu::write_kind::WriteKind>;
}

/// The request's write kind, to carry across a `spawn_blocking` hop.
pub(crate) fn request_write_kind() -> Option<quipu::write_kind::WriteKind> {
    REQUEST_WRITE_KIND.try_with(|k| *k).ok().flatten()
}

tokio::task_local! {
    /// The declared write provenance of this request (aegis-7zp4rc), set on
    /// write routes only.
    static REQUEST_WRITE_PROVENANCE: Option<std::sync::Arc<quipu::write_provenance::RequestProvenance>>;
}

/// The request's write provenance, to carry across a `spawn_blocking` hop.
pub(crate) fn request_write_provenance()
-> Option<std::sync::Arc<quipu::write_provenance::RequestProvenance>> {
    REQUEST_WRITE_PROVENANCE
        .try_with(Clone::clone)
        .ok()
        .flatten()
}

/// Classify the request's provenance headers, for a write route only.
fn write_provenance_of(
    headers: &axum::http::HeaderMap,
    client: &str,
    endpoint: &str,
) -> Option<std::sync::Arc<quipu::write_provenance::RequestProvenance>> {
    quipu::write_kind::WriteKind::for_route(endpoint)?;
    let value = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    Some(std::sync::Arc::new(
        quipu::write_provenance::RequestProvenance::classify(
            client,
            endpoint,
            [
                value("x-quipu-agent"),
                value("x-quipu-harness"),
                value("x-quipu-host"),
                value("x-quipu-session"),
                value("x-quipu-model"),
            ],
        ),
    ))
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
    let declared_host = req
        .headers()
        .get("x-quipu-host")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let declared_agent = req
        .headers()
        .get("x-quipu-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0);
    let provenance = write_provenance_of(req.headers(), &client, &endpoint);
    let attributed = |log| {
        quipu::request_usage::with_request_context(
            quipu::request_usage::with_declared_attribution(
                log,
                declared_host.as_deref(),
                declared_agent.as_deref(),
            ),
            peer,
            provenance.as_deref(),
        )
    };
    eprintln!(
        "{}",
        attributed(quipu::request_usage::structured_request_log(
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
        ))
    );
    let completion_provenance = provenance.clone();
    let mut completion = request_completion::Completion {
        id,
        client: client.clone(),
        task,
        method: method.to_string(),
        path,
        endpoint: endpoint.clone(),
        declared_host,
        declared_agent,
        started: std::time::Instant::now(),
        metrics: quipu::metrics::metrics(),
        cancellations: request_completion::cancellations(),
        emit: Box::new(move |log| {
            eprintln!(
                "{}",
                quipu::request_usage::with_request_context(
                    log,
                    peer,
                    completion_provenance.as_deref()
                )
            );
        }),
        finished: false,
    };
    let resp = REQUEST_WRITE_KIND
        .scope(
            quipu::write_kind::WriteKind::for_route(&endpoint),
            REQUEST_WRITE_PROVENANCE.scope(provenance.clone(), next.run(req)),
        )
        .await;
    let status = resp.status().as_u16();
    let auth = resp
        .extensions()
        .get::<quipu::request_usage::AuthOutcome>()
        .copied()
        .unwrap_or(quipu::request_usage::AuthOutcome::Pending);
    let usage = resp
        .extensions()
        .get::<quipu::request_usage::RequestUsage>()
        .copied();
    completion.finish(status, auth, usage, false);
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
        let mut cancelled = String::new();
        request_completion::cancellations().render(&mut cancelled);
        assert!(cancelled.contains(
            "quipu_http_requests_cancelled_total{client=\"unattributed\",endpoint=\"/pending\"} 1"
        ));
        let metrics = quipu::metrics::metrics().render(0, 0, 0, None);
        assert!(
            metrics.contains("quipu_http_requests_total{endpoint=\"/pending\",status=\"499\"} 1")
        );
        app.oneshot(
            axum::http::Request::builder()
                .uri("/done")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(sequence.load(Ordering::Relaxed), 2);
        let mut cancelled = String::new();
        request_completion::cancellations().render(&mut cancelled);
        assert!(!cancelled.contains("endpoint=\"/done\""));
    }

    /// aegis-7zp4rc: a write route's provenance headers are classified in the
    /// middleware and survive the `blocking` hop to the thread that commits; a
    /// read route carries none.
    #[tokio::test]
    async fn write_provenance_reaches_the_blocking_thread_on_write_routes_only() {
        async fn seen() -> String {
            super::super::base::blocking(|| {
                Ok(quipu::write_provenance::current().map_or_else(
                    || "none".to_string(),
                    |p| format!("{}|{}|{}", p.client, p.endpoint, p.completeness.as_str()),
                ))
            })
            .await
            .map_err(|_| ())
            .unwrap()
        }
        let app = axum::Router::new()
            .route("/knot", axum::routing::post(seen))
            .route("/query", axum::routing::post(seen))
            .layer(axum::middleware::from_fn(log_request));
        let call = |uri: &'static str, headers: &[(&'static str, &'static str)]| {
            let mut req = axum::http::Request::builder().method("POST").uri(uri);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            app.clone()
                .oneshot(req.body(axum::body::Body::empty()).unwrap())
        };
        let body = |r: axum::response::Response| async move {
            String::from_utf8(
                axum::body::to_bytes(r.into_body(), 1 << 16)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap()
        };
        let named = [
            ("x-quipu-client", "camayoc-ingress"),
            ("x-quipu-agent", "camayoc"),
            ("x-quipu-harness", "cron"),
            ("x-quipu-host", "host-a"),
        ];
        assert_eq!(
            body(call("/knot", &named).await.unwrap()).await,
            "camayoc-ingress|/knot|complete"
        );
        assert_eq!(
            body(
                call("/knot", &[("x-quipu-client", "camayoc-ingress")])
                    .await
                    .unwrap()
            )
            .await,
            "camayoc-ingress|/knot|absent"
        );
        assert_eq!(body(call("/query", &named).await.unwrap()).await, "none");
    }
}
