//! Hardware-backed verdict signature schemes, beside the v1 raw ed25519.
//!
//! v1 (`crate::signing`) verifies a hex ed25519 signature over the canonical
//! verdict message `v1|predicate|target|outcome|evidenceHash|tier|verifier`.
//! That scheme is unchanged and remains the default. This module adds two
//! schemes whose private keys live in hardware and whose signatures carry a
//! user-presence proof, so a human can attest a verdict with a device:
//!
//! | scheme | device | algorithm |
//! |---|---|---|
//! | `webauthn-es256` | passkey / platform authenticator | ECDSA P-256 + SHA-256 (COSE alg -7) |
//! | `webauthn-eddsa` | security key with Ed25519 | Ed25519 (COSE alg -8) |
//! | `sshsig-sk-ed25519` | FIDO security key via OpenSSH | `sk-ssh-ed25519@openssh.com` |
//!
//! The canonical message is the SAME v1 verdict message for every scheme, so
//! what the human approves is exactly what the store records.
//!
//! ## Where the scheme comes from
//!
//! A `aegis:VerifierRegistration` may carry `aegis:signatureScheme`. Absent
//! means `ed25519`, so every registration written before this module keeps
//! its meaning. A verdict names the scheme it was signed with, and it only
//! verifies against a registration declaring that same scheme: the caller
//! cannot pick a weaker scheme for a key registered under a stronger one.
//!
//! ## Why the new schemes are OFF by default
//!
//! `aegis:VerifierRegistration` is still graph-writable: until registry
//! amendments are gated to an enrolled human key, anyone holding the write
//! bearer can register a key. Enabling these schemes does not add a forgery
//! path beyond today's (that writer could equally register an ed25519 key),
//! but a verdict labelled "hardware-backed" invites more trust than the
//! registry can currently justify. So turning them on must be a deliberate
//! operator act: `[quipu.governance] hardware_verdict_schemes = true`. While it
//! is off, a verdict or a registration using one of these schemes is refused.
//!
//! ## Signature-counter policy (clone detection)
//!
//! Both `WebAuthn` and FIDO sk signatures carry an authenticator counter. The
//! verifier takes the highest counter previously recorded for the
//! registration (`aegis:signCount`) and applies `WebAuthn` Level 3 §7.2: if
//! EITHER counter is nonzero, the presented counter must be strictly greater
//! than the recorded one, or the signature is refused because the credential
//! may have been cloned. So once a nonzero counter has been recorded, a
//! presented 0 is refused too: accepting it would tell the recorder to store
//! 0 and silently switch clone detection off for that credential. Only when
//! BOTH are 0 (an authenticator that does not count, such as a synced
//! passkey, from its first use) is the signature accepted without an advance.
//!
//! Verification is read-only: the accepted counter
//! is returned as `sign_count`, and whoever RECORDS the verdict is responsible
//! for persisting it as the new high-water mark. Clone detection is therefore
//! exactly as strong as the counters that have been recorded.

pub mod cose;
pub mod sshsig;
pub mod webauthn;

#[cfg(test)]
pub(crate) mod sshsig_tests;
#[cfg(test)]
pub(crate) mod webauthn_tests;

/// The OpenSSH signature namespace every SSHSIG verdict must use. A signature
/// made for any other purpose (git commits, file signing) is refused.
pub const SSHSIG_NAMESPACE: &str = "quipu-verdict";

/// Domain-separation prefix for the `WebAuthn` challenge. The challenge binds the
/// assertion to "a quipu verdict", not merely to some bytes.
const WEBAUTHN_CHALLENGE_DOMAIN: &[u8] = b"quipu-verdict-webauthn-v1\0";

/// A verdict signature scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// v1: raw ed25519, hex key and hex signature (`crate::signing`).
    Ed25519,
    /// `WebAuthn` assertion, ECDSA P-256 / SHA-256 credential (COSE alg -7).
    WebauthnEs256,
    /// `WebAuthn` assertion, Ed25519 credential (COSE alg -8).
    WebauthnEddsa,
    /// OpenSSH SSHSIG made with an `sk-ssh-ed25519@openssh.com` FIDO key.
    SshsigSkEd25519,
}

