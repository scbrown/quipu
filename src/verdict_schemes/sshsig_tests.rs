//! SSHSIG verification.
//!
//! (a) REAL OpenSSH output: the fixtures below were produced by
//!     `ssh-keygen -Y sign -f <key> -n <namespace> msg` (OpenSSH 10.2) with a
//!     throwaway plain `ssh-ed25519` key, and cross-checked with
//!     `ssh-keygen -Y verify` before being pasted here. They pin the envelope
//!     parser, the signed-data construction and the namespace check to what
//!     OpenSSH actually emits (which hashes with sha512).
//! (b) `sk-ssh-ed25519@openssh.com`: no FIDO hardware or software sk provider
//!     is available in CI, so the sk signature is built in-test with ring over
//!     the exact `PROTOCOL.u2f` digest layout
//!     `SHA256(application) || flags || uint32 counter || SHA256(signed data)`.

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair as _};

use super::sshsig::test_support::{put_string, signed_data};
use super::sshsig::{
    Envelope, encode_envelope, parse_armored, parse_openssh_public_key, verify_envelope, verify_sk,
};
use super::{SSHSIG_NAMESPACE, sha256};

/// The message both real fixtures sign (a canonical v1 verdict message).
pub(crate) const MESSAGE: &[u8] =
    b"v1|human-approval|http://example.org/decision/1|satisfied|sha256:00ff|human|approver";

const REAL_PUBKEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIV1qVwFUGVYSPLFV8WP0wH02TFEkxDNkl0+wzvZYSW4 quipu-test-fixture";

/// `ssh-keygen -Y sign -n quipu-verdict`
const REAL_SIG_QUIPU: &str = "-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAghXWpXAVQZVhI8sVXxY/TAfTZMU
STEM2SXT7DO9lhJbgAAAANcXVpcHUtdmVyZGljdAAAAAAAAAAGc2hhNTEyAAAAUwAAAAtz
c2gtZWQyNTUxOQAAAECMg0hz0LwA/VxUtu4XYqMWN4LgOgc+aKzKzh0lLc+U7Qskk/AM/s
+x7eEG4NqmoW4NQ+jLehmo8Ov+Npf75EwA
-----END SSH SIGNATURE-----
";

/// `ssh-keygen -Y sign -n other-namespace`, same key and message.
const REAL_SIG_OTHER_NAMESPACE: &str = "-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAghXWpXAVQZVhI8sVXxY/TAfTZMU
STEM2SXT7DO9lhJbgAAAAPb3RoZXItbmFtZXNwYWNlAAAAAAAAAAZzaGE1MTIAAABTAAAA
C3NzaC1lZDI1NTE5AAAAQNzyTxAx5SGqSRGTiJui/Yh7ZGY/OXMS9Bmj2tYBqzbZvc7dG5
sg3XR7OhKxVajOPva/J+gDYGa3e8fZrRjLWQc=
-----END SSH SIGNATURE-----
";

// ── (a) real OpenSSH fixtures ───────────────────────────────────────────

#[test]
fn real_openssh_signature_parses_and_verifies() {
    let key = parse_openssh_public_key(REAL_PUBKEY).unwrap();
    let env = parse_armored(REAL_SIG_QUIPU).unwrap();
    assert_eq!(env.namespace, SSHSIG_NAMESPACE);
    assert_eq!(env.hash_algorithm, "sha512");
    let checked = verify_envelope(&env, &key, MESSAGE).unwrap();
    assert!(!checked.is_sk);
}

#[test]
fn real_openssh_signature_for_another_namespace_is_refused() {
    let key = parse_openssh_public_key(REAL_PUBKEY).unwrap();
    let env = parse_armored(REAL_SIG_OTHER_NAMESPACE).unwrap();
    let err = verify_envelope(&env, &key, MESSAGE).unwrap_err();
    assert!(err.contains("namespace"), "{err}");
}

