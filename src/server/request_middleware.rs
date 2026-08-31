//! Cross-cutting HTTP request middleware.

pub(crate) async fn log_request(
    req: axum::extract::Request,
    next: axum::middleware::Next,
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
    static REQ_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = REQ_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
