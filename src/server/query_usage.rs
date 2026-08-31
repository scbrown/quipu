//! Request-usage annotations attached by the query handler.

pub(crate) fn client(headers: &axum::http::HeaderMap) -> String {
    quipu::metrics::normalize_client(
        headers.get("x-quipu-client").and_then(|v| v.to_str().ok()),
        headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
    )
}

pub(crate) fn query_text(input: &serde_json::Value) -> String {
    input
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("<no query field>")
        .chars()
        .take(300)
        .collect()
}

pub(crate) fn annotate(
    mut response: axum::response::Response,
    query_shape: quipu::request_usage::QueryShape,
    result_size: usize,
) -> axum::response::Response {
    response
        .extensions_mut()
        .insert(quipu::request_usage::RequestUsage {
            query_shape,
            result_size,
        });
    response
}

pub(crate) fn result_size(result: &quipu::QueryResult) -> usize {
    match result {
        quipu::QueryResult::Select { rows, .. } => rows.len(),
        quipu::QueryResult::Graph(triples) => triples.len(),
        quipu::QueryResult::Ask(_) => 1,
    }
}
