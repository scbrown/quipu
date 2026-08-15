//! Claimed-linkage verification tests. Size-exempt (`*tests.rs`).
//!
//! The similarity method is injected ([`TextSimilarity`] stubs), so every
//! outcome is reachable deterministically, with no live embedding model.

use super::*;
use crate::error::Error;
use crate::namespace::{RDF_TYPE, RDFS_LABEL};
use crate::store::Datum;
use crate::types::{Op, Value};

const TS: &str = "2026-01-01T00:00:00Z";
const ITEM: &str = "http://ex/bead-8dk";
const CLAIM: &str = "attach nearest decided requests as precedent when minting";

/// A method that returns the same score for every comparison.
struct Fixed(f64);

impl TextSimilarity for Fixed {
    fn identity(&self) -> String {
        "test:fixed".to_string()
    }
    fn score(&self, _a: &str, _b: &str) -> crate::error::Result<f64> {
        Ok(self.0)
    }
}

/// A method that always fails — the loud-degradation case.
struct Broken;

impl TextSimilarity for Broken {
    fn identity(&self) -> String {
        "test:broken".to_string()
    }
    fn score(&self, _a: &str, _b: &str) -> crate::error::Result<f64> {
        Err(Error::Store("the scorer is on fire".to_string()))
    }
}

