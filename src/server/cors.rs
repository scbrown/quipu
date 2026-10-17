//! Browser API policy. Attribution headers identify usage; they do not grant access.

pub(super) fn layer(origins: &[String]) -> tower_http::cors::CorsLayer {
    // An empty list preserves the existing allow-any policy; configured origins
    // remain an explicit allowlist. Bearer authorization is enforced separately.
    let base = if origins.is_empty() {
        tower_http::cors::CorsLayer::new().allow_origin(tower_http::cors::Any)
    } else {
        let origins: Vec<axum::http::HeaderValue> =
            origins.iter().filter_map(|o| o.parse().ok()).collect();
        tower_http::cors::CorsLayer::new().allow_origin(origins)
    };
    base.expose_headers([axum::http::HeaderName::from_static("x-quipu-truncated")])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-quipu-client"),
            axum::http::HeaderName::from_static("x-quipu-task"),
            axum::http::HeaderName::from_static("x-quipu-purpose"),
        ])
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, Response};
    use tower::ServiceExt;

    async fn preflight(origins: &[String], origin: &str) -> Response<Body> {
        axum::Router::new()
            .route("/query", axum::routing::post(|| async { "ok" }))
            .layer(super::layer(origins))
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/query")
                    .header("origin", origin)
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "content-type,x-quipu-client,x-quipu-task,x-quipu-purpose",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn browser_attribution_headers_are_admitted_without_wildcard_headers() {
        let response = preflight(&[], "http://pages.example").await;
        let headers = response.headers();
        assert_eq!(response.status(), 200);
        assert_eq!(headers["access-control-allow-origin"], "*");
        let allowed = headers["access-control-allow-headers"].to_str().unwrap();
        eprintln!("Access-Control-Allow-Headers: {allowed}");
        let allowed: std::collections::BTreeSet<_> = allowed.split(',').collect();
        assert_eq!(
            allowed,
            [
                "content-type",
                "authorization",
                "x-quipu-client",
                "x-quipu-task",
                "x-quipu-purpose"
            ]
            .into_iter()
            .collect()
        );
        assert!(!allowed.contains("x-unapproved-header"));
        assert!(headers.get("access-control-allow-credentials").is_none());
        let methods = headers["access-control-allow-methods"].to_str().unwrap();
        assert_eq!(methods, "GET,POST,OPTIONS");
    }

    #[tokio::test]
    async fn attribution_does_not_bypass_configured_origin_allowlist() {
        let origins = vec!["https://brief.example".to_owned()];
        let allowed = preflight(&origins, "https://brief.example").await;
        assert_eq!(
            allowed.headers()["access-control-allow-origin"],
            "https://brief.example"
        );
        let rejected = preflight(&origins, "https://sibling.invalid").await;
        assert!(
            rejected
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
    }
}
