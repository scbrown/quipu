//! The published test vector for signed writes (aegis-bys8d1 S1).
//!
//! Clients that do not link quipu (seeds signs with ed25519-dalek in wasm)
//! prove they produce the same bytes by reproducing `tests/vectors/
//! write-attestation-v1.json`. This test is the other half: the committed file
//! must equal what quipu's OWN functions compute, so the vector cannot drift
//! from the server. Regenerate deliberately with
//! `QUIPU_UPDATE_VECTORS=1 cargo test --test write_attestation_vector`; a
//! change to this file is a wire-format change and needs review as one.
#![cfg(not(target_arch = "wasm32"))]

use base64::Engine as _;
use quipu::session_attestation::{
    AttestationEnvelope, SessionBinding, SignedBinding, WRITE_V1, WriteBinding, body_sha256,
    canonical_message,
};
use ring::signature::{Ed25519KeyPair, KeyPair};

const PATH: &str = "tests/vectors/write-attestation-v1.json";

fn compute() -> serde_json::Value {
    let seed = [0x5eu8; 32];
    let key = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    let public_key = hex::encode(key.public_key().as_ref());
    let binding = SessionBinding::new(
        "urn:example:agent",
        "example-session",
        public_key.clone(),
        "urn:example:introducer",
        1_800_000_000,
        1_900_000_000,
    )
    .unwrap();
    let body = "{\"turtle\":\"<urn:example:s> <urn:example:p> \\\"o\\\" .\"}";
    let body_hash = body_sha256(body.as_bytes());
    let mut envelope = AttestationEnvelope {
        version: WRITE_V1.into(),
        key_id: binding.key_id.clone(),
        session: binding.session.clone(),
        introducer: binding.introducer.clone(),
        issued_at_epoch: 1_800_000_100,
        nonce: "00112233445566778899aabbccddeeff".into(),
        signature: String::new(),
    };
    let write = WriteBinding {
        method: "POST",
        path: "/knot",
        content_type: "application/json",
        body_sha256: &body_hash,
    };
    let message = canonical_message(&envelope, &SignedBinding::Write(write));
    envelope.signature = hex::encode(key.sign(&message).as_ref());
    let envelope_json = serde_json::to_string(&envelope).unwrap();
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(envelope_json.as_bytes());
    serde_json::json!({
        "description": "Signed HTTP write, quipu-write-v1. Ed25519 (RFC 8032, deterministic). \
            Every value below is derived by quipu's own functions from the inputs; a client \
            must reproduce each one byte for byte.",
        "inputs": {
            "ed25519_seed_hex": hex::encode(seed),
            "agent": binding.agent,
            "session": binding.session,
            "introducer": binding.introducer,
            "issued_at_epoch": envelope.issued_at_epoch,
            "nonce": envelope.nonce,
            "method": "POST",
            "path": "/knot",
            "content_type": "application/json",
            "body": body,
        },
        "derived": {
            "public_key_hex": public_key,
            "key_id": binding.key_id,
            "body_sha256": body_hash,
            "canonical_message": String::from_utf8(message).unwrap(),
            "signature_hex": envelope.signature,
            "envelope_json": envelope_json,
            "header_name": "x-quipu-attestation",
            "header_value": header,
        },
        "rules": [
            "body_sha256 is lowercase hex of the raw body bytes, 64 chars, no prefix",
            "key_id is \"sha256:\" + lowercase hex SHA-256 of the RAW 32-byte public key: it HAS a prefix",
            "body_sha256 has NO prefix; key_id HAS one; do not normalise either",
            "canonical_message is exactly the bytes signed; every line ends in \\n",
            "no field may contain a control character (newline, CR, tab, ...)",
            "path is the request path exactly as sent; a signed write carries no query string",
            "content_type is the Content-Type header byte for byte",
            "nonce is 32 lowercase hex chars, single use; issued_at within 300 s of server time",
            "header_value is base64url WITHOUT padding of envelope_json",
            "accepted on POST /knot, /update, /episode; an invalid attestation is 401 and never falls back to a bearer"
        ]
    })
}

#[test]
fn the_committed_vector_is_what_quipu_computes() {
    let computed = compute();
    let rendered = serde_json::to_string_pretty(&computed).unwrap() + "\n";
    if std::env::var_os("QUIPU_UPDATE_VECTORS").is_some() {
        std::fs::write(PATH, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(PATH)
        .unwrap_or_else(|e| panic!("{PATH}: {e}; regenerate with QUIPU_UPDATE_VECTORS=1"));
    assert_eq!(
        committed, rendered,
        "the published vector drifted from quipu's own output"
    );
}

#[test]
fn the_vector_signature_verifies_and_a_one_byte_change_does_not() {
    let v = compute();
    let d = &v["derived"];
    let message = d["canonical_message"].as_str().unwrap();
    let key = d["public_key_hex"].as_str().unwrap();
    let sig = d["signature_hex"].as_str().unwrap();
    assert!(quipu::signing::verify_hex(key, message.as_bytes(), sig));
    let tampered = message.replacen("/knot", "/knoT", 1);
    assert!(!quipu::signing::verify_hex(key, tampered.as_bytes(), sig));
}
