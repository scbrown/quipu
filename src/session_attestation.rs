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
use crate::metrics::attestation::{VerificationObservation, VerificationResult as Verdict};
use crate::share::sha256;

pub const WRITE_V1: &str = "quipu-write-v1";
/// v1 plus an AUDIENCE: the receiving store's id, so a write accepted by one
/// quipu cannot be relayed to another that trusts the same key (aegis-72cpbx).
pub const WRITE_V2: &str = "quipu-write-v2";
pub const SHARE_V1: &str = "quipu-share-v1";

/// The one clock window every attestation is checked against, shares and
/// signed HTTP writes alike; the nonce horizon derives from it.
pub const ATTESTATION_SKEW_SECS: u64 = 300;

#[path = "session_attestation_write.rs"]
mod write;
pub use write::{Refusal, body_sha256, check_binding_deferred};

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
    /// Granted to SIGN HTTP WRITES (aegis-bys8d1). Operator-granted only:
    /// `serde(skip)` means it never travels in a share manifest and never
    /// deserializes as anything but `false`, so no producer can grant itself
    /// write by shipping a binding. A binding registered to trust a share
    /// producer is share-only until an operator grants write explicitly.
    #[serde(skip)]
    pub allow_write: bool,
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
            allow_write: false,
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
    /// `quipu-write-v2` only: the store id the client signed for. Absent from
    /// the JSON when `None`, so v1 and share envelopes serialize exactly as they
    /// did before v2 existed (the published v1 vector pins those bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
}

/// Fields uniquely binding one HTTP mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteBinding<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub content_type: &'a str,
    pub body_sha256: &'a str,
    /// `None` selects `quipu-write-v1`. `Some` selects `quipu-write-v2`, and
    /// it must be the RECEIVING store's own id, never the client's claim: it
    /// is what gets signed, so a verifier that took it from the envelope would
    /// sign-check whatever audience the sender chose.
    pub audience: Option<&'a str>,
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
            Self::Write(WriteBinding { audience: None, .. }) => WRITE_V1,
            Self::Write(WriteBinding {
                audience: Some(_), ..
            }) => WRITE_V2,
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
    let mut observation = VerificationObservation::new(payload);
    observation.result = Verdict::Invalid;
    validate_envelope(envelope, payload)?;
    observation.result = Verdict::Error;
    if envelope.issued_at_epoch.abs_diff(now_epoch) > allowed_skew_secs {
        observation.result = Verdict::Skew;
        return Err(Error::InvalidValue(
            "attestation issuance is outside the accepted clock window".into(),
        ));
    }
    // The key_id must be the digest of the key we are about to verify against,
    // or an envelope could name one key and be checked against another.
    let expected = key_id_of(public_key).ok_or_else(|| {
        observation.result = Verdict::Invalid;
        Error::InvalidValue("accompanying public key is not 32-byte lowercase hex".into())
    })?;
    if envelope.key_id != expected {
        observation.result = Verdict::Invalid;
        return Err(Error::InvalidValue(
            "attestation key_id does not match the accompanying public key".into(),
        ));
    }
    if !crate::signing::verify_hex(
        public_key,
        &canonical_message(envelope, payload),
        &envelope.signature,
    ) {
        observation.result = Verdict::Badsig;
        return Err(Error::InvalidValue(
            "attestation signature does not verify against the accompanying public key".into(),
        ));
    }
    observation.result = Verdict::Ok;
    Ok(())
}

pub fn verify_binding<B: AttestationBindings + ?Sized>(
    bindings: &B,
    envelope: &AttestationEnvelope,
    payload: &SignedBinding<'_>,
    now_epoch: u64,
    allowed_skew_secs: u64,
) -> Result<VerifiedPrincipal> {
    let mut observation = VerificationObservation::new(payload);
    let binding = check_binding(
        bindings,
        envelope,
        payload,
        now_epoch,
        allowed_skew_secs,
        &mut observation,
    )?;
    if !bindings.consume_nonce(&binding.session, &envelope.nonce, now_epoch)? {
        observation.result = Verdict::Replay;
        return Err(Error::InvalidValue("attestation nonce replay".into()));
    }
    observation.result = Verdict::Ok;
    Ok(binding.into())
}

