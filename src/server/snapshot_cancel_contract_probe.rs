//! Same-input HTTP contract probe, runnable against the uncancellable base.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use std::sync::Arc;
use tower::ServiceExt;

async fn call(app: axum::Router, route: &str, input: JsonValue) -> (u16, JsonValue) {
    let response = app
        .oneshot(
            Request::post(route)
                .header("content-type", "application/json")
                .body(Body::from(input.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(JsonValue::Null),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_promotion_stop_contract() {
    use sha2::Digest;
    let shared = Arc::new(super::super::StoreHandle::writer_only(
        quipu::Store::open_in_memory().unwrap(),
    ));
    let app = routes().with_state(shared.clone());
    let turtle = "<http://ex/contract> <http://www.w3.org/2000/01/rdf-schema#label> \"control\" .";
    let hash = format!("sha256:{:x}", sha2::Sha256::digest(turtle.as_bytes()));
    let id = snapshot_upload::snapshot_upload_id("stop-contract", &hash);
    let (stage, _) = call(
        app.clone(),
        "/knot/stage",
        serde_json::json!({
            "upload_id":id,"snapshot":"stop-contract","content_hash":hash,"part_hash":hash,
            "payload":turtle,"part_number":0,"total_parts":1,"total_bytes":turtle.len(),
        }),
    )
    .await;
    assert_eq!(stage, 200);
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let thread = std::thread::spawn(move || {
        let _writer = holder.writer.lock();
        locked_tx.send(()).unwrap();
        wait.recv().unwrap();
    });
    locked_rx.recv().unwrap();
    let input = serde_json::json!({"upload_id":id});
    let (accepted, pending) = call(app.clone(), "/knot/promote", input.clone()).await;
    let (cancel_status, cancellation) = call(app.clone(), "/knot/cancel", input.clone()).await;
    release.send(()).unwrap();
    thread.join().unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let (status, body) = call(app.clone(), "/knot/promote", input.clone()).await;
            assert_eq!(status, 200);
            if body["pending"] != true {
                break body;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let facts = shared.lock().graph_counts().unwrap().1;
    eprintln!(
        "STOP_CONTRACT {}",
        serde_json::json!({
            "accepted_http":accepted,"accepted":pending,"cancel_http":cancel_status,
            "cancel":cancellation,"terminal":result,"active_facts":facts,
        })
    );
    assert_eq!(accepted, 200);
    assert_eq!(
        cancel_status, 200,
        "an accepted promotion must have a stop path"
    );
    assert_eq!(result["cancelled"], true);
    assert_eq!(facts, 0);
}
