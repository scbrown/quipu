//! Claimed-linkage verification — is an `aegis:implements` claim GROUNDED in
//! the work item it cites? (docs/design/semantic-grounded-edit-policies.md,
//! "Further applications" #1 — the grounding-integrity application.)
//!
//! A commit or bead-close that claims to implement a work item writes a
//! provenance edge the graph has, until now, simply trusted. An unverified
//! claim is exactly how a wrong provenance edge poisons everything derived
//! from it (replay-derived rules included), so this module makes the edge
//! CHECKABLE: compare the claiming content's similarity against the cited
//! item's description, and type the answer.
//!
//! ## Four answers, none of them a shrug
//!
//! - **Grounded** — the claim and the cited item are similar at or above the
//!   declared threshold.
//! - **Cited-but-dissimilar** — a real item is cited and the content is not
//!   near it: a fabricated linkage, its own violation class, never folded
//!   into either neighbour.
//! - **No-citation** — nothing was cited, or the citation resolves to nothing
//!   in the graph (an invented IRI is a fabricated REFERENCE, caught the same
//!   way the grounded ticket predicate catches `QUIP-999`).
//! - **Unevaluated** — no similarity method could run. Loud and its own
//!   variant, never a silent pass and never conflated with no-citation: "the
//!   check could not run" and "there was nothing to check" license entirely
//!   different responses. A recorded verdict maps this to the existing
//!   outcome `"unknown"` — see [`LinkageOutcome::vocabulary_value`].
//!
//! ## The verdict seal
//!
//! Every scored answer carries score, threshold, method/model identity and a
//! corpus watermark ([`LinkageEvidence`]) — the trust-chain rule applied to
//! embeddings: a score means nothing outside the model and corpus state that
//! produced it, and with all four on record the claim is an experiment anyone
//! can re-run, not an assertion. The catalog side lives in
//! `shapes/policies/linkage.ttl` (tier `"embedding"`, `must-ground`, advisory
//! placement only — the placement rules refuse a hard PAG deny on this tier).
//!
//! ## What this module does NOT do
//!
//! It records nothing. The typed result is the deliverable; a consumer that
//! writes any fact derived from it (the verdict, or the checked edge) must
//! land it `sourceKind "inferred"` in the low-trust plane — the camayoc
//! ingress discipline, unchanged. And the heavy corpus (bobbin's embedded
//! beads index) stays external: this entry point verifies against what the
//! STORE holds, which is quipu's honest share of the design.

use crate::embedding::build_entity_text;
use crate::error::Result;
use crate::store::Store;

use super::similarity::{EmbeddingCosine, TextSimilarity};

/// The threshold the shipped catalog declares (`aegis:op_implements_grounded`,
/// shapes/policies/linkage.ttl). A caller with its own calibrated
/// `OperatingPoint` passes that instead; this constant exists so code and
/// catalog agree on the exemplar value rather than drifting apart.
pub const CATALOG_LINKAGE_THRESHOLD: f64 = 0.75;

/// What seals a scored linkage verdict: enough to re-run the comparison and
/// disprove it. Recorded (by any consumer that persists this) as
/// `aegis:similarityScore` / `aegis:similarityThreshold` /
/// `aegis:embeddingModel` / `aegis:corpusWatermark` on the verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkageEvidence {
    /// The cited work item's IRI, resolved in the graph.
    pub cited_item: String,
    /// Similarity of the claiming content to the cited item's description.
    pub score: f64,
    /// The threshold the score was compared against.
    pub threshold: f64,
    /// The method/model identity that produced the score.
    pub method: String,
    /// The corpus state the comparison saw: the cited item's newest
    /// transaction (`tx:<n>`), because the item's DESCRIPTION is the corpus
    /// here — re-verify after the item changes and you are honestly scoring
    /// a different comparison.
    pub corpus_watermark: String,
}

/// The typed result of verifying one `aegis:implements` claim.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkageOutcome {
    /// The claim is similar to the cited item at or above the threshold.
    Grounded(LinkageEvidence),
    /// A real item is cited and the content is not near it — a fabricated
    /// linkage. The evidence rides along so the accusation is falsifiable.
    CitedButDissimilar(LinkageEvidence),
    /// Nothing cited, or the citation resolves to nothing in the graph.
    /// `detail` says which, naming the IRI when there is one.
    NoCitation {
        /// What exactly failed to resolve, for the reader deciding whether
        /// this is an omission or an invention.
        detail: String,
    },
    /// The check could not run — no similarity method, a failing one, or a
    /// cited item with no comparable text. NEVER a pass: an unevaluated
    /// claim is exactly as unverified as it was before this module existed,
    /// and saying so loudly is the whole contract.
    Unevaluated {
        /// Why it could not run, naming the remedy where there is one.
        reason: String,
    },
}

impl LinkageOutcome {
    /// The closed `aegis:linkageOutcome` vocabulary value a recorded verdict
    /// carries, or `None` for [`LinkageOutcome::Unevaluated`] — which maps to
    /// the verdict plane's existing outcome `"unknown"` (a check that could
    /// not run has told you nothing) rather than minting a fourth linkage
    /// value that would duplicate it.
    #[must_use]
    pub fn vocabulary_value(&self) -> Option<&'static str> {
        match self {
            LinkageOutcome::Grounded(_) => Some("grounded"),
            LinkageOutcome::CitedButDissimilar(_) => Some("cited-but-dissimilar"),
            LinkageOutcome::NoCitation { .. } => Some("no-citation"),
            LinkageOutcome::Unevaluated { .. } => None,
        }
    }
}

