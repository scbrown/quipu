//! The attestation seam, exercised THROUGH `import_share` (aegis-c9c44).
//!
//! The verifier's own unit tests prove it rejects tamper, replay and downgrade.
//! What they cannot prove is that anything CALLS it — and until this file, nothing
//! did: `session_attestation` had zero callers outside its own module and tests, so
//! a complete and correct verifier sat inert while `share_import.rs` never
//! mentioned it. A capability nothing invokes is not a capability.
//!
//! So every arm here goes through `import_share` and asserts on the store as well
//! as the return value. "Fails BEFORE staging" is a claim about the store, and a
//! test that only reads the `Err` cannot tell a refusal from a rollback.

use super::*;
use crate::session_attestation::{
    AttestationEnvelope, SessionBinding, ShareBinding, SignedBinding,
};

const TS: &str = "2026-09-05T12:00:00Z";
const NOW: u64 = 1_788_609_600; // the same instant, as epoch seconds

/// A REAL share, produced by `crate::share::share`, not a hand-built manifest.
///
/// sattler asked for the in-situ proof on a real share and that is not ceremony:
/// the attestation binds `share_id`/`graph_hash`/`shapes_hash`/`tx_anchor`, so a
/// fixture whose fields are invented proves the verifier agrees with my typing,
/// not that it agrees with the producer.
fn real_share(store: &Store) -> (tempfile::TempDir, ShareImportRequest) {
    let dir = tempfile::tempdir().unwrap();
    let share_dir = dir.path().join("share");
    crate::share::share(
        store,
        share_dir.to_str().unwrap(),
        &crate::share::ShareOptions {
            no_shapes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let read = |n: &str| std::fs::read_to_string(share_dir.join(n)).unwrap();
    let request = ShareImportRequest {
        manifest: serde_json::from_str(&read("manifest.json")).unwrap(),
        export_ntriples: read("export.nt"),
        shapes_turtle: read("shapes.ttl"),
        source: "https://example.org/producer/share".into(),
        actor: Some("alice".into()),
        accept_exact: false,
        attestation: None,
    };
    (dir, request)
}

fn producer_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut store,
        &b"<urn:a> <urn:p> \"one\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        TS,
        Some("alice"),
        None,
    )
    .unwrap();
    store
}