#[test]
fn real_openssh_signature_over_a_tampered_message_is_refused() {
    let key = parse_openssh_public_key(REAL_PUBKEY).unwrap();
    let env = parse_armored(REAL_SIG_QUIPU).unwrap();
    let tampered =
        b"v1|human-approval|http://example.org/decision/1|unsatisfied|sha256:00ff|human|approver";
    let err = verify_envelope(&env, &key, tampered).unwrap_err();
    assert!(err.contains("does not verify"), "{err}");
}

#[test]
fn real_plain_key_signature_is_not_a_hardware_verdict() {
    // Cryptographically valid, but nothing in it proves a device or a touch.
    let err = verify_sk(REAL_SIG_QUIPU, REAL_PUBKEY, 0, MESSAGE).unwrap_err();
    assert!(err.contains("requires a registered"), "{err}");
}

#[test]
fn armor_and_envelope_damage_is_refused() {
    assert!(parse_armored("not a signature").is_err());
    let mut env_bytes = {
        let env = parse_armored(REAL_SIG_QUIPU).unwrap();
        encode_envelope(&env)
    };
    env_bytes.push(0);
    assert!(
        super::sshsig::parse_blob(&env_bytes)
            .unwrap_err()
            .contains("trailing")
    );
    env_bytes.pop();
    env_bytes[0] = b'X';
    assert!(
        super::sshsig::parse_blob(&env_bytes)
            .unwrap_err()
            .contains("magic")
    );
}

// ── (b) hand-built sk-ssh-ed25519 signatures ────────────────────────────

const SK_TYPE: &str = "sk-ssh-ed25519@openssh.com";

pub(crate) struct SkKey {
    kp: Ed25519KeyPair,
    application: Vec<u8>,
}

impl SkKey {
    pub(crate) fn new() -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Self {
            kp: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(),
            application: b"ssh:".to_vec(),
        }
    }

    fn blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_string(&mut out, SK_TYPE.as_bytes());
        put_string(&mut out, self.kp.public_key().as_ref());
        put_string(&mut out, &self.application);
        out
    }

    /// The OpenSSH public key line a human would register.
    pub(crate) fn openssh_line(&self) -> String {
        use base64::Engine as _;
        format!(
            "{SK_TYPE} {} test-key",
            base64::engine::general_purpose::STANDARD.encode(self.blob())
        )
    }

    /// An armored SSHSIG exactly as OpenSSH would emit it for this key.
    pub(crate) fn sign(&self, namespace: &str, message: &[u8], flags: u8, counter: u32) -> String {
        let mut env = Envelope {
            public_key_blob: self.blob(),
            namespace: namespace.into(),
            reserved: Vec::new(),
            hash_algorithm: "sha512".into(),
            signature_blob: Vec::new(),
        };
        let data = signed_data(&env, message);
        let mut sk_signed = sha256(&self.application).to_vec();
        sk_signed.push(flags);
        sk_signed.extend_from_slice(&counter.to_be_bytes());
        sk_signed.extend_from_slice(&sha256(&data));
        let sig = self.kp.sign(&sk_signed);
        let mut blob = Vec::new();
        put_string(&mut blob, SK_TYPE.as_bytes());
        put_string(&mut blob, sig.as_ref());
        blob.push(flags);
        blob.extend_from_slice(&counter.to_be_bytes());
        env.signature_blob = blob;
        armor(&encode_envelope(&env))
    }
}

pub(crate) fn armor(blob: &[u8]) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(blob);
    let lines: Vec<&str> = b64
        .as_bytes()
        .chunks(70)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
        lines.join("\n")
    )
}

/// Rewrite one field of an armored sk signature's trailing flags/counter
/// WITHOUT re-signing.
fn with_signature_tail(armored: &str, flags: u8) -> String {
    let env = parse_armored(armored).unwrap();
    let mut env2 = env.clone();
    let n = env2.signature_blob.len();
    env2.signature_blob[n - 5] = flags;
    armor(&encode_envelope(&env2))
}

