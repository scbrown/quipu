//! `WebAuthn` assertion verification: a software authenticator built in-test
//! (ring P-256 / Ed25519) produces assertions exactly as the spec lays them
//! out, then every check is exercised in both directions.

use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, Ed25519KeyPair, KeyPair as _};

use super::cose::{self, CoseKey};
use super::webauthn::{Assertion, Registered, verify};
use super::{Scheme, sha256, webauthn_challenge};

pub(crate) const RP_ID: &str = "approve.example.org";
pub(crate) const ORIGIN: &str = "https://approve.example.org";

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A software authenticator holding one credential.
pub(crate) enum Authenticator {
    Es256(EcdsaKeyPair),
    Ed25519(Ed25519KeyPair),
}

/// The knobs a test turns to build a malformed-but-signed assertion.
#[derive(Clone)]
pub(crate) struct Ceremony {
    pub(crate) typ: &'static str,
    pub(crate) challenge_message: Vec<u8>,
    pub(crate) origin: String,
    pub(crate) rp_id: String,
    pub(crate) flags: u8,
    pub(crate) sign_count: u32,
    pub(crate) cross_origin: bool,
}

impl Ceremony {
    pub(crate) fn for_message(message: &[u8]) -> Self {
        Self {
            typ: "webauthn.get",
            challenge_message: message.to_vec(),
            origin: ORIGIN.into(),
            rp_id: RP_ID.into(),
            flags: 0x05, // UP | UV
            sign_count: 7,
            cross_origin: false,
        }
    }
}

/// A signed assertion, raw bytes.
#[derive(Clone)]
pub(crate) struct Signed {
    pub(crate) authenticator_data: Vec<u8>,
    pub(crate) client_data_json: Vec<u8>,
    pub(crate) signature: Vec<u8>,
}

impl Signed {
    pub(crate) fn as_assertion(&self) -> Assertion<'_> {
        Assertion {
            authenticator_data: &self.authenticator_data,
            client_data_json: &self.client_data_json,
            signature: &self.signature,
        }
    }
    /// The three parts base64url-encoded, as a client posts them.
    pub(crate) fn wire(&self) -> (String, String, String) {
        (
            b64url(&self.authenticator_data),
            b64url(&self.client_data_json),
            b64url(&self.signature),
        )
    }
}

impl Authenticator {
    pub(crate) fn es256() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        Self::Es256(
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap(),
        )
    }

    pub(crate) fn ed25519() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Self::Ed25519(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
    }

    pub(crate) fn scheme(&self) -> Scheme {
        match self {
            Self::Es256(_) => Scheme::WebauthnEs256,
            Self::Ed25519(_) => Scheme::WebauthnEddsa,
        }
    }

    pub(crate) fn cose_key(&self) -> Vec<u8> {
        match self {
            Self::Es256(k) => cose::encode(&CoseKey::Es256 {
                sec1_uncompressed: k.public_key().as_ref().to_vec(),
            }),
            Self::Ed25519(k) => cose::encode(&CoseKey::Ed25519 {
                public: k.public_key().as_ref().try_into().unwrap(),
            }),
        }
    }

    pub(crate) fn cose_key_b64url(&self) -> String {
        b64url(&self.cose_key())
    }

    pub(crate) fn sign(&self, c: &Ceremony) -> Signed {
        let mut client = serde_json::json!({
            "type": c.typ,
            "challenge": b64url(&webauthn_challenge(&c.challenge_message)),
            "origin": c.origin,
        });
        if c.cross_origin {
            client["crossOrigin"] = serde_json::json!(true);
        }
        let client_data_json = serde_json::to_vec(&client).unwrap();
        let mut authenticator_data = sha256(c.rp_id.as_bytes()).to_vec();
        authenticator_data.push(c.flags);
        authenticator_data.extend_from_slice(&c.sign_count.to_be_bytes());
        let mut signed = authenticator_data.clone();
        signed.extend_from_slice(&sha256(&client_data_json));
        let signature = match self {
            Self::Es256(k) => k
                .sign(&SystemRandom::new(), &signed)
                .unwrap()
                .as_ref()
                .to_vec(),
            Self::Ed25519(k) => k.sign(&signed).as_ref().to_vec(),
        };
        Signed {
            authenticator_data,
            client_data_json,
            signature,
        }
    }
}

const MESSAGE: &[u8] =
    b"v1|human-approval|http://example.org/decision/1|satisfied|sha256:00ff|human|approver";

