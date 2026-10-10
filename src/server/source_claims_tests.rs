//! Public REST regression: removing A cannot remove B's identical fact.
use axum::{Router, body::Body, http::Request, routing::post as route_post};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

async fn post(app: &Router, path: &str, value: Value) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn rest_source_claims_preserve_identical_fact_until_last_owner() {
    for order in [["A", "B"], ["B", "A"]] {
        let state = Arc::new(crate::StoreHandle::writer_only(
            quipu::Store::open_in_memory().unwrap(),
        ));
        let app = Router::new()
            .route("/knot", post_route())
            .route("/retract/source", route_post(crate::tools::retract_source))
            .route("/query", route_post(crate::query_endpoint::query_post))
            .with_state(state);
        for source in ["A", "B", "B"] {
            post(&app, "/knot", json!({"source":source,"turtle":"<urn:test:overlap> <http://www.w3.org/2000/01/rdf-schema#label> \"overlap control\" ."})).await;
        }
        let query = json!({"query":"SELECT ?o WHERE { <urn:test:overlap> <http://www.w3.org/2000/01/rdf-schema#label> ?o }"});
        assert_eq!(post(&app, "/query", query.clone()).await["count"], 1);
        for (index, source) in order.into_iter().enumerate() {
            let plan = post(
                &app,
                "/retract/source",
                json!({"source":source,"repair":"test"}),
            )
            .await;
            assert_eq!(plan["planned"], 1);
            let result = post(
                &app,
                "/retract/source",
                json!({"source":source,"repair":"test","apply":true,"expect":1}),
            )
            .await;
            assert_eq!(result["remaining"], 0);
            assert_eq!(
                post(&app, "/query", query.clone()).await["count"],
                1 - index
            );
        }
    }
}

fn post_route() -> axum::routing::MethodRouter<crate::SharedStore> {
    axum::routing::post(crate::publication::knot)
}
