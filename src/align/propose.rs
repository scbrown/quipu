//! Candidate generation: which concepts in two graphs might be the same thing.
//!
//! Split into a PURE core over two concept lists and a store-backed enumerator,
//! because the properties that matter are properties of the core:
//! determinism (acceptance criterion 2) and suppression of already-reviewed
//! pairs (criterion 3) are decided here and are testable without a database.
//!
//! ## It reuses resolution's scoring rule rather than growing a second matcher
//!
//! The design says candidate generation must not become a second matcher. The
//! part that would actually diverge is not the orchestration — alignment needs
//! a graph-scoped candidate space that `resolve_entity` cannot express — it is
//! the JUDGEMENT about when two names mean the same thing. So this shares that:
//! case-insensitive exact match scores 1.0, the
//! [`is_slash_qualified_commit_id`] exemption applies (two different commit
//! hashes are distinct even though their shared `commit/<repo>/` prefix scores
//! high), and everything else is Jaro-Winkler over the same `strsim`.
//!
//! Sharing the rule is what keeps alignment from proposing a pair that
//! resolution deliberately refuses.

use std::collections::BTreeMap;

use crate::resolution::is_slash_qualified_commit_id;

use super::sssom::{Justification, Mapping, MappingSet, OWL_SAME_AS};

/// A concept as alignment sees it: an IRI, a label to match on, and its types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concept {
    /// The concept's IRI.
    pub iri: String,
    /// Its `rdfs:label`.
    pub label: String,
    /// Its `rdf:type`s, sorted. Used by per-type link rules.
    pub types: Vec<String>,
}

/// A declarative rule per `rdf:type`, in the manner of Silk and LIMES.
///
/// A single global fuzzy-name threshold is the thing to avoid: it cannot say
/// "two Repositories with the same url are the same even if the names differ,
/// and two Hosts with similar names are NOT the same unless the owner matches".
/// A rule a human can read, argue with and version can.
///
/// v1 carries the two cut-offs and a type gate; the richer signal combination
/// the design points at (shared literal keys, embedding distance, a
/// Fellegi-Sunter combination) extends this struct rather than replacing it.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkSpec {
    /// Below this, a pair is not worth showing anyone.
    pub floor: f64,
    /// At or above this, a pair is confident enough to bulk-accept.
    ///
    /// Between the two is LogMap's "uncertain band" — the only region worth a
    /// human's attention, and the reason both numbers live next to the scoring
    /// that produced them rather than in a command-line flag.
    pub auto_accept: f64,
    /// Require the two concepts to share at least one `rdf:type`.
    ///
    /// On by default: two graphs that both call something `bobbin` and mean a
    /// Repository and a Host respectively are exactly the false positive an
    /// exact name match cannot catch, and it is the case the design names.
    pub require_shared_type: bool,
}

impl Default for LinkSpec {
    fn default() -> Self {
        // 0.85 is resolution's own candidate threshold; starting alignment
        // somewhere else would propose pairs resolution would not, for no
        // stated reason.
        Self {
            floor: 0.85,
            auto_accept: 1.0,
            require_shared_type: true,
        }
    }
}

/// Score two labels by the same rule resolution uses.
///
/// Returns `None` when the pair is not a candidate at all — which is different
/// from scoring zero, and is how the commit-id exemption refuses rather than
/// ranks.
#[must_use]
pub fn score_labels(a: &str, b: &str) -> Option<(f64, Justification)> {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    if a == b {
        return Some((1.0, Justification::LexicalMatching));
    }
    if is_slash_qualified_commit_id(&a) && is_slash_qualified_commit_id(&b) {
        return None;
    }
    Some((
        strsim::jaro_winkler(&a, &b),
        Justification::LexicalSimilarityThresholdMatching,
    ))
}

/// Generate candidate mappings between two sets of concepts.
///
/// `prior` is a set already reviewed — every pair in it is suppressed, whether
/// the operator asserted or declined, because both mean "do not show me this
/// again". Suppression is keyed on the unordered pair, so a judgement recorded
/// in one direction is not undone by generating the candidate in the other.
///
/// The result is sorted, so the same inputs produce byte-identical output
/// regardless of the order the concepts arrived in.
#[must_use]
pub fn propose(
    a: &[Concept],
    b: &[Concept],
    spec: &LinkSpec,
    prior: &MappingSet,
    mapping_set_id: &str,
) -> MappingSet {
    let reviewed: BTreeMap<(String, String), ()> = prior.reviewed();
    let mut set = MappingSet::new(mapping_set_id);

    for left in a {
        for right in b {
            if left.iri == right.iri {
                continue;
            }
            if spec.require_shared_type && !left.types.iter().any(|t| right.types.contains(t)) {
                continue;
            }
            let Some((score, justification)) = score_labels(&left.label, &right.label) else {
                continue;
            };
            if score < spec.floor {
                continue;
            }
            let candidate = Mapping {
                subject_id: left.iri.clone(),
                subject_label: Some(left.label.clone()),
                predicate_id: OWL_SAME_AS.to_string(),
                object_id: right.iri.clone(),
                object_label: Some(right.label.clone()),
                mapping_justification: justification,
                predicate_modifier_not: None,
                confidence: Some(score),
                // Unauthored and unreviewed: `propose` decides nothing.
                author_id: None,
                quipu_review: None,
                quipu_reviewed_by: None,
            };
            if reviewed.contains_key(&candidate.pair_key()) {
                continue;
            }
            set.mappings.push(candidate);
        }
    }

    set.sort();
    set
}
