//! The signed-write settle layer on its own (aegis-bys8d1): the binding
//! re-check and nonce spend that run under the writer lock, and the `record`
//! backstop that stops a refused request from opening a transaction. The
//! server tests cover the whole stack; these pin each layer, so deleting one
//! is a failing test even while the others still hold.

use std::sync::Arc;

use ring::signature::{Ed25519KeyPair, KeyPair};

use super::{AttestState, AttestationSlot, Identity, PendingAttestation, with_identity};
use crate::Store;
use crate::session_attestation::SessionBinding;

const TS: &str = "2026-01-01T00:00:00Z";

fn registered(store: &Store, session: &str, expires_in: i64) -> SessionBinding {
    // One key per session: the store refuses to bind a public key twice.
    let seed = session
        .bytes()
        .fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b));
    let key = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
    let now = crate::time::epoch_secs();
    let binding = SessionBinding::new(
        "urn:crew:tester",
        session,
        hex::encode(key.public_key().as_ref()),
        "urn:crew:lead",
        now - 60,
        now.saturating_add_signed(expires_in),
    )
    .unwrap();
    store.attestation_register(&binding).unwrap();
    binding
}

fn pending(binding: &SessionBinding, nonce: &str) -> Arc<PendingAttestation> {
    PendingAttestation::new(
        binding.session.clone(),
        nonce.to_owned(),
        binding.key_id.clone(),
        binding.introducer.clone(),
    )
}

fn identity(p: &Arc<PendingAttestation>) -> Identity {
    Identity {
        principal: "urn:crew:tester".into(),
        credential_id: None,
        auth_class: "attested_session".into(),
        attestation: AttestationSlot(Some(p.clone())),
    }
}

const NONCE: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn settle_spends_once_and_a_second_request_with_the_nonce_is_a_replay() {
    let store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s1", 3600);
    let first = pending(&b, NONCE);
    store.settle_attestation(&first).unwrap();
    assert_eq!(first.state(), AttestState::Spent);
    // Idempotent within the SAME request (several lock takes, several txs).
    store.settle_attestation(&first).unwrap();
    let replay = pending(&b, NONCE);
    assert!(store.settle_attestation(&replay).is_err());
    assert!(matches!(
        replay.state(),
        AttestState::Refused {
            verdict: "replay",
            ..
        }
    ));
}

#[test]
fn a_revocation_after_the_http_check_is_honoured_and_nothing_commits() {
    // malcolm's arm (2): the binding was valid when the middleware checked it
    // and is revoked while the write waits for the writer.
    let mut store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s2", 3600);
    let p = pending(&b, NONCE);
    store.attestation_revoke("s2").unwrap();
    let before = store.latest_tx_id().unwrap();
    let result = with_identity(Some(identity(&p)), || store.transact(&[], TS, None, None));
    assert!(
        result.is_err(),
        "a revoked session must not open a transaction"
    );
    assert!(matches!(
        p.state(),
        AttestState::Refused {
            verdict: "revoked",
            ..
        }
    ));
    assert_eq!(store.latest_tx_id().unwrap(), before, "nothing committed");
}

#[test]
fn expired_unbound_and_changed_bindings_are_refused_with_their_verdicts() {
    let store = Store::open_in_memory().unwrap();
    let expired = registered(&store, "s3", -30);
    let p = pending(&expired, NONCE);
    assert!(store.settle_attestation(&p).is_err());
    assert!(matches!(
        p.state(),
        AttestState::Refused {
            verdict: "expired",
            ..
        }
    ));

    let ghost = SessionBinding {
        session: "never-registered".into(),
        ..expired.clone()
    };
    let p = pending(&ghost, NONCE);
    assert!(store.settle_attestation(&p).is_err());
    assert!(matches!(
        p.state(),
        AttestState::Refused {
            verdict: "unbound",
            ..
        }
    ));

    let live = registered(&store, "s4", 3600);
    let changed = PendingAttestation::new(
        live.session.clone(),
        NONCE.into(),
        "0".repeat(64),
        live.introducer.clone(),
    );
    assert!(store.settle_attestation(&changed).is_err());
    assert!(matches!(
        changed.state(),
        AttestState::Refused {
            verdict: "invalid",
            ..
        }
    ));
}

#[test]
fn the_record_backstop_settles_when_no_lock_hook_ran() {
    // A path that opens a transaction without the writer-lock hook still
    // spends the nonce, and a replay through the same path cannot commit.
    let mut store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s5", 3600);
    let first = pending(&b, NONCE);
    let tx = with_identity(Some(identity(&first)), || {
        store.transact(&[], TS, None, None)
    })
    .unwrap();
    assert_eq!(first.state(), AttestState::Spent);
    assert_eq!(
        store.transaction_auth(tx).unwrap().map(|i| i.auth_class),
        Some("attested_session".to_owned())
    );
    let replay = pending(&b, NONCE);
    let before = store.latest_tx_id().unwrap();
    assert!(
        with_identity(Some(identity(&replay)), || store.transact(
            &[],
            TS,
            None,
            None
        ))
        .is_err()
    );
    assert_eq!(store.latest_tx_id().unwrap(), before);
}

#[test]
fn a_refused_request_stays_refused_for_every_later_transaction() {
    let mut store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s6", 3600);
    let p = pending(&b, NONCE);
    store.attestation_revoke("s6").unwrap();
    assert!(store.settle_attestation(&p).is_err());
    // Re-registering does NOT rescue this request: the refusal is recorded.
    for _ in 0..2 {
        assert!(with_identity(Some(identity(&p)), || store.transact(&[], TS, None, None)).is_err());
    }
}
