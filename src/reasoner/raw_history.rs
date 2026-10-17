//! Admission of exact retained raw bytes under externally reviewed coverage.
//!
//! No producer or live-store snapshotting is provided here. The signed review
//! must already attest that the exact artifact is a consistent, complete raw
//! snapshot for the authorized scope, including pre-window supporting history.
//! Integrity checks cannot independently prove that completeness statement.

use std::io::Write;

use sha2::{Digest, Sha256};

use crate::{config::QuipuConfig, store::Store};

use super::history_attestation::{HistoryAttestation, VerifiedHistoryAttestation};

/// The one dual-time evaluation admitted by this source receipt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RawHistoryRequest {
    /// Whole UTC Unix seconds, within the reviewed valid-time window.
    pub valid_at: i64,
    /// A real retained source-local transaction within the reviewed interval.
    pub as_of_tx: i64,
}

/// Owned immutable read source, restricted by the accompanying coverage receipt.
///
/// Opening a copied artifact avoids races with changes to a caller's retention
/// path. Its private file lives exactly as long as this reader. This is not a
/// producer certificate or permission to query outside the admitted request.
pub struct VerifiedRawHistorySource {
    store: Store,
    _artifact: tempfile::NamedTempFile,
    terms: HistoryAttestation,
    request: RawHistoryRequest,
    admitted_at: i64,
}

impl VerifiedRawHistorySource {
    /// Read-only source with the caller's read-relevant policy copied across.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Signed artifact, lineage, coverage and evidence identity.
    #[must_use]
    pub fn terms(&self) -> &HistoryAttestation {
        &self.terms
    }

    /// Exact request covered by this admission, not a mutable current context.
    #[must_use]
    pub fn request(&self) -> &RawHistoryRequest {
        &self.request
    }

    /// Original raw-source admission time; a retained receipt keeps this value.
    #[must_use]
    pub fn admitted_at(&self) -> i64 {
        self.admitted_at
    }
}

/// Explicit refusals preserve UNKNOWN instead of manufacturing completeness.
#[derive(Debug, thiserror::Error)]
pub enum RawHistoryError {
    /// Bounded v1 raw artifact is at most 256 MiB.
    #[error("retained raw history artifact exceeds capacity")]
    Capacity,
    /// Bytes do not match the externally reviewed signed artifact.
    #[error("retained raw history artifact digest mismatch")]
    ArtifactMismatch,
    /// Malformed/inconsistent raw database or unsupported persisted providers.
    #[error("unsupported or inconsistent retained raw history artifact")]
    InvalidArtifact,
    /// Do not detach actual/configured providers to fit a native-only profile.
    #[error("raw historical admission refuses attached providers")]
    AttachedProviders,
    /// Store identity differs from the reviewed source lineage.
    #[error("retained raw history lineage mismatch")]
    LineageMismatch,
    /// A transaction endpoint/request is not actually retained.
    #[error("unknown retained raw history transaction")]
    UnknownTransaction,
    /// Requested dual-time coordinates fall outside the reviewed coverage.
    #[error("raw historical request is outside reviewed coverage")]
    OutsideCoverage,
    /// A prior terms receipt does not extend authority past its expiry.
    #[error("history review is not current for fresh raw-source admission")]
    NotCurrent,
    /// Required explicit graph is not present in this artifact.
    #[error("reviewed raw history graph is unavailable")]
    MissingGraph,
    /// Filesystem failure while owning the immutable private read copy.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Raw database/read-policy error, never interpreted as absence.
    #[error(transparent)]
    Store(#[from] crate::Error),
}

/// Admit retained bytes, never a live database path or an unverified full pack.
///
/// `configured` and `read_policy` must be the actual caller's configuration and
/// configured store. Supplying a default configuration is not a policy bypass.
/// The signed review is the completeness authority; this function checks exact
/// bytes, database consistency, lineage, retained transactions and provider
/// closure. A partial import cannot acquire authority merely by passing these
/// structural checks: its digest still needs independently authorized review.
pub fn admit_raw_history(
    bytes: &[u8],
    attestation: &VerifiedHistoryAttestation<'_>,
    request: RawHistoryRequest,
    configured: &QuipuConfig,
    read_policy: &Store,
    admission_time: i64,
) -> Result<VerifiedRawHistorySource, RawHistoryError> {
    use RawHistoryError as E;
    if !configured.attachments.is_empty() || !read_policy.attachments().is_empty() {
        return Err(E::AttachedProviders);
    }
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(E::Capacity);
    }
    let terms = attestation.payload();
    if admission_time < attestation.admitted_at()
        || admission_time < terms.issued_at
        || admission_time >= terms.expires_at
    {
        return Err(E::NotCurrent);
    }
    let coverage = &terms.coverage;
    if format!("{:x}", Sha256::digest(bytes)) != terms.raw_artifact_sha256 {
        return Err(E::ArtifactMismatch);
    }
    if request.valid_at < coverage.valid_time_from
        || request.valid_at >= coverage.valid_time_until
        || request.as_of_tx < coverage.transaction_first
        || request.as_of_tx > coverage.transaction_last
    {
        return Err(E::OutsideCoverage);
    }
    if !bytes.starts_with(b"SQLite format 3\0") {
        return Err(E::InvalidArtifact);
    }
    let mut artifact = tempfile::NamedTempFile::new()?;
    artifact.write_all(bytes)?;
    artifact.flush()?;
    // This fresh private file has no WAL/journal siblings. The review binds
    // these exact standalone bytes; a caller cannot append an unseen live WAL.
    let path = artifact.path().to_str().ok_or(E::InvalidArtifact)?;
    let mut store = Store::open_immutable(path)?;
    store.adopt_read_config_from(read_policy);
    let check: String = store
        .conn
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(crate::Error::from)?;
    if check != "ok" {
        return Err(E::InvalidArtifact);
    }
    if store.store_id()? != coverage.source_store_id {
        return Err(E::LineageMismatch);
    }
    // Read-only opening intentionally does not mount frozen providers. Inspect
    // their persisted registry as well, so that omission cannot certify closure.
    let frozen_table: bool = store
        .conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='frozen_packs')",
            [],
            |row| row.get(0),
        )
        .map_err(crate::Error::from)?;
    if frozen_table {
        let active: bool = store
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM frozen_packs WHERE thawed_at IS NULL)",
                [],
                |row| row.get(0),
            )
            .map_err(crate::Error::from)?;
        if active {
            return Err(E::AttachedProviders);
        }
    }
    for tx in [
        coverage.transaction_first,
        coverage.transaction_last,
        request.as_of_tx,
    ] {
        if store.get_transaction(tx)?.is_none() {
            return Err(E::UnknownTransaction);
        }
    }
    for graph in [&coverage.catalogue_graph, &coverage.premise_graph] {
        if crate::store::inferred::is_inferred_graph_iri(graph) {
            return Err(E::InvalidArtifact);
        }
        let registered = if graph == crate::schema::ROOT_GRAPH_IRI {
            // ROOT is reserved graph zero, not an interned named-graph IRI.
            store
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM graphs WHERE g = ?1)",
                    [crate::schema::ROOT_GRAPH],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(crate::Error::from)?
        } else {
            store.registered_graph_id(graph)?.is_some()
        };
        if !registered {
            return Err(E::MissingGraph);
        }
    }
    Ok(VerifiedRawHistorySource {
        store,
        _artifact: artifact,
        terms: terms.clone(),
        request,
        admitted_at: admission_time,
    })
}