/// Verify a claimed linkage using the store's own embedding path.
///
/// Convenience over [`verify_claimed_linkage_with`]: the method is the
/// configured [`EmbeddingProvider`](crate::embedding::EmbeddingProvider) via
/// [`EmbeddingCosine`], and its absence degrades to
/// [`LinkageOutcome::Unevaluated`] — advisory absent, never an error, never a
/// fabricated score.
pub fn verify_claimed_linkage(
    store: &Store,
    claiming_text: &str,
    cited_item_iri: Option<&str>,
    threshold: f64,
) -> Result<LinkageOutcome> {
    let method = EmbeddingCosine::from_store(store);
    verify_claimed_linkage_with(
        store,
        claiming_text,
        cited_item_iri,
        threshold,
        method.as_ref().map(|m| m as &dyn TextSimilarity),
    )
}

/// Verify a claimed linkage with an explicit similarity method — the
/// injectable seam that makes the three-outcome logic provable without a live
/// embedding model.
///
/// `Err` only for store faults; every VERIFICATION state is a typed
/// [`LinkageOutcome`], including the method failing mid-comparison (an
/// [`LinkageOutcome::Unevaluated`] naming the error — a broken scorer has
/// judged nothing, and an error here would tempt callers into `unwrap_or`
/// shortcuts that read as passes).
pub fn verify_claimed_linkage_with(
    store: &Store,
    claiming_text: &str,
    cited_item_iri: Option<&str>,
    threshold: f64,
    method: Option<&dyn TextSimilarity>,
) -> Result<LinkageOutcome> {
    let Some(iri) = cited_item_iri else {
        return Ok(LinkageOutcome::NoCitation {
            detail: "the claim cites no work item".to_string(),
        });
    };

    // Resolution is against the GRAPH, not against IRI syntax: an IRI the
    // store has never seen and an interned term with no active facts are both
    // citations of nothing — the fabricated-reference case.
    let Some(entity) = store.lookup(iri)? else {
        return Ok(LinkageOutcome::NoCitation {
            detail: format!(
                "cited item '{iri}' is not in the graph — a citation of \
                 nothing, indistinguishable from an invented reference \
                 until the item lands"
            ),
        });
    };
    let facts = store.entity_facts(entity)?;
    if facts.is_empty() {
        return Ok(LinkageOutcome::NoCitation {
            detail: format!(
                "cited item '{iri}' has no active facts — the term exists but \
                 the graph holds nothing under it to implement"
            ),
        });
    }

    // The corpus here is the cited item's own description, so the watermark
    // is the item's newest transaction: the state of the text the score was
    // computed against, pinned before anything can change it.
    let watermark = format!("tx:{}", facts.iter().map(|f| f.tx).max().unwrap_or(0));
    let item_text = build_entity_text(store, entity)?;
    if item_text.is_empty() {
        // The item is REAL (facts exist) but carries nothing comparable —
        // reference-only values. Calling that no-citation would accuse a
        // legitimate citation of fabrication; calling it grounded would score
        // against emptiness. Unevaluated, loudly, is the only honest answer.
        return Ok(LinkageOutcome::Unevaluated {
            reason: format!(
                "cited item '{iri}' exists but has no textual description to \
                 compare against — give it an rdfs:label or rdfs:comment, or \
                 verify against an external corpus (bobbin's beads index)"
            ),
        });
    }

    let Some(method) = method else {
        return Ok(LinkageOutcome::Unevaluated {
            reason: "no similarity method is available — configure an \
                     embedding provider (see quipu's NO_PROVIDER_HELP) or \
                     inject one; an unevaluated claim stays exactly as \
                     unverified as it was, it does not pass"
                .to_string(),
        });
    };
    let score = match method.score(claiming_text, &item_text) {
        Ok(score) if score.is_finite() => score,
        Ok(score) => {
            return Ok(LinkageOutcome::Unevaluated {
                reason: format!(
                    "similarity method '{}' produced a non-finite score \
                     ({score}) — a malfunction, not a judgment",
                    method.identity()
                ),
            });
        }
        Err(e) => {
            return Ok(LinkageOutcome::Unevaluated {
                reason: format!(
                    "similarity method '{}' failed: {e} — the claim is \
                     unverified, not cleared",
                    method.identity()
                ),
            });
        }
    };

    let evidence = LinkageEvidence {
        cited_item: iri.to_string(),
        score,
        threshold,
        method: method.identity(),
        corpus_watermark: watermark,
    };
    // At-or-above grounds: the threshold is the declared operating point, and
    // a score sitting exactly on it satisfies the declaration.
    Ok(if score >= threshold {
        LinkageOutcome::Grounded(evidence)
    } else {
        LinkageOutcome::CitedButDissimilar(evidence)
    })
}

#[cfg(test)]
#[path = "linkage_tests.rs"]
mod tests;
