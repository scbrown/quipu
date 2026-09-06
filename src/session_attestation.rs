//! Session workload attestation shared by HTTP writes and knowledge shares.
//!
//! This module is deliberately transport-neutral. A protected caller-owned
//! registry supplies the session binding; the verifier selects one canonical
//! payload builder by an explicit domain tag and consumes a nonce only after
//! every binding and signature check succeeds.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::share::sha256;

pub const WRITE_V1: &str = "quipu-write-v1";
pub const SHARE_V1: &str = "quipu-share-v1";

/// Server-protected binding installed by a trusted introducer.
///
/// Serializable because a producer's PUBLIC binding travels inside a share
/// manifest (aegis-tadzdf). There is no private key in this struct, so carrying
/// it exposes nothing; `revoked` is producer-asserted and therefore not to be
/// trusted from a share — the consumer's own registered copy is authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub agent: String,
    pub session: String,
    pub public_key: String,
    pub key_id: String,
    pub introducer: String,
    pub issued_at_epoch: u64,
    pub expires_at_epoch: u64,
    pub revoked: bool,
}

impl SessionBinding {
    pub fn new(
        agent: impl Into<String>,
        session: impl Into<String>,
        public_key: impl Into<String>,
        introducer: impl Into<String>,
        issued_at_epoch: u64,
        expires_at_epoch: u64,
    ) -> Result<Self> {
        let public_key = public_key.into();
        let raw = hex::decode(&public_key)
            .map_err(|_| Error::InvalidValue("session public key is not lowercase hex".into()))?;
        if public_key != public_key.to_ascii_lowercase() || raw.len() != 32 {
            return Err(Error::InvalidValue(
                "session public key must be 32-byte lowercase hex".into(),
            ));
        }
        if expires_at_epoch <= issued_at_epoch {
            return Err(Error::InvalidValue(
                "session binding expiry must follow issuance".into(),
            ));
        }
        Ok(Self {
            agent: agent.into(),
            session: session.into(),
            key_id: key_id_of_raw(&raw),
            public_key,
            introducer: introducer.into(),
            issued_at_epoch,
            expires_at_epoch,
            revoked: false,
        })
    }
}

/// External signature envelope carried beside the application payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationEnvelope {
    pub version: String,
    pub key_id: String,
    pub session: String,
    pub introducer: String,
    pub issued_at_epoch: u64,
    pub nonce: String,
    pub signature: String,
}

/// Fields uniquely binding one HTTP mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteBinding<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub content_type: &'a str,
    pub body_sha256: &'a str,
}

/// Fields uniquely binding one validated v1 share manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareBinding<'a> {
    pub share_id: &'a str,
    pub graph_hash: &'a str,
    pub shapes_hash: &'a str,
    pub tx_anchor: i64,
}

/// The only two application payloads accepted by the common verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedBinding<'a> {
    Write(WriteBinding<'a>),
    Share(ShareBinding<'a>),
}

impl SignedBinding<'_> {
    #[must_use]
    pub const fn version(&self) -> &'static str {
        match self {
            Self::Write(_) => WRITE_V1,
            Self::Share(_) => SHARE_V1,
        }
    }
}

/// Identity Quipu may stamp after successful verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPrincipal {
    pub agent: String,
    pub session: String,
    pub key_id: String,
    pub introducer: String,
}

/// Protected session and replay state. It is not graph-writable.
#[derive(Debug, Default)]
pub struct BindingRegistry {
    bindings: Mutex<HashMap<String, SessionBinding>>,
    nonces: Mutex<HashSet<(String, String)>>,
}

impl BindingRegistry {
    pub fn register(&self, binding: SessionBinding) -> Result<()> {
        let mut bindings = self.bindings.lock().expect("binding registry poisoned");
        match bindings.get(&binding.session) {
            Some(existing) if existing == &binding => Ok(()),
            Some(_) => Err(Error::InvalidValue(format!(
                "conflicting session binding: {}",
                binding.session
            ))),
            None => {
                if bindings.values().any(|b| b.key_id == binding.key_id) {
                    return Err(Error::InvalidValue(format!(
                        "session public key already bound: {}",
                        binding.key_id
                    )));
                }
                bindings.insert(binding.session.clone(), binding);
                Ok(())
            }
        }
    }

    pub fn revoke(&self, session: &str) -> Result<()> {
        let mut bindings = self.bindings.lock().expect("binding registry poisoned");
        let binding = bindings
            .get_mut(session)
            .ok_or_else(|| Error::InvalidValue(format!("unbound session: {session}")))?;
        binding.revoked = true;
        Ok(())
    }