#[test]
fn sk_signature_with_user_presence_is_accepted() {
    let key = SkKey::new();
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 42);
    let v = verify_sk(&sig, &key.openssh_line(), 0, MESSAGE).unwrap();
    assert!(v.user_present);
    assert!(!v.user_verified);
    assert_eq!(v.sign_count, 42);

    // verify-required keys also set UV (0x04), which is reported.
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x05, 43);
    assert!(
        verify_sk(&sig, &key.openssh_line(), 42, MESSAGE)
            .unwrap()
            .user_verified
    );
}

#[test]
fn sk_wrong_namespace_is_refused() {
    let key = SkKey::new();
    let sig = key.sign("git", MESSAGE, 0x01, 1);
    let err = verify_sk(&sig, &key.openssh_line(), 0, MESSAGE).unwrap_err();
    assert!(err.contains("namespace"), "{err}");
}

#[test]
fn sk_without_user_presence_is_refused() {
    let key = SkKey::new();
    // Properly signed by the key, but the authenticator did not assert UP.
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x04, 1);
    let err = verify_sk(&sig, &key.openssh_line(), 0, MESSAGE).unwrap_err();
    assert!(err.contains("user-presence"), "{err}");
}

#[test]
fn sk_flags_forged_after_signing_are_refused() {
    let key = SkKey::new();
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x00, 1);
    let forged = with_signature_tail(&sig, 0x01);
    let err = verify_sk(&forged, &key.openssh_line(), 0, MESSAGE).unwrap_err();
    assert!(err.contains("does not verify"), "{err}");
}

#[test]
fn sk_tampered_message_is_refused() {
    let key = SkKey::new();
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 1);
    let tampered =
        b"v1|human-approval|http://example.org/decision/1|unsatisfied|sha256:00ff|human|approver";
    let err = verify_sk(&sig, &key.openssh_line(), 0, tampered).unwrap_err();
    assert!(err.contains("does not verify"), "{err}");
}

#[test]
fn sk_wrong_key_is_refused() {
    let registered = SkKey::new();
    let other = SkKey::new();
    // A valid signature by another key: its embedded key is not the registered one.
    let sig = other.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 1);
    let err = verify_sk(&sig, &registered.openssh_line(), 0, MESSAGE).unwrap_err();
    assert!(err.contains("other than the registered"), "{err}");

    // And an envelope that CLAIMS the registered key but was signed by another.
    let mut env = parse_armored(&sig).unwrap();
    let reg_env = parse_armored(&registered.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 1)).unwrap();
    env.public_key_blob = reg_env.public_key_blob;
    let err = verify_sk(
        &armor(&encode_envelope(&env)),
        &registered.openssh_line(),
        0,
        MESSAGE,
    )
    .unwrap_err();
    assert!(err.contains("does not verify"), "{err}");
}

#[test]
fn sk_counter_regression_is_refused() {
    let key = SkKey::new();
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 5);
    let err = verify_sk(&sig, &key.openssh_line(), 5, MESSAGE).unwrap_err();
    assert!(err.contains("counter"), "{err}");
    assert!(verify_sk(&sig, &key.openssh_line(), 4, MESSAGE).is_ok());
    // A key whose recorded counter is nonzero may not fall back to 0.
    let zero = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 0);
    let err = verify_sk(&zero, &key.openssh_line(), 5, MESSAGE).unwrap_err();
    assert!(err.contains("counter"), "{err}");
    assert!(verify_sk(&zero, &key.openssh_line(), 0, MESSAGE).is_ok());
}

#[test]
fn sk_weak_hash_algorithm_is_refused() {
    let key = SkKey::new();
    let sig = key.sign(SSHSIG_NAMESPACE, MESSAGE, 0x01, 1);
    let mut env = parse_armored(&sig).unwrap();
    env.hash_algorithm = "sha1".into();
    let err = verify_sk(
        &armor(&encode_envelope(&env)),
        &key.openssh_line(),
        0,
        MESSAGE,
    )
    .unwrap_err();
    assert!(err.contains("hash algorithm"), "{err}");
}

#[test]
fn registered_key_line_must_be_self_consistent() {
    let key = SkKey::new();
    let line = key.openssh_line().replacen(SK_TYPE, "ssh-ed25519", 1);
    assert!(parse_openssh_public_key(&line).is_err());
}
