//! Signed HTTP writes: a write authenticated by a session attestation instead
//! of a bearer (aegis-bys8d1).
//!
//! The client signs `session_attestation::canonical_message` over the request's
//! method, path, content type and body hash with a key registered once, out of
//! band, through `quipu attest`. Nothing secret crosses the wire.
//!
//! Three stages, each for a reason stated where it happens:
//! 1. HERE, before the handler: decode the envelope, hash the body, and run every
//!    check except the nonce spend against a READ connection, plus a read of
//!    whether the nonce is already spent. A cheap, early refusal.
//! 2. When the handler takes the writer lock (`StoreHandle::lock`): the binding
//!    is re-checked and the nonce spent in the same lock hold as the work, and a
//!    refused request cannot open a transaction (`transaction_auth::record`).
//! 3. HERE, after the handler: the refusal, if any, becomes a 401 with its
//!    verdict; otherwise the nonce is spent durably (a spend inside a savepoint
//!    that rolled back must not leave it replayable, and a write that took no
//!    writer lock must not be replayable later against a different state).
//!
//! An attestation header that is present but invalid is a 401. It NEVER falls
//! back to a bearer on the same request.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use quipu::session_attestation::{
    ATTESTATION_SKEW_SECS, AttestationEnvelope, Refusal, SignedBinding, WRITE_V2, WriteBinding,
};
use quipu::transaction_auth::{AttestState, AttestationSlot, Identity, PendingAttestation};

use crate::SharedStore;

/// The header carrying `base64url(JSON AttestationEnvelope)`, unpadded.
pub(crate) const HEADER: &str = "x-quipu-attestation";

/// Endpoints that accept a signed write, each audited for its barrier:
/// /knot, /update and /episode mutate ONLY through store transactions, so
/// `transaction_auth::record` refuses a refused request. /graph/create writes
/// the graphs registry WITHOUT a transaction; it is an `rw_handler!`, which
/// calls `refuse_if_refused` right after the writer lock is taken. Any other
/// endpoint is added only after the same audit names its barrier.
const ATTESTED_PATHS: [&str; 4] = ["/knot", "/update", "/episode", "/graph/create"];

/// The largest body a signed write may carry (the server's own body limit).
const MAX_BODY: usize = 64 * 1024 * 1024;

pub(crate) async fn handle(
    store: SharedStore,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let (pending, identity, req) = match verify(&store, req).await {
        Ok(verified) => verified,
        Err(refusal) => return refuse(&refusal),
    };
    let mut req = req;
    req.extensions_mut()
        .insert(quipu::http_auth::AuthenticatedPrincipal::attested(
            &identity.principal,
        ));
    let mut response = super::run_identified(Some(identity), next.run(req)).await;
    if let AttestState::Refused { verdict, message } = pending.state() {
        return refuse(&Refusal { verdict, message });
    }
    // Spent by the lock hook = spent in autocommit at lock acquisition, so
    // already durable: a failure here is benign. UNSPENT = no writer lock was
    // taken, so nothing was mutated; if the spend fails, the nonce would stay
    // replayable against a later state, so answer INDETERMINATE (503) rather
    // than 2xx. That loses nothing and a client re-signs (malcolm, bys8d1).
    let was_unspent = pending.state() == AttestState::Unspent;
    if let Err(e) = spend_durably(&store, &pending).await {
        eprintln!(
            "{} signed write: durable nonce spend failed for session {}: {e}",
            quipu::time::now_iso(),
            pending.session
        );
        if was_unspent {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({
                    "error": "signed write indeterminate: its nonce could not be recorded; \
                              nothing was written; re-sign with a fresh nonce",
                    "verdict": "error",
                    "indeterminate": true,
                })),
            )
                .into_response();
        }
    }
    response
        .extensions_mut()
        .insert(quipu::request_usage::AuthOutcome::AuthenticatedAttested);
    response
}

type Verified = (Arc<PendingAttestation>, Identity, axum::extract::Request);

