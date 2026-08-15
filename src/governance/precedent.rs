//! Escalation precedent — the nearest prior DECIDED requests ride a freshly
//! minted `DecisionRequest` (docs/design/semantic-grounded-edit-policies.md,
//! "Further applications" #4).
//!
//! An operator asked to rule on an escalation rules better knowing "you
//! rejected something similar, and here is where you said why". This module
//! finds that precedent at mint time: prior `DecisionRequest`s that actually
//! GOT a ruling — a signed `aegis:Decision` from a registered decider, the
//! same trust rule [`super::router::resolve`] applies, reused rather than
//! restated — scored by similarity to the request being minted.
//!
//! ## Advice, with the receipts attached
//!
//! Stage-1 of the identify-and-inform-before-refusing ordering: the precedent
//! informs the human; the ruling stays the human's. Two disciplines follow:
//!
//! - **Every link is falsifiable.** Each attached precedent carries its
//!   similarity score AND the method/model identity that produced it
//!   (`aegis:similarityScore`, `aegis:similarityMethod`), because a score is
//!   meaningless outside its method — re-run the named method over the two
//!   requests' policy+target texts to disprove the claim. Scores attach to a
//!   reified `aegis:PrecedentLink` node, not to the request, since three
//!   precedents on one request would otherwise leave three bare scores nobody
//!   can pair with their subjects.
//! - **The advisory path cannot block minting.** Minting the request is the
//!   load-bearing act (a refusal an operator cannot act on is an outage
//!   wearing governance's clothes); the precedent is advice about it. No
//!   similarity method configured, a failing provider, a malformed prior —
//!   all degrade to "no precedent attached", never to a mint error and never
//!   to a fabricated score.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use super::router::{decision_verifies, evidence_hash};
use super::similarity::{EmbeddingCosine, TextSimilarity};

/// The most precedents one request carries. Three is enough to show a pattern
/// ("we keep rejecting this shape of thing"); an uncapped list buries the
/// nearest ruling under its own long tail.
pub const MAX_PRECEDENTS: usize = 3;

/// One scored precedent claim: a prior decided request near the one minted.
#[derive(Debug, Clone, PartialEq)]
pub struct Precedent {
    /// The prior `DecisionRequest`'s IRI.
    pub request_iri: String,
    /// Similarity of the minted request's policy+target text to the prior's.
    pub score: f64,
    /// The method/model identity that produced `score` — what a reader
    /// re-runs to falsify the claim.
    pub method: String,
}

/// The precedent datums for a request being minted at `subject`, or empty.
///
/// Empty on EVERY degraded path — no embedding provider, no decided priors,
/// nothing scoring above zero, or any error along the way — because this is
/// the advisory half of [`super::router::mint_request`] and advice that can
/// veto the thing it advises about has the authority relation backwards.
pub(crate) fn advisory_datums(
    store: &Store,
    subject: i64,
    policy_iri: &str,
    target_iri: &str,
    timestamp: &str,
) -> Vec<Datum> {
    // No method, no precedent. Deliberately NOT a cheaper textual heuristic:
    // an unlabeled fallback would put scores on record that the recorded
    // method name could not reproduce (see `similarity.rs`).
    let Some(method) = EmbeddingCosine::from_store(store) else {
        return Vec::new();
    };
    nearest_decided(store, policy_iri, target_iri, &method)
        .and_then(|found| datums_for(store, subject, policy_iri, target_iri, &found, timestamp))
        .unwrap_or_default()
}

