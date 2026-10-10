//! The signed-HTTP-write half of the session attestation verifier
//! (aegis-bys8d1): every check except the nonce spend, with the verdict a
//! client can act on. A child module so it reuses the parent's private
//! `check_binding` core rather than a second copy of the checks.

use super::{
    AttestationBindings, AttestationEnvelope, SessionBinding, SignedBinding, Verdict,
    VerificationObservation, VerifiedPrincipal, check_binding,
};

/// Lowercase hex SHA-256 of a request body, as a `WriteBinding` binds it:
/// 64 hex characters, NO `sha256:` prefix (the published test vector pins it).
#[must_use]
pub fn body_sha256(body: &[u8]) -> String {
    hex::encode(ring::digest::digest(&ring::digest::SHA256, body).as_ref())
}

/// A refused attestation, with the verdict a client can act on (aegis-bys8d1 S4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Wire code: `badsig`, `skew`, `replay`, `unbound`, `revoked`, `expired`,
    /// `invalid` or `error`. The same vocabulary as the verification metric.
    pub verdict: &'static str,
    pub message: String,
}

/// Every check EXCEPT spending the nonce: for a caller that spends it later,
/// inside the store work it authorises (signed HTTP writes, aegis-bys8d1).
pub fn check_binding_deferred<B: AttestationBindings + ?Sized>(
    bindings: &B,
    envelope: &AttestationEnvelope,
    payload: &SignedBinding<'_>,
    now_epoch: u64,
    allowed_skew_secs: u64,
) -> std::result::Result<VerifiedPrincipal, Refusal> {
    let mut observation = VerificationObservation::new(payload);
    match check_binding(
        bindings,
        envelope,
        payload,
        now_epoch,
        allowed_skew_secs,
        &mut observation,
    ) {
        Ok(binding) => {
            observation.result = Verdict::Ok;
            Ok(binding.into())
        }
        Err(e) => Err(Refusal {
            verdict: observation.result.label(),
            message: e.to_string(),
        }),
    }
}

impl From<SessionBinding> for VerifiedPrincipal {
    fn from(binding: SessionBinding) -> Self {
        Self {
            agent: binding.agent,
            session: binding.session,
            key_id: binding.key_id,
            introducer: binding.introducer,
        }
    }
}