async fn verify(store: &SharedStore, req: axum::extract::Request) -> Result<Verified, Refusal> {
    let invalid = |message: &str| Refusal {
        verdict: "invalid",
        message: message.to_owned(),
    };
    let path = req.uri().path().to_owned();
    if req.uri().query().is_some() {
        return Err(invalid("a signed write must not carry a query string"));
    }
    if !ATTESTED_PATHS.contains(&path.as_str()) {
        return Err(invalid(&format!(
            "signed writes are accepted on {} only",
            ATTESTED_PATHS.join(", ")
        )));
    }
    let envelope = decode(
        req.headers()
            .get(HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
    )?;
    let method = req.method().as_str().to_owned();
    let content_type = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| invalid("the request body could not be read within the size limit"))?;
    let body_sha256 = quipu::session_attestation::body_sha256(&bytes);

    let checked = {
        let store = store.clone();
        let envelope = envelope.clone();
        let (method, path, content_type) = (method.clone(), path.clone(), content_type.clone());
        tokio::task::spawn_blocking(move || {
            let reader = store.read();
            // v2 signs THIS store's id. Taken from the store, never from the
            // envelope: the envelope's claim is only compared against it.
            let audience = if envelope.version == WRITE_V2 {
                Some(reader.store_id().map_err(|e| Refusal {
                    verdict: "error",
                    message: format!("this store's id could not be read: {e}"),
                })?)
            } else {
                None
            };
            let write = WriteBinding {
                method: &method,
                path: &path,
                content_type: &content_type,
                body_sha256: &body_sha256,
                audience: audience.as_deref(),
            };
            let principal = quipu::session_attestation::check_binding_deferred(
                &*reader,
                &envelope,
                &SignedBinding::Write(write),
                quipu::time::epoch_secs(),
                ATTESTATION_SKEW_SECS,
            )?;
            match reader.attestation_nonce_spent(&envelope.session, &envelope.nonce) {
                Ok(false) => Ok(principal),
                Ok(true) => Err(Refusal {
                    verdict: "replay",
                    message: "the nonce was already spent".to_owned(),
                }),
                Err(e) => Err(Refusal {
                    verdict: "error",
                    message: e.to_string(),
                }),
            }
        })
        .await
        .map_err(|e| Refusal {
            verdict: "error",
            message: format!("attestation check failed: {e}"),
        })??
    };

    let pending = PendingAttestation::new(
        checked.session.clone(),
        envelope.nonce.clone(),
        checked.key_id.clone(),
        checked.introducer.clone(),
    );
    let identity = Identity {
        principal: checked.agent.clone(),
        credential_id: Some(checked.key_id.clone()),
        auth_class: "attested_session".to_owned(),
        attestation: AttestationSlot(Some(pending.clone())),
    };
    let req = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
    Ok((pending, identity, req))
}

/// `base64url(JSON AttestationEnvelope)`, with or without padding.
fn decode(header: &str) -> Result<AttestationEnvelope, Refusal> {
    let invalid = |message: String| Refusal {
        verdict: "invalid",
        message,
    };
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header.trim().trim_end_matches('='))
        .map_err(|e| invalid(format!("{HEADER} is not base64url: {e}")))?;
    serde_json::from_slice(&raw)
        .map_err(|e| invalid(format!("{HEADER} is not an attestation envelope: {e}")))
}

async fn spend_durably(store: &SharedStore, pending: &PendingAttestation) -> quipu::Result<bool> {
    let store = store.clone();
    let (session, nonce) = (pending.session.clone(), pending.nonce.clone());
    tokio::task::spawn_blocking(move || store.lock().attestation_ensure_spent(&session, &nonce))
        .await
        .map_err(|e| quipu::Error::InvalidValue(format!("durable nonce spend failed: {e}")))?
}

fn refuse(refusal: &Refusal) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "error": format!("unauthorized: signed write refused ({})", refusal.verdict),
            "verdict": refusal.verdict,
            "message": refusal.message,
            "reason": "attestation_refused",
            "credential_type": "attested_signature",
            "expected_scope": "write",
            "provisioning": "Ask the identity introducer to register the client public key and bind its agent/session with allow_write; renew expired bindings. A rejected signature never falls back to bearer authentication.",
        })),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(quipu::request_usage::AuthOutcome::Unauthorized);
    response
}

#[cfg(test)]
#[path = "write_attest_tests.rs"]
mod tests;