fn check(
    auth: &Authenticator,
    cose_key: &[u8],
    recorded: u32,
    signed: &Signed,
    message: &[u8],
) -> Result<super::Verified, String> {
    verify(
        auth.scheme(),
        &Registered {
            cose_key,
            rp_id: RP_ID,
            origin: ORIGIN,
            recorded_sign_count: recorded,
        },
        &signed.as_assertion(),
        message,
    )
}

/// Assert a refusal whose reason mentions `needle` — so each arm proves the
/// check it names fired, not merely that SOMETHING failed.
fn refused(result: Result<super::Verified, String>, needle: &str) {
    match result {
        Ok(v) => panic!("expected refusal containing '{needle}', got acceptance {v:?}"),
        Err(e) => assert!(
            e.contains(needle),
            "expected refusal containing '{needle}', got '{e}'"
        ),
    }
}

#[test]
fn es256_known_good_assertion_is_accepted() {
    let auth = Authenticator::es256();
    let signed = auth.sign(&Ceremony::for_message(MESSAGE));
    let v = check(&auth, &auth.cose_key(), 0, &signed, MESSAGE).unwrap();
    assert!(v.user_present && v.user_verified);
    assert_eq!(v.sign_count, 7);
}

#[test]
fn eddsa_known_good_assertion_is_accepted_and_wrong_key_refused() {
    let auth = Authenticator::ed25519();
    let signed = auth.sign(&Ceremony::for_message(MESSAGE));
    assert!(check(&auth, &auth.cose_key(), 0, &signed, MESSAGE).is_ok());
    let other = Authenticator::ed25519();
    refused(
        check(&auth, &other.cose_key(), 0, &signed, MESSAGE),
        "signature does not verify",
    );
}

#[test]
fn wrong_challenge_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.challenge_message =
        b"v1|human-approval|http://example.org/decision/2|satisfied|x|human|approver".to_vec();
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "challenge",
    );
}

#[test]
fn a_different_verdict_message_is_refused() {
    // The same valid assertion presented for a verdict with a flipped outcome.
    let auth = Authenticator::es256();
    let signed = auth.sign(&Ceremony::for_message(MESSAGE));
    let flipped =
        b"v1|human-approval|http://example.org/decision/1|unsatisfied|sha256:00ff|human|approver";
    refused(
        check(&auth, &auth.cose_key(), 0, &signed, flipped),
        "challenge",
    );
}

#[test]
fn a_raw_caller_challenge_is_not_accepted_as_truth() {
    // A client that puts the bare message hash (not quipu's domain-separated
    // derivation) in the challenge must not verify.
    let auth = Authenticator::es256();
    let signed = auth.sign(&Ceremony::for_message(MESSAGE));
    let mut client: serde_json::Value = serde_json::from_slice(&signed.client_data_json).unwrap();
    client["challenge"] = serde_json::json!(b64url(&sha256(MESSAGE)));
    // Re-sign honestly over the altered client data so only the challenge differs.
    let client_data_json = serde_json::to_vec(&client).unwrap();
    let mut data = signed.authenticator_data.clone();
    data.extend_from_slice(&sha256(&client_data_json));
    let Authenticator::Es256(k) = &auth else {
        unreachable!()
    };
    let signature = k
        .sign(&SystemRandom::new(), &data)
        .unwrap()
        .as_ref()
        .to_vec();
    let resigned = Signed {
        authenticator_data: signed.authenticator_data,
        client_data_json,
        signature,
    };
    refused(
        check(&auth, &auth.cose_key(), 0, &resigned, MESSAGE),
        "challenge",
    );
}

#[test]
fn wrong_origin_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.origin = "https://evil.example.net".into();
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "origin",
    );
}

#[test]
fn cross_origin_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.cross_origin = true;
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "cross-origin",
    );
}

#[test]
fn wrong_rp_id_hash_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.rp_id = "evil.example.net".into();
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "rpIdHash",
    );
}

#[test]
fn missing_user_presence_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.flags = 0x04; // UV only
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "(UP)",
    );
}

#[test]
fn missing_user_verification_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.flags = 0x01; // UP only
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "(UV)",
    );
}

#[test]
fn wrong_type_is_refused() {
    let auth = Authenticator::es256();
    let mut c = Ceremony::for_message(MESSAGE);
    c.typ = "webauthn.create";
    refused(
        check(&auth, &auth.cose_key(), 0, &auth.sign(&c), MESSAGE),
        "webauthn.get",
    );
}

