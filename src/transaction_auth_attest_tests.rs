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
    let mut binding = SessionBinding::new(
        "urn:crew:tester",
        session,
        hex::encode(key.public_key().as_ref()),
        "urn:crew:lead",
        now - 60,
        now.saturating_add_signed(expires_in),
    )
    .unwrap();
    binding.allow_write = true;
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

#[test]
fn a_write_grant_withdrawn_while_queued_is_refused_as_scope() {
    let mut store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s7", 3600);
    let p = pending(&b, NONCE);
    store.attestation_set_write("s7", false).unwrap();
    let before = store.latest_tx_id().unwrap();
    assert!(with_identity(Some(identity(&p)), || store.transact(&[], TS, None, None)).is_err());
    assert!(matches!(
        p.state(),
        AttestState::Refused {
            verdict: "scope",
            ..
        }
    ));
    assert_eq!(store.latest_tx_id().unwrap(), before);
}

#[test]
fn the_write_grant_never_travels_in_a_serialized_binding() {
    // A producer's binding rides in a share manifest; a grant carried there
    // must neither be emitted nor accepted.
    let store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s8", 3600);
    assert!(b.allow_write);
    let json = serde_json::to_value(&b).unwrap();
    assert!(json.get("allow_write").is_none(), "{json}");
    let mut forged = json;
    forged["allow_write"] = serde_json::Value::Bool(true);
    let parsed: SessionBinding = serde_json::from_value(forged).unwrap();
    assert!(!parsed.allow_write);
}

#[test]
fn a_binding_that_predates_the_grant_migrates_as_share_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE attestation_bindings (
                 session TEXT PRIMARY KEY, agent TEXT NOT NULL, public_key TEXT NOT NULL,
                 key_id TEXT NOT NULL UNIQUE, introducer TEXT NOT NULL,
                 issued_at_epoch INTEGER NOT NULL, expires_at_epoch INTEGER NOT NULL,
                 revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)));",
        )
        .unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
        let b = SessionBinding::new(
            "urn:crew:producer",
            "legacy",
            hex::encode(key.public_key().as_ref()),
            "urn:crew:lead",
            1,
            u64::from(u32::MAX),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO attestation_bindings VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0)",
            rusqlite::params![
                b.session,
                b.agent,
                b.public_key,
                b.key_id,
                b.introducer,
                u32::MAX
            ],
        )
        .unwrap();
    }
    let store = Store::open(path.to_str().unwrap()).unwrap();
    let b = store.attestation_binding("legacy").unwrap().unwrap();
    assert!(
        !b.allow_write,
        "an existing producer binding must read as share-only"
    );
}

#[test]
fn the_http_precheck_refuses_a_share_only_key_as_scope_and_admits_a_granted_one() {
    use crate::session_attestation::{
        AttestationEnvelope, BindingRegistry, SignedBinding, WRITE_V1, WriteBinding,
        canonical_message, check_binding_deferred,
    };
    let key = Ed25519KeyPair::from_seed_unchecked(&[77; 32]).unwrap();
    let now = crate::time::epoch_secs();
    let mut binding = SessionBinding::new(
        "urn:crew:producer",
        "precheck",
        hex::encode(key.public_key().as_ref()),
        "urn:crew:lead",
        now - 60,
        now + 3600,
    )
    .unwrap();
    let write = WriteBinding {
        method: "POST",
        path: "/knot",
        content_type: "application/json",
        body_sha256: &"0".repeat(64),
        audience: None,
    };
    let mut envelope = AttestationEnvelope {
        version: WRITE_V1.into(),
        key_id: binding.key_id.clone(),
        session: "precheck".into(),
        introducer: "urn:crew:lead".into(),
        issued_at_epoch: now,
        nonce: NONCE.into(),
        signature: String::new(),
        audience: None,
    };
    envelope.signature = hex::encode(
        key.sign(&canonical_message(
            &envelope,
            &SignedBinding::Write(write.clone()),
        ))
        .as_ref(),
    );
    for (granted, want) in [(false, Err("scope")), (true, Ok(()))] {
        binding.allow_write = granted;
        let registry = BindingRegistry::default();
        registry.register(binding.clone()).unwrap();
        let got = check_binding_deferred(
            &registry,
            &envelope,
            &SignedBinding::Write(write.clone()),
            now,
            crate::session_attestation::ATTESTATION_SKEW_SECS,
        )
        .map(|_| ())
        .map_err(|r| r.verdict);
        assert_eq!(got, want, "allow_write={granted}");
    }
}

#[test]
fn refuse_if_refused_stops_non_transactional_work_for_a_refused_request() {
    // The guard rw_handler! calls after taking the writer lock.
    let store = Store::open_in_memory().unwrap();
    let b = registered(&store, "s9", 3600);
    let ok = pending(&b, NONCE);
    with_identity(Some(identity(&ok)), super::refuse_if_refused).unwrap();
    store.attestation_revoke("s9").unwrap();
    let refused = pending(&b, "fedcba9876543210fedcba9876543210");
    assert!(store.settle_attestation(&refused).is_err());
    assert!(with_identity(Some(identity(&refused)), super::refuse_if_refused).is_err());
    // Outside any signed request it is a no-op.
    super::refuse_if_refused().unwrap();
}
