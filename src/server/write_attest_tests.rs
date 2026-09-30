//! The rw_handler! barrier for signed writes that mutate WITHOUT a
//! transaction (aegis-bys8d1, malcolm's arm). Over HTTP a replay is stopped by
//! the pre-check and create is idempotent, so a server test cannot tell whether
//! `refuse_if_refused` (tools.rs, right after the writer lock) exists at all.
//! This drives the real `graph_create` handler with a request whose binding
//! was valid at the pre-check and revoked before the writer lock: exactly the
//! case only that line stops.

use std::sync::Arc;

use axum::extract::State;
use quipu::session_attestation::SessionBinding;
use quipu::transaction_auth::{AttestState, AttestationSlot, Identity, PendingAttestation};
use ring::signature::{Ed25519KeyPair, KeyPair};

use crate::{SharedStore, StoreHandle};

const GRAPH: &str = "urn:barrier:graph";

fn signed_request(revoke: bool) -> (SharedStore, Arc<PendingAttestation>, Identity) {
    let store = quipu::Store::open_in_memory().unwrap();
    let key = Ed25519KeyPair::from_seed_unchecked(&[31; 32]).unwrap();
    let now = quipu::time::epoch_secs();
    let mut binding = SessionBinding::new(
        "urn:crew:seeds",
        "barrier",
        hex::encode(key.public_key().as_ref()),
        "urn:crew:lead",
        now - 60,
        now + 3600,
    )
    .unwrap();
    binding.allow_write = true;
    store.attestation_register(&binding).unwrap();
    if revoke {
        // Valid when the middleware checked it; revoked while the write queued.
        store.attestation_revoke("barrier").unwrap();
    }
    let pending = PendingAttestation::new(
        binding.session.clone(),
        "0123456789abcdef0123456789abcdef".into(),
        binding.key_id.clone(),
        binding.introducer.clone(),
    );
    let identity = Identity {
        principal: binding.agent,
        credential_id: Some(binding.key_id),
        auth_class: "attested_session".into(),
        attestation: AttestationSlot(Some(pending.clone())),
    };
    (Arc::new(StoreHandle::writer_only(store)), pending, identity)
}

fn graph_exists(store: &SharedStore) -> bool {
    quipu::tool_graph_list(&store.read(), &serde_json::json!({}))
        .unwrap()
        .to_string()
        .contains(GRAPH)
}

#[tokio::test]
async fn a_refused_signed_graph_create_creates_no_graph() {
    let (store, pending, identity) = signed_request(true);
    let result = super::super::run_identified(
        Some(identity),
        crate::tools::graph_create(
            State(store.clone()),
            axum::Json(serde_json::json!({ "graph": GRAPH })),
        ),
    )
    .await;
    assert!(result.is_err(), "a refused request must not reach the tool");
    assert!(matches!(
        pending.state(),
        AttestState::Refused {
            verdict: "revoked",
            ..
        }
    ));
    assert!(
        !graph_exists(&store),
        "the refused request created the graph"
    );
}

#[tokio::test]
async fn control_the_same_signed_graph_create_lands_when_not_refused() {
    let (store, pending, identity) = signed_request(false);
    let result = super::super::run_identified(
        Some(identity),
        crate::tools::graph_create(
            State(store.clone()),
            axum::Json(serde_json::json!({ "graph": GRAPH })),
        ),
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(pending.state(), AttestState::Spent);
    assert!(graph_exists(&store));
}