/// A store holding one work item with a describable label.
fn store_with_item() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let entity = store.intern(ITEM).unwrap();
    let d = |attribute: i64, value: Value| Datum {
        entity,
        attribute,
        value,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    let datums = vec![
        d(
            store.intern(RDF_TYPE).unwrap(),
            Value::Ref(
                store
                    .intern("http://aegis.gastown.local/ontology/WorkItem")
                    .unwrap(),
            ),
        ),
        d(
            store.intern(RDFS_LABEL).unwrap(),
            Value::Str("Escalation precedent: attach nearest decided DecisionRequests".to_string()),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();
    store
}

#[test]
fn a_near_claim_is_grounded_with_the_full_seal_on_record() {
    let store = store_with_item();
    let outcome =
        verify_claimed_linkage_with(&store, CLAIM, Some(ITEM), 0.75, Some(&Fixed(0.9))).unwrap();
    let LinkageOutcome::Grounded(evidence) = outcome else {
        panic!("a 0.9 score over a 0.75 threshold must ground, got {outcome:?}");
    };
    // The seal: everything needed to re-run the comparison and disprove it.
    assert_eq!(evidence.cited_item, ITEM);
    assert!((evidence.score - 0.9).abs() < 1e-9);
    assert!((evidence.threshold - 0.75).abs() < 1e-9);
    assert_eq!(evidence.method, "test:fixed");
    assert!(
        evidence.corpus_watermark.starts_with("tx:"),
        "the watermark pins which state of the item was compared: {}",
        evidence.corpus_watermark
    );
}

#[test]
fn a_far_claim_is_cited_but_dissimilar_not_merely_ungrounded() {
    // The fabricated-linkage outcome: a REAL item is cited and the content is
    // not near it. Its own class — folding it into no-citation would hide
    // exactly the provenance poisoning this check exists to catch — and the
    // evidence rides along, so the accusation is falsifiable too.
    let store = store_with_item();
    let outcome =
        verify_claimed_linkage_with(&store, CLAIM, Some(ITEM), 0.75, Some(&Fixed(0.2))).unwrap();
    let LinkageOutcome::CitedButDissimilar(evidence) = outcome else {
        panic!("a 0.2 score must be cited-but-dissimilar, got {outcome:?}");
    };
    assert!((evidence.score - 0.2).abs() < 1e-9);
    assert_eq!(evidence.method, "test:fixed");
}

#[test]
fn a_score_on_the_threshold_grounds() {
    // >= not >: the threshold is the declared operating point, and a score
    // sitting exactly on it satisfies the declaration.
    let store = store_with_item();
    let outcome =
        verify_claimed_linkage_with(&store, CLAIM, Some(ITEM), 0.75, Some(&Fixed(0.75))).unwrap();
    assert!(matches!(outcome, LinkageOutcome::Grounded(_)));
}

#[test]
fn citing_nothing_is_no_citation() {
    let store = store_with_item();
    let outcome =
        verify_claimed_linkage_with(&store, CLAIM, None, 0.75, Some(&Fixed(0.9))).unwrap();
    let LinkageOutcome::NoCitation { detail } = outcome else {
        panic!("no cited IRI must be no-citation, got {outcome:?}");
    };
    assert!(detail.contains("cites no"), "{detail}");
    assert_eq!(
        LinkageOutcome::NoCitation { detail }.vocabulary_value(),
        Some("no-citation")
    );
}

#[test]
fn an_unresolvable_citation_is_no_citation_naming_the_invented_iri() {
    // The fabricated-reference case: an IRI the graph does not hold. Even a
    // perfect similarity method must not ground a citation of nothing.
    let store = store_with_item();
    let outcome = verify_claimed_linkage_with(
        &store,
        CLAIM,
        Some("http://ex/QUIP-999"),
        0.75,
        Some(&Fixed(1.0)),
    )
    .unwrap();
    let LinkageOutcome::NoCitation { detail } = outcome else {
        panic!("an unresolvable citation must be no-citation, got {outcome:?}");
    };
    assert!(
        detail.contains("QUIP-999") && detail.contains("not in the graph"),
        "the detail names the invented reference: {detail}"
    );
}

#[test]
fn an_interned_but_factless_iri_is_no_citation_too() {
    // Interning a term is not putting an entity in the graph: a citation must
    // resolve to FACTS, or a claimant could pre-intern its alibi.
    let store = store_with_item();
    store.intern("http://ex/hollow").unwrap();
    let outcome = verify_claimed_linkage_with(
        &store,
        CLAIM,
        Some("http://ex/hollow"),
        0.75,
        Some(&Fixed(1.0)),
    )
    .unwrap();
    let LinkageOutcome::NoCitation { detail } = outcome else {
        panic!("a factless citation must be no-citation, got {outcome:?}");
    };
    assert!(detail.contains("no active facts"), "{detail}");
}

#[test]
fn no_similarity_method_is_unevaluated_loud_and_distinct_from_no_citation() {
    // The degraded-provider contract: a real citation with no way to check it
    // is UNVERIFIED, not absent and not passed. The variant is its own, the
    // reason names the remedy, and the vocabulary mapping is None — the
    // verdict plane's "unknown", never a fourth linkage value.
    let store = store_with_item();
    let outcome = verify_claimed_linkage(&store, CLAIM, Some(ITEM), 0.75).unwrap();
    let LinkageOutcome::Unevaluated { reason } = &outcome else {
        panic!("no provider must be unevaluated, got {outcome:?}");
    };
    assert!(
        reason.contains("embedding provider"),
        "the reason names the remedy: {reason}"
    );
    assert!(
        reason.contains("does not pass"),
        "and says what unevaluated is NOT: {reason}"
    );
    assert_eq!(outcome.vocabulary_value(), None);
}

#[test]
fn a_failing_method_is_unevaluated_naming_the_failure() {
    // A broken scorer has judged nothing. Returning Err would tempt callers
    // into unwrap_or shortcuts that read as passes; the typed loud non-answer
    // keeps the claim visibly unverified.
    let store = store_with_item();
    let outcome =
        verify_claimed_linkage_with(&store, CLAIM, Some(ITEM), 0.75, Some(&Broken)).unwrap();
    let LinkageOutcome::Unevaluated { reason } = outcome else {
        panic!("a failing method must be unevaluated, got {outcome:?}");
    };
    assert!(
        reason.contains("test:broken") && reason.contains("on fire"),
        "the reason names the method and the failure: {reason}"
    );
}

#[test]
fn an_item_with_no_comparable_text_is_unevaluated_not_accused() {
    // Facts exist but none are describable text (reference-only values):
    // calling it no-citation would accuse a legitimate citation of
    // fabrication, and scoring against emptiness would be a fabricated
    // comparison. Unevaluated, naming what the item lacks.
    let mut store = store_with_item();
    let entity = store.intern("http://ex/refonly").unwrap();
    let other = store.intern("http://ex/other").unwrap();
    let datums = vec![Datum {
        entity,
        attribute: store.intern("http://ex/pointsAt").unwrap(),
        value: Value::Ref(other),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    store.transact(&datums, TS, None, None).unwrap();
    let outcome = verify_claimed_linkage_with(
        &store,
        CLAIM,
        Some("http://ex/refonly"),
        0.75,
        Some(&Fixed(1.0)),
    )
    .unwrap();
    let LinkageOutcome::Unevaluated { reason } = outcome else {
        panic!("an undescribable item must be unevaluated, got {outcome:?}");
    };
    assert!(
        reason.contains("no textual description") && reason.contains("rdfs:label"),
        "the reason names the gap and the remedy: {reason}"
    );
}

#[test]
fn the_vocabulary_values_are_the_closed_three_outcome_set() {
    // The Rust enum and the aegis:linkageOutcome sh:in must say the same
    // thing; this is the code half of that agreement (the shape half lives in
    // governance_tests.rs).
    let evidence = LinkageEvidence {
        cited_item: ITEM.to_string(),
        score: 0.9,
        threshold: 0.75,
        method: "test:fixed".to_string(),
        corpus_watermark: "tx:1".to_string(),
    };
    assert_eq!(
        LinkageOutcome::Grounded(evidence.clone()).vocabulary_value(),
        Some("grounded")
    );
    assert_eq!(
        LinkageOutcome::CitedButDissimilar(evidence).vocabulary_value(),
        Some("cited-but-dissimilar")
    );
}
