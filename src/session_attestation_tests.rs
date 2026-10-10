//! Unit tests for the session attestation verifier.

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

use super::*;

const NOW: u64 = 1_800_000_000;
const NONCE: &str = "0123456789abcdef0123456789abcdef";

fn fixture() -> (BindingRegistry, Ed25519KeyPair, SessionBinding) {
    let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    let mut binding = SessionBinding::new(
        "urn:agent:malcolm",
        "session-1",
        hex::encode(key.public_key().as_ref()),
        "creel-extension:key-1",
        NOW - 60,
        NOW + 60,
    )
    .unwrap();
    binding.allow_write = true; // signs the write domain too (aegis-bys8d1)
    let registry = BindingRegistry::default();
    registry.register(binding.clone()).unwrap();
    (registry, key, binding)
}

fn share<'a>() -> SignedBinding<'a> {
    SignedBinding::Share(ShareBinding {
        share_id: "sha256:share",
        graph_hash: "sha256:graph",
        shapes_hash: "sha256:shapes",
        tx_anchor: 42,
    })
}

fn envelope(binding: &SessionBinding) -> AttestationEnvelope {
    AttestationEnvelope {
        version: SHARE_V1.into(),
        key_id: binding.key_id.clone(),
        session: binding.session.clone(),
        introducer: binding.introducer.clone(),
        issued_at_epoch: NOW,
        nonce: NONCE.into(),
        signature: String::new(),
        audience: None,
    }
}

fn sign(key: &Ed25519KeyPair, envelope: &mut AttestationEnvelope, payload: &SignedBinding<'_>) {
    envelope.signature = crate::signing::sign_hex(key, &canonical_message(envelope, payload));
}

#[test]
fn both_domains_use_one_verifier_and_distinct_canonical_builders() {
    let (registry, key, binding) = fixture();
    let payload = share();
    let mut env = envelope(&binding);
    sign(&key, &mut env, &payload);
    let principal = registry.verify(&env, &payload, NOW, 30).unwrap();
    assert_eq!(principal.agent, "urn:agent:malcolm");

    let write = SignedBinding::Write(WriteBinding {
        method: "POST",
        path: "/episode",
        content_type: "application/json",
        body_sha256: "sha256:body",
        audience: None,
    });
    env.version = WRITE_V1.into();
    env.nonce = "abcdef0123456789abcdef0123456789".into();
    sign(&key, &mut env, &write);
    assert!(registry.verify(&env, &write, NOW, 30).is_ok());
}

#[test]
fn tamper_substitution_replay_and_domain_downgrade_are_rejected() {
    let (registry, key, binding) = fixture();
    let payload = share();
    let mut env = envelope(&binding);
    sign(&key, &mut env, &payload);
    let mut tampered = share();
    let SignedBinding::Share(ref mut share) = tampered else {
        unreachable!()
    };
    share.graph_hash = "sha256:altered";
    assert!(registry.verify(&env, &tampered, NOW, 30).is_err());

    assert!(registry.verify(&env, &payload, NOW, 30).is_ok());
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());

    let mut wrong_domain = envelope(&binding);
    wrong_domain.version = WRITE_V1.into();
    sign(&key, &mut wrong_domain, &payload);
    assert!(registry.verify(&wrong_domain, &payload, NOW, 30).is_err());
}

#[test]
fn unbound_expired_revoked_and_malformed_nonce_are_rejected_without_consuming_nonce() {
    let (registry, key, binding) = fixture();
    let payload = share();
    let mut env = envelope(&binding);
    sign(&key, &mut env, &payload);

    env.session = "unknown".into();
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());
    env.session = binding.session.clone();
    env.nonce = "not-a-nonce".into();
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());
    env.nonce = NONCE.into();
    assert!(registry.verify(&env, &payload, NOW + 120, 30).is_err());
    registry.revoke(&binding.session).unwrap();
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());
}

#[test]
fn registration_is_idempotent_but_conflicts_and_key_reuse_refuse() {
    let (registry, _key, binding) = fixture();
    registry.register(binding.clone()).unwrap();
    let mut conflict = binding.clone();
    conflict.agent = "urn:agent:other".into();
    assert!(registry.register(conflict).is_err());
    let mut reused = binding;
    reused.session = "session-2".into();
    assert!(registry.register(reused).is_err());
}
