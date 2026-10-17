//! Detached, externally authorized history-coverage attestations.
//!
//! This verifies signed terms and retained review bytes only. It neither issues
//! certificates nor opens/validates a raw store. Artifact integrity, lineage,
//! actual transaction retention and provider closure remain caller obligations.
//! No evaluator accepts this receipt as a source-admission certificate.

use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"quipu-history-source/v1\0";

/// Exact coverage authorized by a separately reviewed external grant.
///
/// Coverage includes supporting facts originating before the window, not just
/// records created within it. Catalogue and premise graphs are both required.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCoverage {
    /// Native store lineage; this alone is not completeness authority.
    pub source_store_id: String,
    /// Graph carrying the transaction-pinned edition catalogue.
    pub catalogue_graph: String,
    /// Graph carrying the transaction/valid-time-pinned premises.
    pub premise_graph: String,
    /// Inclusive first retained transaction covered by the review.
    pub transaction_first: i64,
    /// Inclusive last retained transaction covered by the review.
    pub transaction_last: i64,
    /// Inclusive valid-time lower bound, UTC Unix seconds.
    pub valid_time_from: i64,
    /// Exclusive valid-time upper bound, UTC Unix seconds.
    pub valid_time_until: i64,
}

/// Supported artifact profile. A live database with a separate WAL is not this.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryArtifactKind {
    /// Consistent retained raw database, including all committed history.
    RawSqliteSnapshot,
}

/// Bounded provider profile; not permission to detach configured attachments.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryProviderScope {
    /// All required facts are native, with no attached providers.
    NativeOnly,
}

/// Versioned signed terms. All times are whole UTC Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryAttestation {
    /// Must equal one; a different format needs its own signature domain.
    pub schema_version: u32,
    /// Identity matched against the external grant, never a discovered key.
    pub issuer_id: String,
    /// Exact externally reviewed policy bytes authorizing this grant.
    pub grant_policy_sha256: String,
    /// Retained review evidence identity.
    pub reviewed_evidence_ref: String,
    /// Digest of the actual retained review bytes.
    pub reviewed_evidence_sha256: String,
    /// Digest of the consistent retained raw artifact.
    pub raw_artifact_sha256: String,
    /// Explicit supported raw artifact format.
    pub artifact_kind: HistoryArtifactKind,
    /// Explicit supported provider closure.
    pub provider_scope: HistoryProviderScope,
    /// Both catalogue and premise history, including pre-window supports.
    pub coverage: HistoryCoverage,
    /// First admission instant, inclusive.
    pub issued_at: i64,
    /// Fresh-admission expiry instant, exclusive.
    pub expires_at: i64,
}

/// Strict detached envelope. Public keys are deliberately absent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedHistoryAttestation {
    /// Signed terms.
    pub payload: HistoryAttestation,
    /// Ed25519 signature, lowercase hex.
    pub signature: String,
}

/// Trusted caller input, resolved from independently reviewed policy.
///
/// Never deserialize this from the untrusted manifest or derive it from its
/// assertions. Policy loading/approval is outside this verifier's authority.
pub struct HistoryCoverageGrant<'a> {
    /// Approved issuer identity.
    pub issuer_id: &'a str,
    /// Approved Ed25519 public key, supplied externally.
    pub public_key_hex: &'a str,
    /// Externally pinned policy revision's digest.
    pub policy_sha256: &'a str,
    /// Actual retained policy revision bytes.
    pub policy_bytes: &'a [u8],
    /// Approved review evidence identity.
    pub reviewed_evidence_ref: &'a str,
    /// Approved digest, not merely a digest supplied by the issuer.
    pub reviewed_evidence_sha256: &'a str,
    /// Exact reviewed artifact; an issuer cannot swap it under the same review.
    pub raw_artifact_sha256: &'a str,
    /// Exact authorized scope; no implicit widening to neighbouring graphs.
    pub coverage: &'a HistoryCoverage,
}

/// A verified statement of signed terms, not a store-completeness certificate.
pub struct VerifiedHistoryAttestation<'a> {
    payload: &'a HistoryAttestation,
    admitted_at: i64,
}

impl VerifiedHistoryAttestation<'_> {
    /// Exact verified signed terms.
    #[must_use]
    pub fn payload(&self) -> &HistoryAttestation {
        self.payload
    }

    /// Original admission time; expiry does not rewrite retained receipts.
    #[must_use]
    pub fn admitted_at(&self) -> i64 {
        self.admitted_at
    }
}

