use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::post,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use super::{READING, share_asof, spool_dir};
use crate::{SharedStore, StoreHandle};

const G: &str = "https://example.org/project/board";

fn app() -> (Router, String) {
    let mut store = quipu::Store::open_in_memory().unwrap();
    let graph = store.graph_create(G).unwrap();
    quipu::rdf::ingest_rdf_to_graph(
        &mut store,
        b"<https://example.org/item/one> <https://example.org/title> \"one\" .\n\
          <https://example.org/item/two> <https://example.org/title> \"two\" ."
            .as_slice(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
        graph,
    )
    .unwrap();
    let identity = store.store_id().unwrap();
    let shared: SharedStore = Arc::new(StoreHandle::writer_only(store));
    let app = Router::new()
        .route("/share/asof", post(share_asof))
        .with_state(shared);
    (app, identity)
}

async fn post_cut(app: Router, input: Value) -> (StatusCode, Option<String>, String) {
    let response = app
        .oneshot(
            Request::post("/share/asof")
                .header("content-type", "application/json")
                .body(Body::from(input.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let retry = response
        .headers()
        .get("retry-after")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap();
    (status, retry, String::from_utf8(bytes.to_vec()).unwrap())
}

/// `(event, id, data)` for each SSE event in a body.
fn events(body: &str) -> Vec<(String, Option<String>, String)> {
    body.split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .filter_map(|block| {
            let mut event = None;
            let mut id = None;
            let mut data = None;
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("id: ") {
                    id = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = Some(v.to_string());
                }
            }
            Some((event?, id, data?))
        })
        .collect()
}

/// The read-phase lock is process-wide; route tests take turns.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Busy first, then a real cut, then a refusal.
#[tokio::test]
async fn a_small_cut_is_one_frame_and_refusals_come_first() {
    let _serial = SERIAL.lock().await;
    let (app, identity) = app();
    let input = json!({"graphs": [G], "expected_store_id": identity});

    // A read phase already running: refused with Retry-After, not queued.
    READING.store(true, Ordering::Release);
    let (status, retry, body) = post_cut(app.clone(), input.clone()).await;
    READING.store(false, Ordering::Release);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(retry.as_deref(), Some("60"));
    assert!(body.contains("cut_in_progress"), "{body}");

    // A small cut is one quipu.asof frame: {manifest, records}.
    let (status, _, body) = post_cut(app.clone(), input.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let frames = events(&body);
    assert_eq!(frames.len(), 1, "{body}");
    assert_eq!(frames[0].0, "quipu.asof");
    let single: Value = serde_json::from_str(&frames[0].2).unwrap();
    assert_eq!(single["manifest"]["source"]["store_id"], identity);
    assert_eq!(single["records"].as_array().unwrap().len(), 2);
    assert_eq!(
        frames[0].1.as_deref(),
        Some(single["manifest"]["watermark_tx"].to_string().as_str())
    );
    let spool = spool_dir().join(format!("{}.parts", sha256_of_manifest(&single["manifest"])));
    assert!(!spool.exists(), "{}", spool.display());

    // A refusal is an ordinary error response, never a started stream.
    let wrong = json!({"graphs": [G], "expected_store_id": "urn:uuid:another-store"});
    let (status, _, body) = post_cut(app.clone(), wrong).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.contains("quipu.asof"), "{body}");
    // The lock was released after each phase.
    assert!(!READING.load(Ordering::Acquire));
}

fn sha256_of_manifest(manifest: &Value) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(manifest).unwrap()))
}

/// A cut over one frame streams begin, parts and a commit barrier whose
/// digests chain exactly as F6's do.
#[tokio::test]
async fn a_large_cut_streams_begin_parts_and_a_commit_barrier() {
    let _serial = SERIAL.lock().await;
    let mut store = quipu::Store::open_in_memory().unwrap();
    let graph = store.graph_create(G).unwrap();
    let mut doc = String::new();
    for i in 0..4000 {
        doc.push_str(&format!(
            "<https://example.org/item/{i}> <https://example.org/title> \"item number {i} with a title long enough to fill frames\" .\n"
        ));
    }
    quipu::rdf::ingest_rdf_to_graph(
        &mut store,
        doc.as_bytes(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
        graph,
    )
    .unwrap();
    let identity = store.store_id().unwrap();
    let shared: SharedStore = Arc::new(StoreHandle::writer_only(store));
    let app = Router::new()
        .route("/share/asof", post(share_asof))
        .with_state(shared);
    let input = json!({"graphs": [G], "expected_store_id": identity});
    let (status, _, body) = post_cut(app, input).await;
    assert_eq!(status, StatusCode::OK);
    let frames = events(&body);
    let (first, last) = (frames.first().unwrap(), frames.last().unwrap());
    assert_eq!(first.0, "quipu.asof.begin");
    assert_eq!(last.0, "quipu.asof.commit");
    let begin: Value = serde_json::from_str(&first.2).unwrap();
    let commit: Value = serde_json::from_str(&last.2).unwrap();
    assert!(first.1.is_none(), "only the barrier carries an id");
    assert_eq!(
        last.1.as_deref(),
        Some(begin["source_tx"].to_string().as_str())
    );
    assert_eq!(begin["manifest"]["source"]["store_id"], identity);
    assert_eq!(begin["source"], commit["source"]);
    let parts = &frames[1..frames.len() - 1];
    assert!(parts.len() > 1, "{} parts", parts.len());
    assert_eq!(parts.len() as u64, begin["parts"].as_u64().unwrap());
    let mut hasher = Sha256::new();
    let mut records = 0;
    for (event, id, data) in parts {
        assert_eq!(event, "quipu.asof.part");
        assert!(id.is_none());
        let part: Value = serde_json::from_str(data).unwrap();
        assert_eq!(
            part["digest_before"],
            hex::encode(hasher.clone().finalize())
        );
        hasher.update(data.as_bytes());
        records += part["records"].as_array().unwrap().len();
    }
    let digest = hex::encode(hasher.finalize());
    assert_eq!(begin["digest"], digest);
    assert_eq!(commit["digest"], digest);
    assert_eq!(records, 4000);
    let spool = spool_dir().join(format!(
        "{}.parts",
        begin["manifest_sha256"]
            .as_str()
            .unwrap()
            .trim_start_matches("sha256:")
    ));
    assert!(!spool.exists(), "{}", spool.display());
}

#[test]
fn the_route_is_write_class_until_read_authorization_exists() {
    assert!(quipu::http_auth::is_write_endpoint("/share/asof"));
    assert!(!quipu::http_auth::READ_ENDPOINTS.contains(&"/share/asof"));
}
