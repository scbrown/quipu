//! Durable attestation state (aegis-c9c44, unit A) — one test per property.
//!
//! The savepoint pair is the reason this file exists. Everything else here is
//! parity with the in-memory registry; the two savepoint tests assert the one
//! thing the in-memory registry cannot do at all.

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

use super::Store;
use crate::session_attestation::{
    AttestationBindings, AttestationEnvelope, SessionBinding, ShareBinding, SignedBinding,
    canonical_message, nonce_horizon_secs, verify_binding,
};

const NOW: u64 = 1_800_000_000;
const SKEW: u64 = 300;
const NONCE: &str = "0123456789abcdef0123456789abcdef";

fn keypair() -> Ed25519KeyPair {
    let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

fn binding_for(key: &Ed25519KeyPair, session: &str) -> SessionBinding {
    SessionBinding::new(
        "urn:agent:malcolm",
        session,
        hex::encode(key.public_key().as_ref()),
        "st:introducer",
        NOW - 60,
        NOW + 60,
    )
    .unwrap()
}

fn bound_store() -> (Store, Ed25519KeyPair, SessionBinding) {
    let store = Store::open_in_memory().unwrap();
    let key = keypair();
    let binding = binding_for(&key, "session-1");
    store.attestation_register(&binding).unwrap();
    (store, key, binding)
}

fn share<'a>() -> SignedBinding<'a> {
    SignedBinding::Share(ShareBinding {
        share_id: "sha256:share",
        graph_hash: "sha256:graph",
        shapes_hash: "sha256:shapes",
        tx_anchor: 42,
    })
}

fn signed_envelope(
    key: &Ed25519KeyPair,
    binding: &SessionBinding,
    nonce: &str,
) -> AttestationEnvelope {
    let mut envelope = AttestationEnvelope {
        version: crate::session_attestation::SHARE_V1.to_string(),
        key_id: binding.key_id.clone(),
        session: binding.session.clone(),
        introducer: binding.introducer.clone(),
        issued_at_epoch: NOW,
        nonce: nonce.to_string(),
        signature: String::new(),
    };
    let message = canonical_message(&envelope, &share());
    envelope.signature = hex::encode(key.sign(&message).as_ref());
    envelope
}

// ---------------------------------------------------------------------------
// The savepoint pair. Both arms, because "the nonce is gone" and "the nonce was
// never written" are indistinguishable from one side alone — and the reassuring
// reading is the wrong one.
// ---------------------------------------------------------------------------

#[test]
fn a_rolled_back_mutation_gives_the_nonce_back() {
    let (store, _key, binding) = bound_store();
    store.conn.execute_batch("SAVEPOINT accept").unwrap();
    assert!(
        store.consume_nonce(&binding.session, NONCE, NOW).unwrap(),
        "first spend inside the savepoint succeeds"
    );
    store
        .conn
        .execute_batch("ROLLBACK TO accept; RELEASE accept")
        .unwrap();

    assert!(
        store.consume_nonce(&binding.session, NONCE, NOW).unwrap(),
        "a mutation that was rejected must not burn the caller's nonce: the \
         legitimate holder has to be able to retry, or a refusal becomes a \
         denial of service against the session it refused"
    );
}

#[test]
fn an_accepted_mutation_keeps_the_nonce_spent() {
    let (store, _key, binding) = bound_store();
    store.conn.execute_batch("SAVEPOINT accept").unwrap();
    assert!(store.consume_nonce(&binding.session, NONCE, NOW).unwrap());
    store.conn.execute_batch("RELEASE accept").unwrap();

    assert!(
        !store.consume_nonce(&binding.session, NONCE, NOW).unwrap(),
        "this is the CONTROL for the rollback test above: without it, that \
         test passes just as happily against an implementation that never \
         records a nonce at all"
    );
}

// ---------------------------------------------------------------------------
// Durability — the property the in-memory registry cannot have
// ---------------------------------------------------------------------------

#[test]
fn a_spent_nonce_is_still_spent_after_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("attest.db");
    let path = path.to_str().unwrap();
    let key = keypair();
    let binding = binding_for(&key, "session-1");
    {
        let store = Store::open(path).unwrap();
        store.attestation_register(&binding).unwrap();
        assert!(store.consume_nonce(&binding.session, NONCE, NOW).unwrap());
    }
    let store = Store::open(path).unwrap();
    assert!(
        store
            .attestation_binding(&binding.session)
            .unwrap()
            .is_some(),
        "the binding survives the restart"
    );
    assert!(
        !store.consume_nonce(&binding.session, NONCE, NOW).unwrap(),
        "and so does the spend — a replay window that reopens on restart is \
         the whole defect this unit exists to close"
    );
}

