//! Actual generic writer dispatch, including credential verification and HTTP readback.
use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{get, post},
};
use quipu::http_auth::{AccessDecision, AuthGeneration, AuthenticatedPrincipal, BearerPolicy};
use serde_json::json;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn generic_writes_keep_named_shared_and_declared_identity_separate() {
    let rows: Vec<_> = ["alice", "bob"].iter().map(|name| json!({
        "credential_id": format!("{name}-1"), "principal": format!("urn:crew:{name}"),
        "audience": "quipu", "token_sha256": format!("{:x}", Sha256::digest(name.as_bytes())),
    })).collect();
    let registry = quipu::crew_credentials::CredentialRegistry::parse(
        &serde_json::to_vec(&json!({"version":1,"credentials":rows})).unwrap(),
        "quipu",
    )
    .unwrap();
    let policy = Arc::new(
        BearerPolicy::new(Some("shared".into()), None, None, 1)
            .unwrap()
            .with_named(registry),
    );
    let state = Arc::new(crate::StoreHandle::writer_only(
        quipu::Store::open_in_memory().unwrap(),
    ));
    for name in ["alice", "bob", "shared"] {
        state.lock().intern(&format!("urn:test:{name}")).unwrap();
    }
    let app = Router::new()
        .route("/set", post(crate::tools::set_predicate))
        .route("/transactions", get(crate::entity::transactions))
        .layer(middleware::from_fn(
            move |mut req: axum::extract::Request, next| {
                let policy = policy.clone();
                async move {
                    let header = req
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    let authorization = quipu::http_auth::authorize_bearers(
                        true,
                        false,
                        &policy,
                        header.as_deref(),
                        1,
                    );
                    if authorization.decision != AccessDecision::Allow {
                        return axum::response::Response::builder()
                            .status(401)
                            .body(axum::body::Body::empty())
                            .unwrap();
                    }
                    if authorization.generation == Some(AuthGeneration::Current) {
                        req.extensions_mut()
                            .insert(AuthenticatedPrincipal::LEGACY_SHARED_BEARER);
                    }
                    crate::auth::run_authorized(
                        req,
                        next,
                        &policy,
                        authorization,
                        header.as_deref(),
                    )
                    .await
                }
            },
        ))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let call = |name: &'static str| {
        tokio::task::spawn_blocking(move || {
            ureq::post(&format!("http://{address}/set"))
            .set("authorization", &format!("Bearer {name}"))
            .set("x-quipu-principal", "urn:crew:spoofed")
            .set("content-type", "application/json")
            .send_string(&json!({"entity":format!("urn:test:{name}"), "predicate":"urn:test:label", "value":name,
                             "actor":"spoofed", "source":"historical-source"}).to_string())
            .unwrap().into_string().map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap()).unwrap()
        })
    };
    let (a, b, shared) = tokio::join!(call("alice"), call("bob"), call("shared"));
    let readback = tokio::task::spawn_blocking(move || {
        let response = ureq::get(&format!("http://{address}/transactions?since=0&limit=10"))
            .set("authorization", "Bearer shared")
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&response).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(readback["count"], 3);
    for tx in readback["transactions"].as_array().unwrap() {
        assert!(tx["authenticated"]["principal"].is_string());
        assert_eq!(tx["actor"], "spoofed");
    }
    server.abort();
    let store = state.lock();
    for (name, result) in [("alice", a), ("bob", b), ("shared", shared)] {
        let result = result.unwrap();
        let tx_id = result["tx_id"].as_i64().unwrap();
        let proof = store.transaction_auth(tx_id).unwrap().unwrap();
        assert_eq!(
            proof.principal,
            if name == "shared" {
                "legacy-shared-bearer".into()
            } else {
                format!("urn:crew:{name}")
            }
        );
        assert_eq!(
            proof.credential_id,
            (name != "shared").then(|| format!("{name}-1"))
        );
        let tx = store.get_transaction(tx_id).unwrap().unwrap();
        assert_eq!(tx.actor.as_deref(), Some("spoofed"));
        assert_eq!(tx.source.as_deref(), Some("historical-source"));
    }
}