fn check_binding<B: AttestationBindings + ?Sized>(
    bindings: &B,
    envelope: &AttestationEnvelope,
    payload: &SignedBinding<'_>,
    now_epoch: u64,
    allowed_skew_secs: u64,
    observation: &mut VerificationObservation,
) -> Result<SessionBinding> {
    observation.result = Verdict::Invalid;
    validate_envelope(envelope, payload)?;
    observation.result = Verdict::Error;
    let binding = bindings.binding(&envelope.session)?.ok_or_else(|| {
        observation.result = Verdict::Unbound;
        Error::InvalidValue("unbound attestation session".into())
    })?;
    if binding.revoked {
        observation.result = Verdict::Revoked;
        return Err(Error::InvalidValue("revoked attestation session".into()));
    }
    if matches!(payload, SignedBinding::Write(_)) && !binding.allow_write {
        observation.result = Verdict::Scope;
        return Err(Error::InvalidValue(
            "session binding is not granted write (quipu attest allow-write)".into(),
        ));
    }
    if now_epoch > binding.expires_at_epoch || now_epoch < binding.issued_at_epoch {
        observation.result = Verdict::Expired;
        return Err(Error::InvalidValue(
            "expired or not-yet-valid session binding".into(),
        ));
    }
    if envelope.key_id != binding.key_id || envelope.introducer != binding.introducer {
        observation.result = Verdict::Invalid;
        return Err(Error::InvalidValue(
            "attestation does not match protected session binding".into(),
        ));
    }
    if envelope.issued_at_epoch.abs_diff(now_epoch) > allowed_skew_secs {
        observation.result = Verdict::Skew;
        return Err(Error::InvalidValue(
            "attestation issuance is outside the accepted clock window".into(),
        ));
    }
    let message = canonical_message(envelope, payload);
    if !crate::signing::verify_hex(&binding.public_key, &message, &envelope.signature) {
        observation.result = Verdict::Badsig;
        return Err(Error::InvalidValue(
            "attestation signature does not verify".into(),
        ));
    }
    Ok(binding)
}

fn validate_envelope(envelope: &AttestationEnvelope, payload: &SignedBinding<'_>) -> Result<()> {
    if envelope.version != payload.version() {
        return Err(Error::InvalidValue(format!(
            "attestation domain mismatch: envelope={} payload={}",
            envelope.version,
            payload.version()
        )));
    }
    // The audience is checked here, before the signature, so a relay gets a
    // verdict that says why (`invalid`) rather than a bare `badsig`. Only v2
    // carries one; anywhere else it would be an unsigned field.
    let expected = match payload {
        SignedBinding::Write(w) => w.audience,
        SignedBinding::Share(_) => None,
    };
    match (expected, envelope.audience.as_deref()) {
        (None, None) => {}
        (None, Some(_)) => {
            return Err(Error::InvalidValue(format!(
                "attestation audience is only signed under {WRITE_V2}"
            )));
        }
        (Some(_), None) => {
            return Err(Error::InvalidValue(format!(
                "{WRITE_V2} attestation carries no audience"
            )));
        }
        (Some(ours), Some(theirs)) if ours != theirs => {
            return Err(Error::InvalidValue(format!(
                "attestation audience mismatch: signed for {theirs}, this store is {ours}"
            )));
        }
        (Some(_), Some(_)) => {}
    }
    // canonical_message is newline-delimited: a field carrying a newline or
    // any other control character could make two different envelopes
    // serialize to the same signed bytes (aegis-bys8d1 S3). Refuse them all.
    let fields: Vec<&str> = match payload {
        SignedBinding::Write(w) => vec![
            w.method,
            w.path,
            w.content_type,
            w.body_sha256,
            w.audience.unwrap_or_default(),
        ],
        SignedBinding::Share(s) => vec![s.share_id, s.graph_hash, s.shapes_hash],
    };
    let common = [
        &envelope.key_id,
        &envelope.session,
        &envelope.introducer,
        &envelope.signature,
    ];
    if common
        .iter()
        .map(|f| f.as_str())
        .chain(fields)
        .any(|f| f.chars().any(char::is_control))
    {
        return Err(Error::InvalidValue(
            "attestation fields must not contain control characters".into(),
        ));
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
        SignedBinding::Write(write) => {
            let mut message = format!(
                "{}\n{common}method={}\npath={}\ncontent_type={}\nbody_sha256={}\n",
                payload.version(),
                write.method,
                write.path,
                write.content_type,
                write.body_sha256
            );
            if let Some(audience) = write.audience {
                message.push_str(&format!("audience={audience}\n"));
            }
            message.into_bytes()
        }
        SignedBinding::Share(share) => format!(
            "{SHARE_V1}\n{common}share_id={}\ngraph_hash={}\nshapes_hash={}\ntx_anchor={}\n",
            share.share_id, share.graph_hash, share.shapes_hash, share.tx_anchor
        )
        .into_bytes(),
    }
}

#[cfg(test)]
#[path = "session_attestation_tests.rs"]
mod tests;
