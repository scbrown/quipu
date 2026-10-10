//! Real pooled HTTP handler controls: same query as the measured baseline,
//! structured field recognized, defaultoff refuses, and enabled lookup is read-only.
use axum::{extract::State, response::IntoResponse};
use quipu::{Datum, Op, Store, Value, vector::KnowledgeVectorStore};
use serde_json::json;
use std::sync::Arc;
fn served(enabled: bool) -> (tempfile::TempDir, super::super::SharedStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corpus.db");
    let path = path.to_str().unwrap();
    let mut store = Store::open(path).unwrap();
    store.search_config_mut().structured = enabled;
    let entity = store.intern("https://example.org/a").unwrap();
    let predicate = store.intern("https://example.org/status").unwrap();
    store
        .transact(
            &[Datum {
                entity,
                attribute: predicate,
                value: Value::Str("open".into()),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    store
        .embed_entity(entity, "memory", &[1.0, 0.0], "2026-01-01T00:00:00Z")
        .unwrap();
    let readers = super::super::ReadPool::open(path, &store, 1);
    assert_eq!(readers.len(), 1);
    let shared = Arc::new(super::super::StoreHandle::serving(
        store,
        readers,
        path,
        Default::default(),
    ));
    (dir, shared)
}
#[tokio::test]
async fn structured_http_defaultoff_and_pooled_candidate_positive_zero_controls() {
    let input = json!({"query":"memory","embedding":[1.0,0.0],"structured_query":"<https://example.org/status>:open"});
    let (_dir, off) = served(false);
    let error = super::super::tools::search(State(off), axum::Json(input.clone()))
        .await
        .unwrap_err();
    let response = error.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("disabled"));
    let (_dir, on) = served(true);
    let before = on.read().latest_tx_id().unwrap();
    let positive = super::super::tools::search(State(on.clone()), axum::Json(input.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(positive["count"], 1);
    assert!(positive.get("ignored_fields").is_none());
    assert_eq!(positive["structured"]["complete"], true);
    let mut zero = input;
    zero["structured_query"] = json!("<https://example.org/status>:missing");
    let zero = super::super::tools::search(State(on.clone()), axum::Json(zero))
        .await
        .unwrap()
        .0;
    assert_eq!(zero["count"], 0);
    assert!(zero.get("ignored_fields").is_none());
    assert_eq!(on.read().latest_tx_id().unwrap(), before);
}
