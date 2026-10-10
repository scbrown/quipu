//! `WebAuthn` assertion verification for verdicts (W3C `WebAuthn` Level 3 §7.2).
//!
//! The authenticator signs `authenticatorData || SHA-256(clientDataJSON)`, and
//! `clientDataJSON` carries the challenge and origin, so one signature binds
//! the device, the site and the verdict. Every check below is a refusal on
//! failure; there is no "warn and continue".
//!
//! 1. `clientDataJSON` parses as JSON, `type == "webauthn.get"`.
//! 2. `challenge` (base64url) decodes to exactly the challenge QUIPU derives
//!    from the verdict message ([`super::webauthn_challenge`]). The caller
//!    supplies the message fields, never the challenge.
//! 3. `origin` equals the registration's allowed origin, byte for byte, and
//!    `crossOrigin` is not `true`.
//! 4. `authenticatorData` is at least 37 bytes; its first 32 bytes equal
//!    SHA-256 of the registration's RP ID.
//! 5. Flags: UP (0x01) AND UV (0x04) are both set.
//! 6. The signature verifies over `authenticatorData || SHA-256(clientDataJSON)`
//!    under the registered COSE key, with the algorithm the scheme names.
//! 7. The signature counter passes [`super::check_counter`].

use ring::signature::{ECDSA_P256_SHA256_ASN1, ED25519, UnparsedPublicKey};

use super::cose::{self, CoseKey};
use super::{Scheme, Verified, check_counter, sha256};

const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;

/// What the registration pins: the credential key, the relying party and the
/// origin the ceremony must have run on.
#[derive(Debug, Clone)]
pub struct Registered<'a> {
    /// `COSE_Key` bytes of the credential public key.
    pub cose_key: &'a [u8],
    /// The RP ID the credential was created for (a host name).
    pub rp_id: &'a str,
    /// The exact origin allowed, e.g. `https://approve.example.org`.
    pub origin: &'a str,
    /// The highest counter recorded for this credential (0 = none).
    pub recorded_sign_count: u32,
}

/// The three parts of an assertion, already base64url-decoded.
#[derive(Debug, Clone)]
pub struct Assertion<'a> {
    /// Raw `authenticatorData`.
    pub authenticator_data: &'a [u8],
    /// Raw `clientDataJSON` bytes, exactly as the client produced them.
    pub client_data_json: &'a [u8],
    /// The assertion signature (DER for ES256, 64 bytes for `EdDSA`).
    pub signature: &'a [u8],
}

/// Verify one assertion for `message` (the canonical verdict message).
///
/// # Errors
/// A human-readable refusal reason naming the failed check.
pub fn verify(
    scheme: Scheme,
    registered: &Registered<'_>,
    assertion: &Assertion<'_>,
    message: &[u8],
) -> Result<Verified, String> {
    let key = cose::parse(registered.cose_key)?;
    match (scheme, &key) {
        (Scheme::WebauthnEs256, CoseKey::Es256 { .. })
        | (Scheme::WebauthnEddsa, CoseKey::Ed25519 { .. }) => {}
        _ => {
            return Err(format!(
                "registered COSE key algorithm does not match scheme '{}'",
                scheme.tag()
            ));
        }
    }

    // 1-3: client data.
    let client: serde_json::Value = serde_json::from_slice(assertion.client_data_json)
        .map_err(|e| format!("clientDataJSON is not valid JSON: {e}"))?;
    let text = |k: &str| client.get(k).and_then(serde_json::Value::as_str);
    if text("type") != Some("webauthn.get") {
        return Err("clientDataJSON.type is not 'webauthn.get'".into());
    }
    let presented =
        super::b64url_decode(text("challenge").ok_or("clientDataJSON has no string 'challenge'")?)?;
    let expected = super::webauthn_challenge(message);
    if presented.as_slice() != expected.as_slice() {
        return Err("challenge does not match the verdict quipu derived".into());
    }
    if text("origin") != Some(registered.origin) {
        return Err(format!(
            "origin does not match the registered origin '{}'",
            registered.origin
        ));
    }
    if client
        .get("crossOrigin")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Err("cross-origin assertions are refused".into());
    }

    // 4-5: authenticator data.
    let ad = assertion.authenticator_data;
    if ad.len() < 37 {
        return Err("authenticatorData is shorter than 37 bytes".into());
    }
    if ad[..32] != sha256(registered.rp_id.as_bytes()) {
        return Err(format!(
            "rpIdHash does not match the registered RP ID '{}'",
            registered.rp_id
        ));
    }
    let flags = ad[32];
    if flags & FLAG_UP == 0 {
        return Err("user presence (UP) flag is not set".into());
    }
    if flags & FLAG_UV == 0 {
        return Err("user verification (UV) flag is not set".into());
    }
    let sign_count = u32::from_be_bytes([ad[33], ad[34], ad[35], ad[36]]);

    // 6: signature over authenticatorData || SHA-256(clientDataJSON).
    let mut signed = Vec::with_capacity(ad.len() + 32);
    signed.extend_from_slice(ad);
    signed.extend_from_slice(&sha256(assertion.client_data_json));
    let ok = match &key {
        CoseKey::Es256 { sec1_uncompressed } => {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, sec1_uncompressed)
                .verify(&signed, assertion.signature)
                .is_ok()
        }
        CoseKey::Ed25519 { public } => UnparsedPublicKey::new(&ED25519, public)
            .verify(&signed, assertion.signature)
            .is_ok(),
    };
    if !ok {
        return Err("signature does not verify under the registered credential key".into());
    }

    // 7: counter, only after the signature proved the counter is genuine.
    let sign_count = check_counter(registered.recorded_sign_count, sign_count)?;
    Ok(Verified {
        user_present: true,
        user_verified: true,
        sign_count,
    })
}