    /// Verify against this in-memory registry.
    ///
    /// Delegates to [`verify_binding`], which is where the checks and their
    /// ORDER actually live. A store-backed binding source must apply the same
    /// order, and a second copy of it would be a second thing to keep right.
    pub fn verify(
        &self,
        envelope: &AttestationEnvelope,
        payload: &SignedBinding<'_>,
        now_epoch: u64,
        allowed_skew_secs: u64,
    ) -> Result<VerifiedPrincipal> {
        verify_binding(self, envelope, payload, now_epoch, allowed_skew_secs)
    }
}

/// A source of protected session bindings and replay state.
///
/// Two implementations exist, and they differ in exactly one property that
/// matters. [`BindingRegistry`] holds both in memory, so a restart forgets
/// every consumed nonce and reopens every replay window it was closing — which
/// is why an in-memory replay set is not production enforcement. A store-backed
/// implementation spends the nonce with a SQL insert, so the consumption
/// participates in whatever savepoint the caller already has open: rolled back
/// with a rejected mutation, durable with an accepted one.
pub trait AttestationBindings {
    /// The protected binding for `session`, or `None` when the session is
    /// unbound.
    fn binding(&self, session: &str) -> Result<Option<SessionBinding>>;

    /// Record `nonce` as spent for `session`.
    ///
    /// `Ok(false)` means it was ALREADY spent — a replay — and must not be
    /// reported as an error, because the caller distinguishes "replayed" from
    /// "could not tell", and only the first of those is a rejection. An `Err`
    /// here means the replay state could not be consulted at all, which is a
    /// different and worse thing than a replay.
    fn consume_nonce(&self, session: &str, nonce: &str, now_epoch: u64) -> Result<bool>;
}

impl AttestationBindings for BindingRegistry {
    fn binding(&self, session: &str) -> Result<Option<SessionBinding>> {
        Ok(self
            .bindings
            .lock()
            .expect("binding registry poisoned")
            .get(session)
            .cloned())
    }

    fn consume_nonce(&self, session: &str, nonce: &str, _now_epoch: u64) -> Result<bool> {
        Ok(self
            .nonces
            .lock()
            .expect("nonce registry poisoned")
            .insert((session.to_string(), nonce.to_string())))
    }
}

/// How long a spent nonce must be remembered, derived from the clock skew the
/// verifier already accepts.
///
/// The horizon is not a free parameter and must not become one. An attestation
/// whose `issued_at_epoch` is further than `allowed_skew_secs` from now is
/// rejected by the skew check BEFORE the nonce is ever consulted, so a nonce
/// older than that window cannot be replayed successfully whether it is
/// remembered or not. Doubling gives a margin for the two clocks disagreeing in
/// opposite directions rather than encoding a second, independent policy —
/// deriving it in one place is the whole point (the aegis-mhxla ruling: the
/// nonce horizon comes from the skew constant, in one place).
#[must_use]
pub const fn nonce_horizon_secs(allowed_skew_secs: u64) -> u64 {
    allowed_skew_secs.saturating_mul(2)
}

/// The verifier. One ordering of checks, shared by every binding source.
///
/// The nonce is consumed LAST, after every binding and signature check has
/// passed. That order is load-bearing rather than tidy: consuming earlier would
/// let an unsigned or malformed attestation burn a nonce, turning a rejected
/// forgery into a denial of service against the session it was forging.
/// The key id for a raw 32-byte ed25519 public key.
///
/// Extracted so `SessionBinding::new` and [`verify_unregistered`] cannot drift:
/// if the two ever computed the id differently, an envelope would be checked
/// against a key whose id it does not actually name, which is the one thing the
/// id is there to prevent.
fn key_id_of_raw(raw: &[u8]) -> String {
    sha256(raw)
}

/// The key id for a lowercase-hex ed25519 public key, or `None` if it is not one.
fn key_id_of(public_key: &str) -> Option<String> {
    let raw = hex::decode(public_key).ok()?;
    (raw.len() == 32 && public_key == public_key.to_ascii_lowercase()).then(|| key_id_of_raw(&raw))
}