/// Failures never substitute a current source or a default issuer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryAttestationError {
    /// Unsupported version, invalid scope, digest or time interval.
    #[error("invalid history attestation terms")]
    InvalidTerms,
    /// Missing retained coverage review bytes.
    #[error("retained history review evidence is unavailable")]
    EvidenceUnavailable,
    /// Retained policy/review bytes differ from the externally pinned revision.
    #[error("history review or grant-policy digest mismatch")]
    EvidenceMismatch,
    /// The external grant does not authorize these exact terms.
    #[error("history attestation is outside the external coverage grant")]
    Unauthorized,
    /// Future or expired statement cannot authorize a fresh evaluation.
    #[error("history attestation is not current for admission")]
    NotCurrent,
    /// Invalid signing bytes or signature.
    #[error("history attestation signature failed")]
    Signature,
}

/// Exact v1 signing bytes: domain separator plus compact struct-ordered JSON.
///
/// This is not RFC8785. Deserialize into the strict structs, then serialize
/// these fields in declaration order; whitespace/key order in an envelope is
/// immaterial. Unknown and duplicate fields are refused by serde. Integer time
/// fields avoid float/date normalization. Enum spellings and field order are
/// versioned parts of the contract.
pub fn history_attestation_message(
    payload: &HistoryAttestation,
) -> Result<Vec<u8>, HistoryAttestationError> {
    let mut message = DOMAIN.to_vec();
    let json = serde_json::to_vec(payload).map_err(|_| HistoryAttestationError::InvalidTerms)?;
    message.extend_from_slice(&json);
    Ok(message)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Verify signed terms against external policy and actual retained evidence.
///
/// No network, filesystem mutation, key discovery or issuer registration.
/// The returned receipt deliberately cannot admit a raw store into evaluation.
pub fn verify_history_attestation<'a>(
    envelope: &'a SignedHistoryAttestation,
    grant: &HistoryCoverageGrant<'_>,
    retained_evidence: Option<&[u8]>,
    admission_time: i64,
) -> Result<VerifiedHistoryAttestation<'a>, HistoryAttestationError> {
    use HistoryAttestationError as E;
    let p = &envelope.payload;
    let c = &p.coverage;
    if p.schema_version != 1
        || p.issuer_id.is_empty()
        || c.source_store_id.is_empty()
        || c.transaction_first <= 0
        || c.transaction_first > c.transaction_last
        || c.valid_time_from >= c.valid_time_until
        || p.issued_at >= p.expires_at
        || [
            &c.catalogue_graph,
            &c.premise_graph,
            &p.reviewed_evidence_ref,
        ]
        .iter()
        .any(|iri| oxrdf::NamedNode::new(iri.as_str()).is_err())
        || [
            &p.grant_policy_sha256,
            &p.reviewed_evidence_sha256,
            &p.raw_artifact_sha256,
        ]
        .iter()
        .any(|hash| !valid_digest(hash))
    {
        return Err(E::InvalidTerms);
    }
    if p.issuer_id != grant.issuer_id
        || p.grant_policy_sha256 != grant.policy_sha256
        || p.reviewed_evidence_ref != grant.reviewed_evidence_ref
        || p.reviewed_evidence_sha256 != grant.reviewed_evidence_sha256
        || p.raw_artifact_sha256 != grant.raw_artifact_sha256
        || &p.coverage != grant.coverage
    {
        return Err(E::Unauthorized);
    }
    let evidence = retained_evidence.ok_or(E::EvidenceUnavailable)?;
    if digest(grant.policy_bytes) != grant.policy_sha256
        || digest(evidence) != grant.reviewed_evidence_sha256
    {
        return Err(E::EvidenceMismatch);
    }
    if admission_time < p.issued_at || admission_time >= p.expires_at {
        return Err(E::NotCurrent);
    }
    if envelope.signature.len() != 128
        || !envelope
            .signature
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !crate::signing::verify_hex(
            grant.public_key_hex,
            &history_attestation_message(p)?,
            &envelope.signature,
        )
    {
        return Err(E::Signature);
    }
    Ok(VerifiedHistoryAttestation {
        payload: p,
        admitted_at: admission_time,
    })
}
