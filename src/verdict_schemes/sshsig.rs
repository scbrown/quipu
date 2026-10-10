//! OpenSSH SSHSIG verification for verdicts (`PROTOCOL.sshsig`, `PROTOCOL.u2f`).
//!
//! A verdict signed with `ssh-keygen -Y sign -n quipu-verdict` over the
//! canonical verdict message. The envelope:
//!
//! ```text
//! "SSHSIG" | uint32 1 | string pubkey | string namespace | string reserved
//!          | string hash_algorithm | string signature
//! ```
//!
//! and the bytes the key signs:
//!
//! ```text
//! "SSHSIG" | string namespace | string reserved | string hash_algorithm
//!          | string H(message)
//! ```
//!
//! For a FIDO `sk-ssh-ed25519@openssh.com` key the authenticator does not sign
//! those bytes directly. Per `PROTOCOL.u2f` it signs
//! `SHA-256(application) | flags | uint32 counter | SHA-256(signed bytes)`, and
//! the signature blob carries the flags and counter after the ed25519
//! signature. Checks, each a refusal:
//!
//! 1. Armor and envelope parse exactly (magic, version 1, no trailing bytes).
//! 2. `namespace == "quipu-verdict"`.
//! 3. `hash_algorithm` is `sha256` or `sha512`.
//! 4. The embedded public key blob is byte-identical to the registered key.
//! 5. The signature type matches the key type.
//! 6. The ed25519 signature verifies over the bytes above.
//! 7. For sk keys: the user-presence flag (0x01) is set, and the counter passes
//!    [`super::check_counter`].
//!
//! The `sshsig-sk-ed25519` scheme accepts ONLY sk keys: a plain `ssh-ed25519`
//! SSHSIG parses and verifies here (that is how the envelope is tested against
//! real `ssh-keygen` output) but is refused as a hardware verdict, because
//! nothing in it proves a device or a touch.

use ring::signature::{ED25519, UnparsedPublicKey};

use super::{SSHSIG_NAMESPACE, Verified, check_counter, sha256};

const MAGIC: &[u8] = b"SSHSIG";
const KEY_ED25519: &str = "ssh-ed25519";
const KEY_SK_ED25519: &str = "sk-ssh-ed25519@openssh.com";
const SK_FLAG_USER_PRESENT: u8 = 0x01;
const SK_FLAG_USER_VERIFIED: u8 = 0x04;

/// The key types this parser understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicKey {
    /// Plain `ssh-ed25519`.
    Ed25519 {
        /// The 32-byte key.
        public: [u8; 32],
    },
    /// FIDO `sk-ssh-ed25519@openssh.com` with its application string.
    SkEd25519 {
        /// The 32-byte key.
        public: [u8; 32],
        /// The FIDO application (RP) string, usually `ssh:`.
        application: Vec<u8>,
    },
}

/// A parsed SSHSIG envelope.
#[derive(Debug, Clone)]
pub struct Envelope {
    /// The signer's public key, SSH wire form.
    pub public_key_blob: Vec<u8>,
    /// The signature namespace (purpose).
    pub namespace: String,
    /// Reserved field, carried into the signed bytes as-is.
    pub reserved: Vec<u8>,
    /// `sha256` or `sha512`: how the message is hashed.
    pub hash_algorithm: String,
    /// The signature, SSH wire form.
    pub signature_blob: Vec<u8>,
}

/// What an SSHSIG verification established, before the hardware policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// Made by a FIDO (sk) key.
    pub is_sk: bool,
    /// sk authenticator flags (0 for plain keys).
    pub flags: u8,
    /// sk authenticator counter (0 for plain keys).
    pub counter: u32,
}

struct Wire<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Wire<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn raw(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or("SSHSIG: truncated")?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.raw(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.raw(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn string(&mut self) -> Result<&'a [u8], String> {
        let n = usize::try_from(self.u32()?).map_err(|_| "SSHSIG: length overflow")?;
        self.raw(n)
    }
    fn utf8(&mut self) -> Result<&'a str, String> {
        std::str::from_utf8(self.string()?).map_err(|_| "SSHSIG: invalid UTF-8".to_string())
    }
    fn end(&self, what: &str) -> Result<(), String> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(format!("SSHSIG: trailing bytes after {what}"))
        }
    }
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    let n = u32::try_from(s.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(s);
}