#[test]
fn tampered_client_data_is_refused() {
    let auth = Authenticator::es256();
    let mut signed = auth.sign(&Ceremony::for_message(MESSAGE));
    // Append a harmless-looking field after signing: every check but the
    // signature still passes, so the signature is what must catch it.
    let mut client: serde_json::Value = serde_json::from_slice(&signed.client_data_json).unwrap();
    client["extra"] = serde_json::json!("x");
    signed.client_data_json = serde_json::to_vec(&client).unwrap();
    refused(
        check(&auth, &auth.cose_key(), 0, &signed, MESSAGE),
        "signature does not verify",
    );
}

#[test]
fn tampered_authenticator_data_is_refused() {
    let auth = Authenticator::es256();
    let mut signed = auth.sign(&Ceremony::for_message(MESSAGE));
    // Bump the counter after signing (flags and rpIdHash still pass).
    signed.authenticator_data[36] ^= 0x01;
    refused(
        check(&auth, &auth.cose_key(), 0, &signed, MESSAGE),
        "signature does not verify",
    );
    // And truncation is refused before anything is parsed.
    let mut short = auth.sign(&Ceremony::for_message(MESSAGE));
    short.authenticator_data.truncate(36);
    refused(
        check(&auth, &auth.cose_key(), 0, &short, MESSAGE),
        "37 bytes",
    );
}

#[test]
fn signature_by_a_different_key_is_refused() {
    let auth = Authenticator::es256();
    let other = Authenticator::es256();
    let signed = other.sign(&Ceremony::for_message(MESSAGE));
    refused(
        check(&auth, &auth.cose_key(), 0, &signed, MESSAGE),
        "signature does not verify",
    );
}

#[test]
fn sign_count_policy_refuses_regression_and_accepts_zero() {
    let auth = Authenticator::es256();
    let signed = auth.sign(&Ceremony::for_message(MESSAGE)); // count 7
    // Recorded 7, presented 7: not an advance -> clone suspected.
    refused(
        check(&auth, &auth.cose_key(), 7, &signed, MESSAGE),
        "counter",
    );
    // Recorded 9, presented 7: regression.
    refused(
        check(&auth, &auth.cose_key(), 9, &signed, MESSAGE),
        "counter",
    );
    // Recorded 6, presented 7: advance, accepted, and 7 is what to record.
    assert_eq!(
        check(&auth, &auth.cose_key(), 6, &signed, MESSAGE)
            .unwrap()
            .sign_count,
        7
    );
    // An authenticator that does not count (synced passkeys report 0).
    let mut c = Ceremony::for_message(MESSAGE);
    c.sign_count = 0;
    assert!(check(&auth, &auth.cose_key(), 9, &auth.sign(&c), MESSAGE).is_ok());
}

#[test]
fn scheme_and_key_algorithm_must_agree() {
    let auth = Authenticator::es256();
    let signed = auth.sign(&Ceremony::for_message(MESSAGE));
    let r = verify(
        Scheme::WebauthnEddsa,
        &Registered {
            cose_key: &auth.cose_key(),
            rp_id: RP_ID,
            origin: ORIGIN,
            recorded_sign_count: 0,
        },
        &signed.as_assertion(),
        MESSAGE,
    );
    refused(r, "does not match scheme");
}

#[test]
fn cose_parser_refuses_non_canonical_or_ambiguous_keys() {
    let auth = Authenticator::es256();
    let good = auth.cose_key();
    assert!(cose::parse(&good).is_ok());

    let mut trailing = good.clone();
    trailing.push(0x00);
    assert!(cose::parse(&trailing).unwrap_err().contains("trailing"));

    // Indefinite-length map.
    let mut indefinite = good.clone();
    indefinite[0] = 0xbf;
    assert!(cose::parse(&indefinite).is_err());

    // Truncated.
    assert!(cose::parse(&good[..good.len() - 1]).is_err());

    // Missing alg: map of 4 without the (3, -7) pair.
    let mut no_alg = vec![0xa4, 0x01, 0x02, 0x20, 0x01];
    no_alg.extend_from_slice(&good[7..]); // x and y entries follow kty/alg/crv
    assert!(cose::parse(&no_alg).unwrap_err().contains("alg"));

    // Duplicate label: kty twice.
    let dup = [0xa2, 0x01, 0x02, 0x01, 0x02];
    assert!(cose::parse(&dup).unwrap_err().contains("duplicate"));

    // An RSA (kty 3, RS256 -257) key is unsupported, not misread.
    let rsa = [0xa3, 0x01, 0x03, 0x03, 0x39, 0x01, 0x00, 0x20, 0x01];
    assert!(cose::parse(&rsa).unwrap_err().contains("unsupported"));
}
