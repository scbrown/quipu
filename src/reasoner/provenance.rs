//! Integrity checks for retained source fragments.
//!
//! Byte identity is distinct from authority: this verifies supplied retained
//! bytes, not a publisher, legal interpretation or history-coverage claim.
//! Missing retention never triggers a fetch from a possibly changed URL.

use sha2::{Digest, Sha256};

/// A byte range, following W3C Web Annotation `DataPositionSelector` units.
///
/// Zero-based start is inclusive; end is exclusive. This is not a
/// `TextPositionSelector`: Unicode characters and normalized text are not bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataPosition {
    /// First included byte.
    pub start: u64,
    /// First excluded byte.
    pub end: u64,
}

/// Canonicalization is explicit and must not silently change source bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FragmentCanonicalization {
    /// Exact retained bytes, including encoding, line endings and whitespace.
    RawBytesV1,
}

/// Identity of a retained fragment within an exact document edition.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FragmentReference {
    /// Document edition IRI; a current mutable source URL is insufficient.
    pub document_edition: String,
    /// Stable identity of the selected provision/fragment.
    pub fragment_iri: String,
    /// SHA256 of the entire retained document's exact bytes (lowercase hex).
    pub document_sha256: String,
    /// Byte-position selection within that document.
    pub selector: DataPosition,
    /// Explicit byte interpretation.
    pub canonicalization: FragmentCanonicalization,
    /// SHA256 of the selected bytes (lowercase hex).
    pub fragment_sha256: String,
}

/// Explicit integrity/retention failures; none substitutes a newer source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FragmentError {
    /// Retained source bytes are unavailable.
    #[error("retained source document is unavailable")]
    Unavailable,
    /// Supplied identity is not an IRI.
    #[error("invalid document or fragment identity")]
    InvalidIdentity,
    /// Byte range is empty, reversed, out of range or not representable.
    #[error("invalid source byte selector")]
    InvalidSelector,
    /// Retained document differs from the pinned edition.
    #[error("retained document digest mismatch")]
    DocumentMismatch,
    /// Selected bytes differ from the pinned fragment.
    #[error("retained fragment digest mismatch")]
    FragmentMismatch,
    /// Bounded v1 retained-document/fragment profile exceeded.
    #[error("source fragment capacity exceeded")]
    Capacity,
}

/// A borrowed fragment whose entire document and selection passed integrity.
pub struct VerifiedFragment<'a> {
    reference: &'a FragmentReference,
    bytes: &'a [u8],
}

impl VerifiedFragment<'_> {
    /// Exact immutable identity supplied for verification.
    #[must_use]
    pub fn reference(&self) -> &FragmentReference {
        self.reference
    }

    /// Exact selected retained bytes; no text decoding or normalization.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }
}

/// Verify a retained fragment without network access or source mutation.
///
/// Both whole-document and fragment checks matter: an unchanged quotation
/// inside a changed edition does not establish the original source identity.
/// This function supplies integrity, not approval or legal correctness.
pub fn verify_fragment<'a>(
    reference: &'a FragmentReference,
    retained_document: Option<&'a [u8]>,
) -> Result<VerifiedFragment<'a>, FragmentError> {
    if oxrdf::NamedNode::new(&reference.document_edition).is_err()
        || oxrdf::NamedNode::new(&reference.fragment_iri).is_err()
    {
        return Err(FragmentError::InvalidIdentity);
    }
    let document = retained_document.ok_or(FragmentError::Unavailable)?;
    if document.len() > 16 * 1024 * 1024 {
        return Err(FragmentError::Capacity);
    }
    if format!("{:x}", Sha256::digest(document)) != reference.document_sha256 {
        return Err(FragmentError::DocumentMismatch);
    }
    let start =
        usize::try_from(reference.selector.start).map_err(|_| FragmentError::InvalidSelector)?;
    let end =
        usize::try_from(reference.selector.end).map_err(|_| FragmentError::InvalidSelector)?;
    if start >= end {
        return Err(FragmentError::InvalidSelector);
    }
    let fragment = document
        .get(start..end)
        .ok_or(FragmentError::InvalidSelector)?;
    if fragment.len() > 1024 * 1024 {
        return Err(FragmentError::Capacity);
    }
    if format!("{:x}", Sha256::digest(fragment)) != reference.fragment_sha256 {
        return Err(FragmentError::FragmentMismatch);
    }
    Ok(VerifiedFragment {
        reference,
        bytes: fragment,
    })
}