fn b64_standard(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("SSHSIG: invalid base64: {e}"))
}

/// Parse an armored `-----BEGIN SSH SIGNATURE-----` block.
///
/// # Errors
/// On bad armor, base64 or envelope structure.
pub fn parse_armored(armored: &str) -> Result<Envelope, String> {
    let body = armored
        .trim()
        .strip_prefix("-----BEGIN SSH SIGNATURE-----")
        .and_then(|s| s.strip_suffix("-----END SSH SIGNATURE-----"))
        .ok_or("SSHSIG: missing BEGIN/END SSH SIGNATURE armor")?;
    let b64: String = body.split_whitespace().collect();
    parse_blob(&b64_standard(&b64)?)
}

/// Parse a raw (de-armored) SSHSIG blob.
///
/// # Errors
/// On any structural problem.
pub fn parse_blob(blob: &[u8]) -> Result<Envelope, String> {
    let mut w = Wire::new(blob);
    if w.raw(MAGIC.len())? != MAGIC {
        return Err("SSHSIG: bad magic preamble".into());
    }
    let version = w.u32()?;
    if version != 1 {
        return Err(format!("SSHSIG: unsupported version {version}"));
    }
    let public_key_blob = w.string()?.to_vec();
    let namespace = w.utf8()?.to_string();
    let reserved = w.string()?.to_vec();
    let hash_algorithm = w.utf8()?.to_string();
    let signature_blob = w.string()?.to_vec();
    w.end("the signature envelope")?;
    Ok(Envelope {
        public_key_blob,
        namespace,
        reserved,
        hash_algorithm,
        signature_blob,
    })
}

/// Parse an SSH public key blob (the wire form inside SSHSIG).
///
/// # Errors
/// On unsupported types or malformed blobs.
pub fn parse_public_key_blob(blob: &[u8]) -> Result<PublicKey, String> {
    let mut w = Wire::new(blob);
    let kind = w.utf8()?;
    let key32 = |b: &[u8]| -> Result<[u8; 32], String> {
        b.try_into()
            .map_err(|_| "SSHSIG: ed25519 key is not 32 bytes".to_string())
    };
    let key = match kind {
        KEY_ED25519 => PublicKey::Ed25519 {
            public: key32(w.string()?)?,
        },
        KEY_SK_ED25519 => PublicKey::SkEd25519 {
            public: key32(w.string()?)?,
            application: w.string()?.to_vec(),
        },
        other => return Err(format!("SSHSIG: unsupported key type '{other}'")),
    };
    w.end("the public key")?;
    Ok(key)
}

/// Parse an OpenSSH public key line (`<type> <base64> [comment]`) into its
/// wire blob, checking the leading type token agrees with the blob.
///
/// # Errors
/// On a malformed line or a type mismatch.
pub fn parse_openssh_public_key(line: &str) -> Result<Vec<u8>, String> {
    let mut parts = line.split_whitespace();
    let (Some(kind), Some(b64)) = (parts.next(), parts.next()) else {
        return Err("registered key is not an OpenSSH public key line".into());
    };
    let blob = b64_standard(b64)?;
    let mut w = Wire::new(&blob);
    if w.utf8()? != kind {
        return Err("registered key: type token does not match the key blob".into());
    }
    parse_public_key_blob(&blob)?;
    Ok(blob)
}

/// The bytes the key signs (before any sk wrapping).
fn signed_data(env: &Envelope, message: &[u8]) -> Result<Vec<u8>, String> {
    let hash: Vec<u8> = match env.hash_algorithm.as_str() {
        "sha256" => sha256(message).to_vec(),
        "sha512" => ring::digest::digest(&ring::digest::SHA512, message)
            .as_ref()
            .to_vec(),
        other => return Err(format!("SSHSIG: unsupported hash algorithm '{other}'")),
    };
    let mut out = MAGIC.to_vec();
    put_string(&mut out, env.namespace.as_bytes());
    put_string(&mut out, &env.reserved);
    put_string(&mut out, env.hash_algorithm.as_bytes());
    put_string(&mut out, &hash);
    Ok(out)
}