fn keypair() -> ring::signature::Ed25519KeyPair {
    use ring::rand::SystemRandom;
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

/// Register a session in `store` and sign an envelope over `m`'s identity.
fn attest(store: &Store, m: &ShareManifest) -> AttestationEnvelope {
    use ring::signature::KeyPair;
    let key = keypair();
    let public = hex::encode(key.public_key().as_ref());
    let binding = SessionBinding::new(
        "producer-agent",
        "session-1",
        &public,
        "introducer-1",
        NOW - 60,
        NOW + 3600,
    )
    .unwrap();
    let key_id = binding.key_id.clone();
    // Idempotent: several arms register the same session.
    let _ = store.attestation_register(&binding);
    let mut env = AttestationEnvelope {
        version: crate::session_attestation::SHARE_V1.into(),
        key_id,
        session: "session-1".into(),
        introducer: "introducer-1".into(),
        issued_at_epoch: NOW,
        nonce: "b".repeat(32),
        signature: String::new(),
    };
    let payload = SignedBinding::Share(ShareBinding {
        share_id: &m.share_id,
        graph_hash: &m.graph_hash,
        shapes_hash: &m.shapes_hash,
        tx_anchor: m.tx_anchor,
    });
    let msg = crate::session_attestation::canonical_message(&env, &payload);
    env.signature = hex::encode(key.sign(&msg).as_ref());
    env
}

fn staged_graphs(store: &Store) -> i64 {
    store
        .conn
        .query_row("SELECT COUNT(*) FROM graphs", [], |r| r.get(0))
        .unwrap_or(-1)
}

/// ABSENCE IS A TIER, NOT A PASS — and the existing behaviour is unchanged.
#[test]
fn a_share_with_no_envelope_still_imports_and_says_it_is_only_transport_trusted() {
    let producer = producer_store();
    let (_d, req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    let out = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(out.attestation.tier, "transport");
    assert!(out.attestation.agent.is_none());
    assert!(
        out.attestation.note.contains("WHO"),
        "the note must say what is NOT proven: {}",
        out.attestation.note
    );
    // Additive, not a new gate: the import itself is unaffected.
    assert!(
        out.outcome == "staged" || out.outcome == "quarantined",
        "{}",
        out.outcome
    );
}

/// The seam itself: a valid envelope is verified and the producer is named.
#[test]
fn a_valid_envelope_is_verified_and_names_the_producer() {
    let producer = producer_store();
    let (_d, mut req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    req.attestation = Some(attest(&consumer, &req.manifest));
    let out = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(out.attestation.tier, "attested");
    assert_eq!(out.attestation.agent.as_deref(), Some("producer-agent"));
    assert_eq!(out.attestation.session.as_deref(), Some("session-1"));
}

/// THE ARM THAT MATTERS: tamper is refused BEFORE staging, and the store is clean.
/// Asserting only on the `Err` cannot tell a refusal from a rollback, and only one
/// of those is what "before staging" means.
#[test]
fn a_tampered_envelope_is_refused_and_nothing_is_staged() {
    let producer = producer_store();
    let (_d, mut req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    let mut env = attest(&consumer, &req.manifest);
    env.signature = hex::encode([0u8; 64]);
    req.attestation = Some(env);
    let before = staged_graphs(&consumer);
    let err = import_share(&mut consumer, &req, TS, None).unwrap_err();
    assert!(
        err.to_string().contains("signature"),
        "the refusal must name the failure: {err}"
    );
    assert_eq!(
        staged_graphs(&consumer),
        before,
        "a refused attestation left a graph behind — it did not fail BEFORE staging"
    );
}

/// A replayed nonce is refused, and the FIRST import must have SUCCEEDED — without
/// that half the arm passes on a verifier that rejects everything.
#[test]
fn a_replayed_nonce_is_refused_on_the_second_import() {
    let producer = producer_store();
    let (_d, mut req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    req.attestation = Some(attest(&consumer, &req.manifest));
    let first = import_share(&mut consumer, &req, TS, None);
    assert!(
        first.is_ok(),
        "CONTROL FAILED: the first import did not succeed: {first:?}"
    );
    let err = import_share(&mut consumer, &req, TS, None).unwrap_err();
    assert!(err.to_string().contains("replay"), "{err}");
}

/// An envelope bound to a DIFFERENT share must not verify this one. This is why
/// the binding is built from the manifest rather than passed in beside it.
#[test]
fn an_envelope_bound_to_another_share_is_refused() {
    let producer = producer_store();
    let (_d, mut req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    let mut other = req.manifest.clone();
    other.share_id = format!("sha256:{}", "c".repeat(64));
    req.attestation = Some(attest(&consumer, &other));
    let err = import_share(&mut consumer, &req, TS, None).unwrap_err();
    assert!(err.to_string().contains("signature"), "{err}");
}

/// An unbound session cannot attest, however well-formed the envelope is.
#[test]
fn an_unbound_session_is_refused() {
    let producer = producer_store();
    let (_d, mut req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    let mut env = attest(&consumer, &req.manifest);
    env.session = "never-registered".into();
    req.attestation = Some(env);
    let err = import_share(&mut consumer, &req, TS, None).unwrap_err();
    assert!(err.to_string().contains("unbound"), "{err}");
}

/// A WRITE-domain envelope must not be accepted for a SHARE. Cross-domain reuse is
/// the downgrade the two canonical builders exist to prevent.
#[test]
fn a_write_domain_envelope_is_refused_for_a_share() {
    let producer = producer_store();
    let (_d, mut req) = real_share(&producer);
    let mut consumer = Store::open_in_memory().unwrap();
    let mut env = attest(&consumer, &req.manifest);
    env.version = crate::session_attestation::WRITE_V1.into();
    req.attestation = Some(env);
    let err = import_share(&mut consumer, &req, TS, None).unwrap_err();
    assert!(err.to_string().contains("domain mismatch"), "{err}");
}