/// The nearest prior DECIDED requests to `(policy_iri, target_iri)`, best
/// first, capped at [`MAX_PRECEDENTS`].
///
/// "Decided" means what [`super::router::resolve`] means: a `Decision` bound
/// to the prior request's evidence hash, signed by a registered decider —
/// checked with the router's own [`decision_verifies`], so a forged or
/// unsigned ruling can no more become precedent than it could permit a write.
/// An undecided request is an open question, and an open question is not
/// precedent for anything.
pub fn nearest_decided(
    store: &Store,
    policy_iri: &str,
    target_iri: &str,
    method: &dyn TextSimilarity,
) -> Result<Vec<Precedent>> {
    // The request being minted must not cite itself (a re-mint after expiry
    // has a decided prior under its OWN evidence hash).
    let own_hash = evidence_hash(policy_iri, target_iri);

    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?r ?rp ?rt ?h ?outcome ?by ?sig WHERE {{ \
            ?r a a:DecisionRequest ; a:forPolicy ?rp ; a:forTarget ?rt ; \
               a:evidenceHash ?h . \
            ?d a a:Decision ; a:evidenceHash ?h ; a:outcome ?outcome ; \
               a:by ?by ; a:signature ?sig . \
         }}"
    );
    // BTreeMap keyed by IRI: dedups a request with several decisions, and
    // fixes the scoring order so equal scores break ties deterministically.
    let mut decided: BTreeMap<String, String> = BTreeMap::new();
    if let QueryResult::Select { rows, .. } = sparql::query(store, &q)? {
        for row in rows {
            let (Some(iri), Some(prior_policy), Some(prior_target), Some(hash)) = (
                iri_of(store, row.get("r")),
                str_of(row.get("rp")),
                str_of(row.get("rt")),
                str_of(row.get("h")),
            ) else {
                continue;
            };
            if hash == own_hash || decided.contains_key(&iri) {
                continue;
            }
            let (Some(outcome), Some(by), Some(sig)) = (
                str_of(row.get("outcome")),
                str_of(row.get("by")),
                str_of(row.get("sig")),
            ) else {
                continue;
            };
            if !decision_verifies(store, &prior_policy, &hash, &outcome, &by, &sig)? {
                continue;
            }
            decided.insert(iri, request_text(&prior_policy, &prior_target));
        }
    }
    if decided.is_empty() {
        return Ok(Vec::new());
    }

    let (iris, texts): (Vec<String>, Vec<String>) = decided.into_iter().unzip();
    let scores = method.score_many(&request_text(policy_iri, target_iri), &texts)?;
    let identity = method.identity();
    let mut found: Vec<Precedent> = iris
        .into_iter()
        .zip(scores)
        // Zero similarity is not weak precedent, it is no precedent — and a
        // non-finite score is a method malfunction, not a fact to record.
        .filter(|(_, score)| score.is_finite() && *score > 0.0)
        .map(|(request_iri, score)| Precedent {
            request_iri,
            score,
            method: identity.clone(),
        })
        .collect();
    found.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.request_iri.cmp(&b.request_iri))
    });
    found.truncate(MAX_PRECEDENTS);
    Ok(found)
}

/// What similarity compares: the same `policy|target` identity the evidence
/// hash canonicalises — the request's whole subject, and nothing that is not
/// on the request.
fn request_text(policy_iri: &str, target_iri: &str) -> String {
    format!("{policy_iri}|{target_iri}")
}

/// The datums linking `subject` to its precedents, one reified
/// `aegis:PrecedentLink` per precedent so every score stays paired with the
/// prior it scores. Link IRIs are content-derived (own hash, prior, method,
/// score bits), so a retry that finds the same precedent re-asserts the same
/// node instead of accumulating one per attempt — while a re-mint whose
/// corpus HAS changed mints new links rather than piling a second score onto
/// an old one.
fn datums_for(
    store: &Store,
    subject: i64,
    policy_iri: &str,
    target_iri: &str,
    precedents: &[Precedent],
    timestamp: &str,
) -> Result<Vec<Datum>> {
    let own_hash = evidence_hash(policy_iri, target_iri);
    let mut datums = Vec::new();
    for p in precedents {
        let canonical = format!(
            "precedent-v1|{own_hash}|{prior}|{method}|{bits:016x}",
            prior = p.request_iri,
            method = p.method,
            bits = p.score.to_bits()
        );
        let digest = ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes());
        let hex = hex::encode(digest.as_ref());
        let link = store.intern(&format!("{DEFAULT_BASE_NS}precedent_{}", &hex[..24]))?;
        let assert = |entity: i64, attribute: i64, value: Value| Datum {
            entity,
            attribute,
            value,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        };
        datums.push(assert(
            link,
            store.intern(RDF_TYPE)?,
            Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}PrecedentLink"))?),
        ));
        datums.push(assert(
            link,
            store.intern(&format!("{DEFAULT_BASE_NS}precedentRequest"))?,
            Value::Ref(store.intern(&p.request_iri)?),
        ));
        datums.push(assert(
            link,
            store.intern(&format!("{DEFAULT_BASE_NS}similarityScore"))?,
            Value::Float(p.score),
        ));
        datums.push(assert(
            link,
            store.intern(&format!("{DEFAULT_BASE_NS}similarityMethod"))?,
            Value::Str(p.method.clone()),
        ));
        datums.push(assert(
            subject,
            store.intern(&format!("{DEFAULT_BASE_NS}precedent"))?,
            Value::Ref(link),
        ));
    }
    Ok(datums)
}

fn str_of(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn iri_of(store: &Store, v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Ref(id)) => store.resolve(*id).ok(),
        _ => None,
    }
}
