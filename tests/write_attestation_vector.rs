//! The published test vectors for signed writes (aegis-bys8d1 S1; v2 adds the
//! audience, aegis-72cpbx).
//!
//! Clients that do not link quipu (seeds signs with ed25519-dalek in wasm)
//! prove they produce the same bytes by reproducing `tests/vectors/
//! write-attestation-v1.json` and `write-attestation-v2.json`. This test is the other half: the committed file
//! must equal what quipu's OWN functions compute, so the vector cannot drift
//! from the server. Regenerate deliberately with
//! `QUIPU_UPDATE_VECTORS=1 cargo test --test write_attestation_vector`; a
//! change to this file is a wire-format change and needs review as one.
#![cfg(not(target_arch = "wasm32"))]

use base64::Engine as _;
use quipu::session_attestation::{
    AttestationEnvelope, SessionBinding, SignedBinding, WRITE_V1, WRITE_V2, WriteBinding,
    body_sha256, canonical_message,
};
use ring::signature::{Ed25519KeyPair, KeyPair};

const PATH: &str = "tests/vectors/write-attestation-v1.json";
const PATH_V2: &str = "tests/vectors/write-attestation-v2.json";
/// The receiving store's id, as `GET /stats` reports it (`store_id`).
const AUDIENCE: &str = "urn:uuid:0f1e2d3c-4b5a-4697-8877-665544332211";

fn compute() -> serde_json::Value {
    compute_for(None)
}

fn compute_for(audience: Option<&str>) -> serde_json::Value {
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
        version: if audience.is_some() {
            WRITE_V2
        } else {
            WRITE_V1
        }
        .into(),
        key_id: binding.key_id.clone(),
        session: binding.session.clone(),
        introducer: binding.introducer.clone(),
        issued_at_epoch: 1_800_000_100,
        nonce: "00112233445566778899aabbccddeeff".into(),
        signature: String::new(),
        audience: audience.map(str::to_owned),
    };
    let write = WriteBinding {
        method: "POST",
        path: "/knot",
        content_type: "application/json",
        body_sha256: &body_hash,
        audience,
    };
    let message = canonical_message(&envelope, &SignedBinding::Write(write));
    envelope.signature = hex::encode(key.sign(&message).as_ref());
    let envelope_json = serde_json::to_string(&envelope).unwrap();
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(envelope_json.as_bytes());
    let mut vector = serde_json::json!({
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
    });
    if let Some(audience) = audience {
        vector["description"] = serde_json::json!(
            "Signed HTTP write, quipu-write-v2: v1 plus the audience, the receiving store's id. \
             Ed25519 (RFC 8032, deterministic). Every value below is derived by quipu's own \
             functions from the inputs; a client must reproduce each one byte for byte."
        );
        vector["inputs"]["audience"] = serde_json::json!(audience);
        let rules = vector["rules"].as_array_mut().unwrap();
        rules.extend([
            serde_json::json!("audience is the receiving store's store_id exactly as GET /stats reports it"),
            serde_json::json!("canonical_message is the v1 lines under the v2 tag, then audience=<store_id>\\n LAST"),
            serde_json::json!("envelope_json carries the same audience; a store with another id refuses it as invalid"),
            serde_json::json!("a v1 envelope must carry NO audience field; a v2 envelope must carry one"),
        ]);
    }
    vector
}

fn check_committed(path: &str, computed: &serde_json::Value) {
    let rendered = serde_json::to_string_pretty(computed).unwrap() + "\n";
    if std::env::var_os("QUIPU_UPDATE_VECTORS").is_some() {
        std::fs::write(path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}; regenerate with QUIPU_UPDATE_VECTORS=1"));
    assert_eq!(
        committed, rendered,
        "the published vector {path} drifted from quipu's own output"
    );
}

#[test]
fn the_committed_vector_is_what_quipu_computes() {
    check_committed(PATH, &compute());
}

#[test]
fn the_committed_v2_vector_is_what_quipu_computes() {
    check_committed(PATH_V2, &compute_for(Some(AUDIENCE)));
}

#[test]
fn the_v2_message_is_the_v1_message_retagged_plus_the_audience() {
    let v1 = compute()["derived"]["canonical_message"]
        .as_str()
        .unwrap()
        .to_owned();
    let v2 = compute_for(Some(AUDIENCE))["derived"]["canonical_message"]
        .as_str()
        .unwrap()
        .to_owned();
    let expected = v1.replacen(WRITE_V1, WRITE_V2, 1) + &format!("audience={AUDIENCE}\n");
    assert_eq!(v2, expected);
    // The v2 signature does not verify for any other audience.
    let d = &compute_for(Some(AUDIENCE))["derived"];
    let key = d["public_key_hex"].as_str().unwrap();
    let sig = d["signature_hex"].as_str().unwrap();
    let other = v2.replace(AUDIENCE, "urn:uuid:00000000-0000-4000-8000-000000000000");
    assert!(quipu::signing::verify_hex(key, v2.as_bytes(), sig));
    assert!(!quipu::signing::verify_hex(key, other.as_bytes(), sig));
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