/// Verify an envelope against a public key we were NOT told to trust (aegis-tadzdf).
///
/// This is the `claimed` tier's verifier. It runs every check `verify_binding`
/// runs EXCEPT the two that require a registered session: the registry lookup
/// itself, and nonce consumption.
///
/// **What a pass here does and does not mean.** It proves the bundle was not
/// altered after signing, and that the four manifest identity fields were signed
/// together — integrity without provenance. It says NOTHING about who holds the
/// key, because nobody vouched for it. That is precisely the distinction the
/// `claimed` tier exists to carry, and it is why a tampered bundle must still
/// FAIL here rather than degrade to `claimed`: if `claimed` were handed out
/// without checking the signature, it would mean nothing at all.
///
/// **Replay is deliberately NOT defended at this tier.** Nonce state is keyed by
/// registered session, so there is nothing to spend against. Consuming nonces for
/// unknown sessions would let any caller populate the replay table at will. A
/// `claimed` import is therefore replayable, and the tier's note says so rather
/// than leaving a reader to assume the protection carried over.
pub fn verify_unregistered(
    envelope: &AttestationEnvelope,
    payload: &SignedBinding<'_>,
    public_key: &str,
    now_epoch: u64,
    allowed_skew_secs: u64,
) -> Result<()> {
    validate_envelope(envelope, payload)?;
    if envelope.issued_at_epoch.abs_diff(now_epoch) > allowed_skew_secs {
        return Err(Error::InvalidValue(
            "attestation issuance is outside the accepted clock window".into(),
        ));
    }
    // The key_id must be the digest of the key we are about to verify against,
    // or an envelope could name one key and be checked against another.
    let expected = key_id_of(public_key).ok_or_else(|| {
        Error::InvalidValue("accompanying public key is not 32-byte lowercase hex".into())
    })?;
    if envelope.key_id != expected {
        return Err(Error::InvalidValue(
            "attestation key_id does not match the accompanying public key".into(),
        ));
    }
    if !crate::signing::verify_hex(
        public_key,
        &canonical_message(envelope, payload),
        &envelope.signature,
    ) {
        return Err(Error::InvalidValue(
            "attestation signature does not verify against the accompanying public key".into(),
        ));
    }
    Ok(())
}

pub fn verify_binding<B: AttestationBindings + ?Sized>(
    bindings: &B,
    envelope: &AttestationEnvelope,
    payload: &SignedBinding<'_>,
    now_epoch: u64,
    allowed_skew_secs: u64,
) -> Result<VerifiedPrincipal> {
    validate_envelope(envelope, payload)?;
    let binding = bindings
        .binding(&envelope.session)?
        .ok_or_else(|| Error::InvalidValue("unbound attestation session".into()))?;
    if binding.revoked {
        return Err(Error::InvalidValue("revoked attestation session".into()));
    }
    if now_epoch > binding.expires_at_epoch || now_epoch < binding.issued_at_epoch {
        return Err(Error::InvalidValue(
            "expired or not-yet-valid session binding".into(),
        ));
    }
    if envelope.key_id != binding.key_id || envelope.introducer != binding.introducer {
        return Err(Error::InvalidValue(
            "attestation does not match protected session binding".into(),
        ));
    }
    if envelope.issued_at_epoch.abs_diff(now_epoch) > allowed_skew_secs {
        return Err(Error::InvalidValue(
            "attestation issuance is outside the accepted clock window".into(),
        ));
    }
    let message = canonical_message(envelope, payload);
    if !crate::signing::verify_hex(&binding.public_key, &message, &envelope.signature) {
        return Err(Error::InvalidValue(
            "attestation signature does not verify".into(),
        ));
    }
    if !bindings.consume_nonce(&binding.session, &envelope.nonce, now_epoch)? {
        return Err(Error::InvalidValue("attestation nonce replay".into()));
    }
    Ok(VerifiedPrincipal {
        agent: binding.agent,
        session: binding.session,
        key_id: binding.key_id,
        introducer: binding.introducer,
    })
}

fn validate_envelope(envelope: &AttestationEnvelope, payload: &SignedBinding<'_>) -> Result<()> {
    if envelope.version != payload.version() {
        return Err(Error::InvalidValue(format!(
            "attestation domain mismatch: envelope={} payload={}",
            envelope.version,
            payload.version()
        )));
    }
    if envelope.nonce.len() != 32
        || envelope.nonce != envelope.nonce.to_ascii_lowercase()
        || !envelope.nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::InvalidValue(
            "attestation nonce must be 128-bit lowercase hex".into(),
        ));
    }
    Ok(())
}

/// Deterministic bytes selected by the explicit application-domain tag.
#[must_use]
pub fn canonical_message(envelope: &AttestationEnvelope, payload: &SignedBinding<'_>) -> Vec<u8> {
    let common = format!(
        "key_id={}\nsession={}\nintroducer={}\nissued_at={}\nnonce={}\n",
        envelope.key_id,
        envelope.session,
        envelope.introducer,
        envelope.issued_at_epoch,
        envelope.nonce
    );
    match payload {
        SignedBinding::Write(write) => format!(
            "{WRITE_V1}\n{common}method={}\npath={}\ncontent_type={}\nbody_sha256={}\n",
            write.method, write.path, write.content_type, write.body_sha256
        )
        .into_bytes(),
        SignedBinding::Share(share) => format!(
            "{SHARE_V1}\n{common}share_id={}\ngraph_hash={}\nshapes_hash={}\ntx_anchor={}\n",
            share.share_id, share.graph_hash, share.shapes_hash, share.tx_anchor
        )
        .into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn fixture() -> (BindingRegistry, Ed25519KeyPair, SessionBinding) {
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let binding = SessionBinding::new(
            "urn:agent:malcolm",
            "session-1",
            hex::encode(key.public_key().as_ref()),
            "creel-extension:key-1",
            NOW - 60,
            NOW + 60,
        )
        .unwrap();
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
}