impl Scheme {
    /// Parse a scheme tag. Unknown tags are an error, never a fallback.
    ///
    /// # Errors
    /// Returns a message naming the accepted tags.
    pub fn parse(tag: &str) -> Result<Self, String> {
        match tag {
            "ed25519" => Ok(Self::Ed25519),
            "webauthn-es256" => Ok(Self::WebauthnEs256),
            "webauthn-eddsa" => Ok(Self::WebauthnEddsa),
            "sshsig-sk-ed25519" => Ok(Self::SshsigSkEd25519),
            other => Err(format!(
                "unknown signature scheme '{other}' (accepted: ed25519, webauthn-es256, \
                 webauthn-eddsa, sshsig-sk-ed25519)"
            )),
        }
    }

    /// The tag as written in `aegis:signatureScheme` and verdict output.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::WebauthnEs256 => "webauthn-es256",
            Self::WebauthnEddsa => "webauthn-eddsa",
            Self::SshsigSkEd25519 => "sshsig-sk-ed25519",
        }
    }

    /// Whether this is one of the hardware schemes behind the config gate.
    #[must_use]
    pub fn is_hardware(self) -> bool {
        !matches!(self, Self::Ed25519)
    }
}

/// The refusal text for a hardware scheme while the gate is off. One string so
/// the verify path and the registration write gate say the same thing.
#[must_use]
pub fn disabled_message(tag: &str) -> String {
    format!(
        "signature scheme '{tag}' is disabled: hardware verdict schemes are off by default \
         until verifier-registry amendments are restricted to an enrolled human key. \
         An operator enables them deliberately with \
         [quipu.governance] hardware_verdict_schemes = true"
    )
}

/// The `WebAuthn` challenge quipu expects for a verdict message: SHA-256 over a
/// domain tag and the canonical message. Quipu DERIVES this; a challenge the
/// caller supplies is never taken as truth, only compared against this.
#[must_use]
pub fn webauthn_challenge(message: &[u8]) -> [u8; 32] {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    ctx.update(WEBAUTHN_CHALLENGE_DOMAIN);
    ctx.update(message);
    let mut out = [0u8; 32];
    out.copy_from_slice(ctx.finish().as_ref());
    out
}

/// The counter policy described in the module docs. `Ok` carries the counter
/// to record; `Err` is the refusal reason.
///
/// | recorded | presented | result |
/// |---|---|---|
/// | 0 | 0 | accept (authenticator does not count) |
/// | 0 | n > 0 | accept |
/// | r > 0 | 0 | refuse |
/// | r > 0 | n <= r | refuse |
/// | r > 0 | n > r | accept |
///
/// # Errors
/// When a counter has been recorded (`recorded > 0`) and `presented <= recorded`.
pub fn check_counter(recorded: u32, presented: u32) -> Result<u32, String> {
    if recorded != 0 && presented <= recorded {
        return Err(format!(
            "signature counter did not advance (recorded {recorded}, presented {presented}): \
             the credential may be cloned"
        ));
    }
    Ok(presented)
}

/// What a successful hardware-scheme verification established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// User presence (touch) was asserted by the authenticator. Always true on
    /// success: every hardware scheme requires it.
    pub user_present: bool,
    /// User verification (biometric / PIN) was asserted.
    pub user_verified: bool,
    /// The authenticator counter from the signature, to be recorded.
    pub sign_count: u32,
}

/// Decode unpadded or padded base64url (the `WebAuthn` wire encoding).
///
/// # Errors
/// On any character outside the base64url alphabet.
pub fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .map_err(|e| format!("invalid base64url: {e}"))
}

/// SHA-256 as a fixed array.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, data).as_ref());
    out
}