/// Verify the envelope's cryptography against `registered_key_blob` for
/// `message`: checks 1-6 of the module docs. The hardware policy (sk-only, UP,
/// counter) is applied by [`verify_sk`].
///
/// # Errors
/// A refusal reason naming the failed check.
pub fn verify_envelope(
    env: &Envelope,
    registered_key_blob: &[u8],
    message: &[u8],
) -> Result<Checked, String> {
    if env.namespace != SSHSIG_NAMESPACE {
        return Err(format!(
            "SSHSIG namespace '{}' is not '{SSHSIG_NAMESPACE}'",
            env.namespace
        ));
    }
    if env.public_key_blob != registered_key_blob {
        return Err("SSHSIG was made by a key other than the registered one".into());
    }
    let key = parse_public_key_blob(&env.public_key_blob)?;
    let data = signed_data(env, message)?;

    let mut w = Wire::new(&env.signature_blob);
    let sig_kind = w.utf8()?;
    let sig = w.string()?;
    let verify = |public: &[u8; 32], msg: &[u8]| {
        UnparsedPublicKey::new(&ED25519, public)
            .verify(msg, sig)
            .is_ok()
    };
    match key {
        PublicKey::Ed25519 { public } => {
            if sig_kind != KEY_ED25519 {
                return Err("SSHSIG signature type does not match the key type".into());
            }
            w.end("the signature")?;
            if !verify(&public, &data) {
                return Err("SSHSIG signature does not verify".into());
            }
            Ok(Checked {
                is_sk: false,
                flags: 0,
                counter: 0,
            })
        }
        PublicKey::SkEd25519 {
            public,
            application,
        } => {
            if sig_kind != KEY_SK_ED25519 {
                return Err("SSHSIG signature type does not match the key type".into());
            }
            let flags = w.u8()?;
            let counter = w.u32()?;
            w.end("the signature")?;
            let mut sk_signed = Vec::with_capacity(32 + 1 + 4 + 32);
            sk_signed.extend_from_slice(&sha256(&application));
            sk_signed.push(flags);
            sk_signed.extend_from_slice(&counter.to_be_bytes());
            sk_signed.extend_from_slice(&sha256(&data));
            if !verify(&public, &sk_signed) {
                return Err("SSHSIG signature does not verify".into());
            }
            Ok(Checked {
                is_sk: true,
                flags,
                counter,
            })
        }
    }
}

/// Verify an armored SSHSIG as a `sshsig-sk-ed25519` verdict signature.
///
/// # Errors
/// A refusal reason naming the failed check.
pub fn verify_sk(
    armored: &str,
    registered_openssh_key: &str,
    recorded_counter: u32,
    message: &[u8],
) -> Result<Verified, String> {
    let registered = parse_openssh_public_key(registered_openssh_key)?;
    if !matches!(
        parse_public_key_blob(&registered)?,
        PublicKey::SkEd25519 { .. }
    ) {
        return Err(format!(
            "scheme sshsig-sk-ed25519 requires a registered {KEY_SK_ED25519} key"
        ));
    }
    let env = parse_armored(armored)?;
    let checked = verify_envelope(&env, &registered, message)?;
    if !checked.is_sk {
        return Err("SSHSIG was not made by a hardware (sk) key".into());
    }
    if checked.flags & SK_FLAG_USER_PRESENT == 0 {
        return Err("SSHSIG user-presence flag is not set".into());
    }
    let sign_count = check_counter(recorded_counter, checked.counter)?;
    Ok(Verified {
        user_present: true,
        user_verified: checked.flags & SK_FLAG_USER_VERIFIED != 0,
        sign_count,
    })
}

/// Build an SSHSIG blob (test helper): the inverse of [`parse_blob`].
#[cfg(test)]
pub(crate) fn encode_envelope(env: &Envelope) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&1u32.to_be_bytes());
    put_string(&mut out, &env.public_key_blob);
    put_string(&mut out, env.namespace.as_bytes());
    put_string(&mut out, &env.reserved);
    put_string(&mut out, env.hash_algorithm.as_bytes());
    put_string(&mut out, &env.signature_blob);
    out
}

/// Expose the signed-data construction and wire helpers to the tests.
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) fn signed_data(env: &super::Envelope, message: &[u8]) -> Vec<u8> {
        super::signed_data(env, message).expect("known hash")
    }
    pub(crate) fn put_string(out: &mut Vec<u8>, s: &[u8]) {
        super::put_string(out, s);
    }
}