// ---------------------------------------------------------------------------
// Parity with the in-memory registry
// ---------------------------------------------------------------------------

#[test]
fn registration_is_idempotent_but_conflicts_and_key_reuse_refuse() {
    let (store, key, binding) = bound_store();
    store
        .attestation_register(&binding)
        .expect("re-registering the identical binding is a no-op");

    let mut different = binding.clone();
    different.expires_at_epoch = NOW + 600;
    let err = store.attestation_register(&different).unwrap_err();
    assert!(
        err.to_string().contains("conflicting session binding"),
        "names the conflict: {err}"
    );

    let reused = binding_for(&key, "session-2");
    let err = store.attestation_register(&reused).unwrap_err();
    assert!(
        err.to_string().contains("already bound"),
        "one key, two sessions means two nonce ledgers and a reopened replay \
         window: {err}"
    );
}

#[test]
fn revoking_an_unbound_session_refuses_rather_than_silently_succeeding() {
    let (store, _key, _binding) = bound_store();
    let err = store.attestation_revoke("session-absent").unwrap_err();
    assert!(err.to_string().contains("unbound session"), "{err}");

    store.attestation_revoke("session-1").unwrap();
    assert!(
        store
            .attestation_binding("session-1")
            .unwrap()
            .unwrap()
            .revoked,
        "and a real revoke is readable back"
    );
}

#[test]
fn the_key_id_is_recomputed_on_read_not_trusted_from_its_column() {
    let (store, _key, binding) = bound_store();
    store
        .conn
        .execute(
            "UPDATE attestation_bindings SET key_id = 'sha256:tampered' WHERE session = ?1",
            rusqlite::params![binding.session],
        )
        .unwrap();
    let read = store
        .attestation_binding(&binding.session)
        .unwrap()
        .unwrap();
    assert_eq!(
        read.key_id, binding.key_id,
        "the column is an index for the uniqueness constraint, not the truth; \
         the truth is the hash of the stored public key"
    );
}

// ---------------------------------------------------------------------------
// Pruning — both arms, so a prune that deletes everything cannot pass
// ---------------------------------------------------------------------------

#[test]
fn pruning_forgets_only_nonces_that_can_no_longer_be_replayed() {
    let (store, _key, binding) = bound_store();
    let horizon = nonce_horizon_secs(SKEW);
    assert!(
        store
            .consume_nonce(&binding.session, NONCE, NOW - horizon - 1)
            .unwrap()
    );
    let recent = "ffffffffffffffffffffffffffffffff";
    assert!(store.consume_nonce(&binding.session, recent, NOW).unwrap());

    assert_eq!(
        store.attestation_prune_nonces(NOW, SKEW).unwrap(),
        1,
        "exactly the one outside the horizon"
    );
    assert!(
        store.consume_nonce(&binding.session, NONCE, NOW).unwrap(),
        "the pruned nonce is spendable again — safe, because an attestation \
         that old is refused by the skew check before the nonce is consulted"
    );
    assert!(
        !store.consume_nonce(&binding.session, recent, NOW).unwrap(),
        "and the one INSIDE the horizon is untouched: this is what separates a \
         prune from a truncate"
    );
}

// ---------------------------------------------------------------------------
// End to end through the real verifier, not the pieces
// ---------------------------------------------------------------------------

#[test]
fn the_store_is_a_binding_source_the_real_verifier_accepts_then_refuses_on_replay() {
    let (store, key, binding) = bound_store();
    let envelope = signed_envelope(&key, &binding, NONCE);

    let principal = verify_binding(&store, &envelope, &share(), NOW, SKEW)
        .expect("a correctly signed share attestation verifies against the store");
    assert_eq!(principal.agent, "urn:agent:malcolm");
    assert_eq!(principal.session, binding.session);

    let err = verify_binding(&store, &envelope, &share(), NOW, SKEW)
        .expect_err("the same envelope twice is a replay");
    assert!(err.to_string().contains("nonce replay"), "{err}");
}

#[test]
fn a_bad_signature_is_refused_without_spending_the_nonce() {
    let (store, key, binding) = bound_store();
    let mut envelope = signed_envelope(&key, &binding, NONCE);
    envelope.signature = hex::encode([0_u8; 64]);

    let err = verify_binding(&store, &envelope, &share(), NOW, SKEW).unwrap_err();
    assert!(
        err.to_string().contains("signature does not verify"),
        "{err}"
    );
    assert!(
        store.consume_nonce(&binding.session, NONCE, NOW).unwrap(),
        "a forgery must not burn a nonce the legitimate holder still needs — \
         that would turn every rejected attestation into an attack on the \
         session it impersonated"
    );
}
