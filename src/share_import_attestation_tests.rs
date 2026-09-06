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
        destination: Default::default(),
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
///
/// Since aegis-tadzdf an unregistered session can reach `claimed` -- but only when
/// the MANIFEST carries the key to check the envelope against. Here it does not,
/// so there is nothing to verify and the refusal stands. The distinction matters:
/// `claimed` is "verified against a key nobody vouched for", never "we gave up".
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

// ─── aegis-tadzdf: the three-tier vocabulary ────────────────────────────────────

/// Mint a real share WITH an embedded attestation, the way `share --attest` does.
fn attested_share(store: &Store, dir: &std::path::Path) -> ShareImportRequest {
    let share_dir = dir.join("share-attested");
    let key_path = dir.join("producer.pk8");
    crate::share::share(
        store,
        share_dir.to_str().unwrap(),
        &crate::share::ShareOptions {
            no_shapes: true,
            attest: Some(crate::share::AttestOptions {
                key_path,
                agent: "producer-agent".into(),
                session: "share-session".into(),
                introducer: "producer-self".into(),
                issued_at_epoch: NOW,
                expires_at_epoch: NOW + 3600,
                nonce: "c".repeat(32),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let read = |n: &str| std::fs::read_to_string(share_dir.join(n)).unwrap();
    let manifest: ShareManifest = serde_json::from_str(&read("manifest.json")).unwrap();
    let envelope = manifest.attestation.as_ref().unwrap().envelope.clone();
    ShareImportRequest {
        manifest,
        export_ntriples: read("export.nt"),
        shapes_turtle: read("shapes.ttl"),
        source: "https://example.org/producer/share".into(),
        actor: Some("alice".into()),
        accept_exact: false,
        destination: Default::default(),
        attestation: Some(envelope),
    }
}

/// An unregistered producer key is `claimed` -- never `attested`.
#[test]
fn an_unregistered_producer_key_is_claimed_not_attested() {
    let producer = producer_store();
    let dir = tempfile::tempdir().unwrap();
    let req = attested_share(&producer, dir.path());
    let mut consumer = Store::open_in_memory().unwrap();
    let out = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(out.attestation.tier, "claimed", "{}", out.attestation.note);
    assert_eq!(out.attestation.agent.as_deref(), Some("producer-agent"));
    assert!(
        out.attestation.note.contains("nobody here vouched"),
        "the note must say the key is unvouched: {}",
        out.attestation.note
    );
}

/// CONDITION 1: registering OUT OF BAND reaches `attested`. If this cannot be
/// written the tier is not real, so this is the acceptance test for the whole
/// change.
#[test]
fn registering_out_of_band_reaches_attested() {
    let producer = producer_store();
    let dir = tempfile::tempdir().unwrap();
    let req = attested_share(&producer, dir.path());
    let mut consumer = Store::open_in_memory().unwrap();

    // The operator obtains the binding some other way and registers it. Taking
    // the public key from the share here is a TEST convenience for reaching the
    // same bytes; the point is that the REGISTRATION is a separate act the
    // consumer performs, which `import_share` never does for itself.
    let carried = req.manifest.attestation.as_ref().unwrap().binding.clone();
    consumer.attestation_register(&carried).unwrap();

    let out = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(out.attestation.tier, "attested", "{}", out.attestation.note);
    assert_eq!(out.attestation.agent.as_deref(), Some("producer-agent"));
    assert_eq!(out.attestation.session.as_deref(), Some("share-session"));
}

/// malcolm's added arm: `import_share` must NOT register the binding it was
/// handed. Pins the circularity so a later "convenience" cannot reintroduce it.
#[test]
fn import_does_not_register_the_binding_it_carries() {
    let producer = producer_store();
    let dir = tempfile::tempdir().unwrap();
    let req = attested_share(&producer, dir.path());
    let mut consumer = Store::open_in_memory().unwrap();

    let first = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(first.attestation.tier, "claimed");
    // The store must be no more trusting after the import than before it.
    assert!(
        consumer.attestation_bindings().unwrap().is_empty(),
        "importing a share must not register its producer: that is what makes \
         `attested` mean something"
    );
    // And a second import still cannot reach attested.
    let second = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(
        second.attestation.tier, "claimed",
        "a repeated import must not bootstrap itself into trust"
    );
}

/// At the `claimed` tier a bad signature is REFUSED, and nothing is staged.
///
/// ⚠ THE OBVIOUS VERSION OF THIS TEST PASSES FOR THE WRONG REASON, and mine did.
/// Tampering with `graph_hash` is rejected by the PAYLOAD hash check long before
/// attestation runs, so the test stayed green with the whole signature
/// verification deleted — caught by mutating `verify_unregistered` away and
/// watching it still pass. An assertion of `contains("signature") ||
/// contains("hash")` let the wrong path satisfy it.
///
/// So this corrupts the SIGNATURE and leaves every hash valid: the payload checks
/// all pass, and the only thing that can refuse the import is the `claimed`-tier
/// verification. The assertion names the signature alone.
#[test]
fn a_bad_signature_is_refused_at_the_claimed_tier_and_stages_nothing() {
    let producer = producer_store();
    let dir = tempfile::tempdir().unwrap();
    let mut req = attested_share(&producer, dir.path());

    // Flip one hex digit of the signature; hashes stay correct.
    let corrupt = |sig: &str| {
        let mut c: Vec<char> = sig.chars().collect();
        c[0] = if c[0] == 'a' { 'b' } else { 'a' };
        c.into_iter().collect::<String>()
    };
    let bad = corrupt(&req.attestation.as_ref().unwrap().signature);
    req.attestation.as_mut().unwrap().signature = bad.clone();
    req.manifest
        .attestation
        .as_mut()
        .unwrap()
        .envelope
        .signature = bad;

    let mut consumer = Store::open_in_memory().unwrap();
    let before = staged_graphs(&consumer);
    let err = import_share(&mut consumer, &req, TS, None).unwrap_err();
    assert!(
        err.to_string().contains("signature"),
        "must fail on the SIGNATURE, not on a hash check that would pass anyway: {err}"
    );
    assert_eq!(
        staged_graphs(&consumer),
        before,
        "a refusal must stage nothing"
    );
}

/// The manifest field is ADDITIVE: a share minted without `--attest` is
/// unchanged, and its `share_id` does not depend on the attestation.
#[test]
fn attesting_does_not_change_the_share_id() {
    let producer = producer_store();
    let dir = tempfile::tempdir().unwrap();
    let (_d, plain) = real_share(&producer);
    let attested = attested_share(&producer, dir.path());
    assert_eq!(
        plain.manifest.share_id, attested.manifest.share_id,
        "the attestation is a statement ABOUT the identity, not part of it"
    );
    assert!(plain.manifest.attestation.is_none());
    assert!(attested.manifest.attestation.is_some());
}
