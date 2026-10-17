//! Controlled byte-retention checks; no legal-source or authority claim.

use quipu::reasoner::provenance::{
    DataPosition, FragmentCanonicalization, FragmentError, FragmentReference, verify_fragment,
};
use sha2::{Digest, Sha256};

fn reference(document: &[u8], start: usize, end: usize) -> FragmentReference {
    FragmentReference {
        document_edition: "https://example.org/document/edition-1".into(),
        fragment_iri: "https://example.org/document/edition-1#article-1".into(),
        document_sha256: format!("{:x}", Sha256::digest(document)),
        selector: DataPosition {
            start: start as u64,
            end: end as u64,
        },
        canonicalization: FragmentCanonicalization::RawBytesV1,
        fragment_sha256: format!("{:x}", Sha256::digest(&document[start..end])),
    }
}

#[test]
fn rule_source_provenance_uses_bytes_not_characters_or_normalized_text() {
    let document = "Aé\r\narticle\t1\n".as_bytes();
    let pin = reference(document, 1, 3);
    let fragment = verify_fragment(&pin, Some(document)).unwrap();
    assert_eq!(fragment.bytes(), "é".as_bytes());
    assert_eq!(fragment.reference(), &pin);
    let serialized = serde_json::to_string(&pin).unwrap();
    let decoded: FragmentReference = serde_json::from_str(&serialized).unwrap();
    assert_eq!(decoded, pin);
    assert!(serialized.contains("raw_bytes_v1"));
    let changed_line_endings = "Aé\narticle\t1\n".as_bytes();
    assert!(matches!(
        verify_fragment(&pin, Some(changed_line_endings)),
        Err(FragmentError::DocumentMismatch)
    ));
}

#[test]
fn rule_source_provenance_missing_and_tampered_retention_refuse() {
    let document = b"header\narticle one\nfooter";
    let pin = reference(document, 7, 18);
    assert_eq!(
        verify_fragment(&pin, Some(document)).unwrap().bytes(),
        b"article one"
    );
    assert!(matches!(
        verify_fragment(&pin, None),
        Err(FragmentError::Unavailable)
    ));
    let mut wrong_fragment = pin.clone();
    wrong_fragment.fragment_sha256 = "0".repeat(64);
    assert!(matches!(
        verify_fragment(&wrong_fragment, Some(document)),
        Err(FragmentError::FragmentMismatch)
    ));
    let changed_outside_selection = b"HEADER\narticle one\nfooter";
    assert!(matches!(
        verify_fragment(&pin, Some(changed_outside_selection)),
        Err(FragmentError::DocumentMismatch)
    ));
    let mut bad_range = pin.clone();
    bad_range.selector.end = u64::MAX;
    assert!(matches!(
        verify_fragment(&bad_range, Some(document)),
        Err(FragmentError::InvalidSelector)
    ));
    bad_range.selector.end = bad_range.selector.start;
    assert!(matches!(
        verify_fragment(&bad_range, Some(document)),
        Err(FragmentError::InvalidSelector)
    ));
}

#[test]
fn rule_source_provenance_changed_editions_do_not_replace_retained_bytes() {
    let old = b"article one: old model";
    let new = b"article one: revised model";
    let old_pin = reference(old, 0, old.len());
    let mut new_pin = reference(new, 0, new.len());
    new_pin.document_edition = "https://example.org/document/edition-2".into();
    new_pin.fragment_iri = "https://example.org/document/edition-2#article-1".into();
    assert_ne!(old_pin.document_sha256, new_pin.document_sha256);
    assert_ne!(old_pin.fragment_iri, new_pin.fragment_iri);
    assert_eq!(verify_fragment(&old_pin, Some(old)).unwrap().bytes(), old);
    assert_eq!(verify_fragment(&new_pin, Some(new)).unwrap().bytes(), new);
    assert!(matches!(
        verify_fragment(&old_pin, Some(new)),
        Err(FragmentError::DocumentMismatch)
    ));
    assert!(matches!(
        verify_fragment(&old_pin, None),
        Err(FragmentError::Unavailable)
    ));
}
