//! Shared query routes; typed answers do not alter the existing SPARQL route.

use crate::{SharedStore, admission::read_blocking, base::AppError};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};

pub(crate) fn routes() -> Router<SharedStore> {
    Router::new()
        .route("/query", get(super::query_get).post(super::query_post_http))
        .route("/rule_query", post(rule_query))
}

async fn rule_query(
    State(store): State<SharedStore>,
    Json(request): Json<quipu::rule_answers::Request>,
) -> Result<Json<quipu::rule_answers::Answer>, AppError> {
    read_blocking(move || Ok(Json(quipu::rule_answers::query(&store.read(), &request)?))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    async fn rule_answer_states_actual_http_matches_readonly_native_and_refuses_duplicate_fields() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("answers.db");
        let path = file.to_str().unwrap();
        let mut store = quipu::Store::open(path).unwrap();
        store
            .load_shapes(
                "rule-answers",
                include_str!("../../shapes/rule-answers.ttl"),
                "2026-01-01",
            )
            .unwrap();
        quipu::tool_knot(&mut store, &json!({"turtle":
            "<urn:supported> <urn:positive> <urn:target> . <urn:refuted> <urn:negative> <urn:target> . <urn:both> <urn:positive> <urn:target> ; <urn:negative> <urn:target> ."
        })).unwrap();
        let reader = quipu::Store::open_read_only(path).unwrap();
        let shared = std::sync::Arc::new(crate::StoreHandle::writer_only(reader));
        let app = routes().with_state(shared);
        for subject in ["supported", "refuted", "both", "neither"] {
            let request = json!({"subject":format!("urn:{subject}"),"positive_predicate":"urn:positive",
                "negative_predicate":"urn:negative","object":{"iri":"urn:target"},"judgment_policy":"urn:policy"});
            let response = app
                .clone()
                .oneshot(
                    Request::post("/rule_query")
                        .header("content-type", "application/json")
                        .body(Body::from(request.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            let wire: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(wire, quipu::tool_rule_query(&store, &request).unwrap());
        }
        let duplicate = r#"{"subject":"urn:a","subject":"urn:b","positive_predicate":"urn:p","negative_predicate":"urn:n","object":{"iri":"urn:o"},"judgment_policy":"urn:policy"}"#;
        let response = app
            .oneshot(
                Request::post("/rule_query")
                    .header("content-type", "application/json")
                    .body(Body::from(duplicate))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
