//! Sealed decisions: sign the WHOLE decision, not a summary of it
//! (aegis-kzt0ql.9.3).
//!
//! `decision-v1` (the escalation router) signs `evidenceHash|outcome|by`. The
//! question, the options, the authorized scope and the expiry are not covered,
//! so they can be rewritten after a human signs and the ruling still stands.
//! This module seals the decision's CONTENT instead.
//!
//! # The flow
//!
//! 1. [`present`] freezes a decision: it computes [`decision_digest`] and mints
//!    a single-use nonce, both chosen by quipu, never by the requester. It
//!    records an `aegis:DecisionPresentation` and returns the challenge.
//! 2. The approver signs [`challenge_message`] over the digest, the chosen
//!    outcome, the nonce, the expiry and the purpose tag.
//! 3. [`attest`] RECOMPUTES the digest from the stored decision (a supplied
//!    hash is never accepted), checks it still equals the frozen one, checks
//!    the signature against a key registered now (S1), and then spends the
//!    nonce and records an `aegis:DecisionVerdict` in one savepoint.
//! 4. [`verify_recorded`] re-checks a recorded verdict later: it recomputes
//!    the digest from the decision as it stands NOW (an edit after signing
//!    invalidates the verdict) and verifies the signature against the key
//!    registered when the store recorded it. It also requires that `attest`
//!    admitted this verdict: it was recorded before the presentation expired,
//!    its nonce was spent FOR it, and its facts were written by the spending
//!    transaction. Until S3 (.9.4) verdicts are graph-writable, so without
//!    this a verdict hand-written with a captured signature would skip every
//!    attest-time gate (wu-rev-345 F1).
//!
//! # What the digest covers
//!
//! The decision's concise bounded description in ROOT: every current fact
//! whose subject is the decision, plus, recursively, the facts of blank nodes
//! it reaches (so option lists written as blank nodes are covered). Only the
//! attestation fields ([`UNSEALED`]) are excluded. The triples are serialized
//! as N-Triples and canonicalized with W3C RDFC-1.0, so the digest does not
//! depend on fact order or blank-node labels. It is `sha256:<hex>` of the
//! canonical bytes.
//!
//! Limits (wu-rev-345 F2, F4): an IRI object is sealed BY REFERENCE, so a
//! change to the referenced entity's own facts (an authorized action, a
//! warrant scope) is not detected; facts about the decision in NAMED graphs
//! are not sealed; and the digest depends on the lexical form quipu emits
//! for literals, so a future re-encoding turns open presentations into
//! [`Refusal::ContentChanged`] (fail-closed).
//!
//! `now` in [`present`] and [`attest`] MUST be the server clock when these
//! are wired to MCP or REST, never caller input, or the expiry becomes
//! caller-controlled (F3).
//!
//! # Vocabulary
//!
//! The seal uses its own predicates where a shared one would carry an
//! `rdfs:domain`, because inference would otherwise re-type the seal's
//! entities: `aegis:forPolicy` and `aegis:expiresAt` imply
//! `aegis:DecisionRequest`, and `aegis:signature` implies `aegis:Verdict`. So a
//! decision names `aegis:decisionPolicy`, a presentation
//! `aegis:presentationExpiresAt`, and a verdict `aegis:sealSignature`.

use std::collections::BTreeSet;

use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use super::verifier_registry::{Scope, Witness, registered_keys};

/// The purpose tag bound into every challenge, so a signature made for a
/// sealed decision cannot be replayed as any other kind of attestation.
pub const PURPOSE: &str = "quipu-verdict";
/// The challenge format.
pub const SCHEME: &str = "quipu-decision-v2";

/// Fields written about a decision when it is attested, not what was decided.
/// These are excluded from the digest.
const UNSEALED: [&str; 2] = [
    "http://aegis.gastown.local/ontology/signature",
    "http://www.w3.org/ns/prov#wasGeneratedBy",
];

fn ns(name: &str) -> String {
    format!("{DEFAULT_BASE_NS}{name}")
}

/// `sha256:<hex>` over the RDFC-1.0 canonical form of the decision's content.
pub fn decision_digest(store: &Store, decision: &str) -> Result<String> {
    let entity = store
        .lookup(decision)?
        .ok_or_else(|| Error::InvalidValue(format!("decision '{decision}' does not exist")))?;
    let mut lines = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = vec![entity];
    while let Some(subject) = queue.pop() {
        if !seen.insert(subject) {
            continue;
        }
        let subject_term = crate::rdf::value_to_term(store, &Value::Ref(subject))?;
        for fact in store.entity_facts(subject)? {
            let predicate = store.resolve(fact.attribute)?;
            if subject == entity && UNSEALED.contains(&predicate.as_str()) {
                continue;
            }
            if let Value::Ref(object) = &fact.value
                && store.resolve(*object)?.starts_with("_:")
            {
                queue.push(*object);
            }
            let object_term = crate::rdf::value_to_term(store, &fact.value)?;
            lines.push(format!("{subject_term} <{predicate}> {object_term} .\n"));
        }
    }
    if lines.is_empty() {
        return Err(Error::InvalidValue(format!(
            "decision '{decision}' has no content to seal"
        )));
    }
    let canonical = crate::share::canonicalize_ntriples(lines.concat().as_bytes())?;
    Ok(crate::share::sha256(&canonical))
}

/// The canonical bytes an approver signs. Every field is fixed by quipu except
/// `outcome`, which is the approver's choice and is therefore covered too.
#[must_use]
pub fn challenge_message(
    decision: &str,
    digest: &str,
    outcome: &str,
    nonce: &str,
    expires_at: i64,
) -> Vec<u8> {
    format!("{SCHEME}|{PURPOSE}|{decision}|{digest}|{outcome}|{nonce}|{expires_at}").into_bytes()
}

/// A frozen decision, ready to be signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presentation {
    /// The presentation's IRI.
    pub iri: String,
    /// The decision it freezes.
    pub decision: String,
    /// The digest frozen at presentation.
    pub digest: String,
    /// The single-use nonce quipu minted.
    pub nonce: String,
    /// Epoch seconds after which it can no longer be answered.
    pub expires_at: i64,
}

impl Presentation {
    /// The message to sign for `outcome`.
    #[must_use]
    pub fn challenge(&self, outcome: &str) -> Vec<u8> {
        challenge_message(
            &self.decision,
            &self.digest,
            outcome,
            &self.nonce,
            self.expires_at,
        )
    }
}

/// Freeze `decision` for `ttl_secs`: record its digest and a fresh nonce.
pub fn present(store: &mut Store, decision: &str, ttl_secs: i64, now: i64) -> Result<Presentation> {
    if ttl_secs <= 0 {
        return Err(Error::InvalidValue("ttl_secs must be positive".into()));
    }
    let digest = decision_digest(store, decision)?;
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| Error::InvalidValue("could not draw a nonce".into()))?;
    let nonce = hex::encode(bytes);
    let presentation = Presentation {
        iri: ns(&format!("presentation_{nonce}")),
        decision: decision.to_string(),
        digest,
        nonce,
        expires_at: now + ttl_secs,
    };
    let ts = crate::time::format_iso(u64::try_from(now).unwrap_or(0));
    let d = |store: &Store, p: &str, v: Value| -> Result<Datum> {
        Ok(Datum {
            entity: store.intern(&presentation.iri)?,
            attribute: store.intern(p)?,
            value: v,
            valid_from: ts.clone(),
            valid_to: None,
            op: Op::Assert,
        })
    };
    let datums = vec![
        d(
            store,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("DecisionPresentation"))?),
        )?,
        d(
            store,
            &ns("forDecision"),
            Value::Ref(store.intern(decision)?),
        )?,
        d(
            store,
            &ns("sealedDigest"),
            Value::Str(presentation.digest.clone()),
        )?,
        d(store, &ns("nonce"), Value::Str(presentation.nonce.clone()))?,
        d(
            store,
            &ns("presentationExpiresAt"),
            Value::Int(presentation.expires_at),
        )?,
        d(store, &ns("purpose"), Value::Str(PURPOSE.into()))?,
    ];
    store.transact(&datums, &ts, None, Some("decision-seal"))?;
    Ok(presentation)
}

/// Why an attestation or a re-verification was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No presentation with that nonce, or it is not for a decision.
    UnknownPresentation,
    /// The presentation has expired.
    Expired,
    /// The decision changed after it was presented (or after it was signed).
    ContentChanged {
        /// The digest that was frozen and signed.
        sealed: String,
        /// The digest the stored decision has now.
        current: String,
    },
    /// The outcome is not one of the decision's declared `aegis:option`s.
    NotAnOption,
    /// The decision names no `aegis:decisionPolicy` for a verifier to be authorized for.
    NoPolicy,
    /// No key registered to the verifier for that policy verifies the signature.
    BadSignature,
    /// The nonce was already spent: a replay.
    Replayed,
    /// The verdict was not admitted by [`attest`]: its nonce was never spent,
    /// the spend admitted a different verdict, or the verdict's facts were
    /// written by some other transaction.
    NotAttested,
}

/// Accept a signed verdict over `nonce`'s presentation. Returns the verdict
/// IRI, or why it was refused. Nothing is written on a refusal.
pub fn attest(
    store: &mut Store,
    nonce: &str,
    outcome: &str,
    verifier: &str,
    signature: &str,
    now: i64,
) -> Result<std::result::Result<String, Refusal>> {
    let Some(p) = read_presentation(store, nonce)? else {
        return Ok(Err(Refusal::UnknownPresentation));
    };
    if now >= p.expires_at {
        return Ok(Err(Refusal::Expired));
    }
    if let Err(refusal) = check_content(store, &p.decision, &p.digest, outcome)? {
        return Ok(Err(refusal));
    }
    let Some(policy) = scalar(store, &p.decision, &ns("decisionPolicy"))? else {
        return Ok(Err(Refusal::NoPolicy));
    };
    let message = p.challenge(outcome);
    let keys = registered_keys(store, verifier, Some(&policy), &Witness::now(), Scope::Root)?;
    if !keys
        .iter()
        .any(|k| crate::signing::verify_hex(k, &message, signature))
    {
        return Ok(Err(Refusal::BadSignature));
    }

    let verdict = ns(&format!("decision_verdict_{nonce}"));
    let ts = crate::time::format_iso(u64::try_from(now).unwrap_or(0));
    let d = |store: &Store, pr: &str, v: Value| -> Result<Datum> {
        Ok(Datum {
            entity: store.intern(&verdict)?,
            attribute: store.intern(pr)?,
            value: v,
            valid_from: ts.clone(),
            valid_to: None,
            op: Op::Assert,
        })
    };
    let datums = vec![
        d(
            store,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("DecisionVerdict"))?),
        )?,
        d(
            store,
            &ns("forPresentation"),
            Value::Ref(store.intern(&p.iri)?),
        )?,
        d(store, &ns("outcome"), Value::Str(outcome.into()))?,
        d(store, &ns("verifier"), Value::Str(verifier.into()))?,
        d(store, &ns("sealSignature"), Value::Str(signature.into()))?,
    ];
    match store.transact_spending_decision_nonce(
        nonce,
        &p.decision,
        &verdict,
        &datums,
        &ts,
        Some(verifier),
        Some("decision-seal"),
    )? {
        Some(_) => Ok(Ok(verdict)),
        None => Ok(Err(Refusal::Replayed)),
    }
}

/// The verdict facts `attest` writes and a re-verification relies on. Each
/// must still be the one the spending transaction wrote.
const ATTESTED: [&str; 4] = ["forPresentation", "outcome", "verifier", "sealSignature"];

/// Re-verify a recorded verdict: the decision must still have the sealed
/// content, the verdict must be the one `attest` admitted before the
/// presentation expired, and the signature must verify under a key
/// registered, for the decision's policy, when the store recorded it.
pub fn verify_recorded(store: &Store, verdict: &str) -> Result<std::result::Result<(), Refusal>> {
    let Some(presentation) = ref_of(store, verdict, &ns("forPresentation"))? else {
        return Ok(Err(Refusal::UnknownPresentation));
    };
    let Some(nonce) = scalar(store, &presentation, &ns("nonce"))? else {
        return Ok(Err(Refusal::UnknownPresentation));
    };
    let Some(p) = read_presentation(store, &nonce)? else {
        return Ok(Err(Refusal::UnknownPresentation));
    };
    let (Some(outcome), Some(verifier), Some(signature)) = (
        scalar(store, verdict, &ns("outcome"))?,
        scalar(store, verdict, &ns("verifier"))?,
        scalar(store, verdict, &ns("sealSignature"))?,
    ) else {
        return Ok(Err(Refusal::BadSignature));
    };
    if let Err(refusal) = check_content(store, &p.decision, &p.digest, &outcome)? {
        return Ok(Err(refusal));
    }
    let Some(policy) = scalar(store, &p.decision, &ns("decisionPolicy"))? else {
        return Ok(Err(Refusal::NoPolicy));
    };
    let Some(witness) = Witness::of_fact(store, verdict, &ns("sealSignature"), &signature)? else {
        return Ok(Err(Refusal::BadSignature));
    };
    // Recorded before the presentation expired. An unparseable instant fails
    // closed. Both sides are canonical `YYYY-MM-DDTHH:MM:SSZ`, so they order
    // as strings.
    let deadline = crate::time::format_iso(u64::try_from(p.expires_at).unwrap_or(0));
    match crate::time::normalize_rfc3339_utc(&witness.at) {
        Some(at) if at < deadline => {}
        _ => return Ok(Err(Refusal::Expired)),
    }
    // Admitted by attest: the one verdict this nonce's spend names, written
    // by the spending transaction.
    if verdict != ns(&format!("decision_verdict_{nonce}")) {
        return Ok(Err(Refusal::NotAttested));
    }
    let Some(spend) = store.decision_nonce_spend(&nonce)? else {
        return Ok(Err(Refusal::NotAttested));
    };
    let Some(spend_tx) = spend.tx else {
        return Ok(Err(Refusal::NotAttested));
    };
    if spend.decision != p.decision || spend.verdict != verdict || witness.tx != Some(spend_tx) {
        return Ok(Err(Refusal::NotAttested));
    }
    for field in ATTESTED {
        if sole_fact_tx(store, verdict, &ns(field))? != Some(spend_tx) {
            return Ok(Err(Refusal::NotAttested));
        }
    }
    let message = p.challenge(&outcome);
    let keys = registered_keys(store, &verifier, Some(&policy), &witness, Scope::Root)?;
    if keys
        .iter()
        .any(|k| crate::signing::verify_hex(k, &message, &signature))
    {
        Ok(Ok(()))
    } else {
        Ok(Err(Refusal::BadSignature))
    }
}

/// The digest recomputed from the stored decision must equal the sealed one,
/// and `outcome` must be a declared option when the decision declares any.
fn check_content(
    store: &Store,
    decision: &str,
    sealed: &str,
    outcome: &str,
) -> Result<std::result::Result<(), Refusal>> {
    let current = decision_digest(store, decision)?;
    if current != sealed {
        return Ok(Err(Refusal::ContentChanged {
            sealed: sealed.to_string(),
            current,
        }));
    }
    let options = scalars(store, decision, &ns("option"))?;
    if !options.is_empty() && !options.iter().any(|o| o == outcome) {
        return Ok(Err(Refusal::NotAnOption));
    }
    Ok(Ok(()))
}

fn read_presentation(store: &Store, nonce: &str) -> Result<Option<Presentation>> {
    let iri = ns(&format!("presentation_{nonce}"));
    let (Some(decision), Some(digest), Some(expires_at)) = (
        ref_of(store, &iri, &ns("forDecision"))?,
        scalar(store, &iri, &ns("sealedDigest"))?,
        int_of(store, &iri, &ns("presentationExpiresAt"))?,
    ) else {
        return Ok(None);
    };
    if scalar(store, &iri, &ns("nonce"))?.as_deref() != Some(nonce)
        || scalar(store, &iri, &ns("purpose"))?.as_deref() != Some(PURPOSE)
    {
        return Ok(None);
    }
    Ok(Some(Presentation {
        iri,
        decision,
        digest,
        nonce: nonce.to_string(),
        expires_at,
    }))
}

/// Current ROOT values of `subject`'s `predicate`.
fn values(store: &Store, subject: &str, predicate: &str) -> Result<Vec<Value>> {
    let (Some(e), Some(a)) = (store.lookup(subject)?, store.lookup(predicate)?) else {
        return Ok(Vec::new());
    };
    Ok(store
        .entity_facts(e)?
        .into_iter()
        .filter(|f| f.attribute == a)
        .map(|f| f.value)
        .collect())
}

/// The transaction that wrote `subject`'s single current `predicate` fact;
/// none when there is no such fact or more than one.
fn sole_fact_tx(store: &Store, subject: &str, predicate: &str) -> Result<Option<i64>> {
    let (Some(e), Some(a)) = (store.lookup(subject)?, store.lookup(predicate)?) else {
        return Ok(None);
    };
    let txs: Vec<i64> = store
        .entity_facts(e)?
        .into_iter()
        .filter(|f| f.attribute == a)
        .map(|f| f.tx)
        .collect();
    Ok(match txs.as_slice() {
        [one] => Some(*one),
        _ => None,
    })
}

fn scalars(store: &Store, subject: &str, predicate: &str) -> Result<Vec<String>> {
    Ok(values(store, subject, predicate)?
        .into_iter()
        .filter_map(|v| match v {
            Value::Str(s) | Value::Typed { lexical: s, .. } => Some(s),
            _ => None,
        })
        .collect())
}

/// A single-valued string field; several values are ambiguous and read as none.
fn scalar(store: &Store, subject: &str, predicate: &str) -> Result<Option<String>> {
    let mut all = scalars(store, subject, predicate)?;
    Ok(if all.len() == 1 { all.pop() } else { None })
}

fn ref_of(store: &Store, subject: &str, predicate: &str) -> Result<Option<String>> {
    let refs: Vec<i64> = values(store, subject, predicate)?
        .into_iter()
        .filter_map(|v| match v {
            Value::Ref(id) => Some(id),
            _ => None,
        })
        .collect();
    match refs.as_slice() {
        [one] => Ok(Some(store.resolve(*one)?)),
        _ => Ok(None),
    }
}

fn int_of(store: &Store, subject: &str, predicate: &str) -> Result<Option<i64>> {
    let ints: Vec<i64> = values(store, subject, predicate)?
        .into_iter()
        .filter_map(|v| match v {
            Value::Int(i) => Some(i),
            _ => None,
        })
        .collect();
    Ok(match ints.as_slice() {
        [one] => Some(*one),
        _ => None,
    })
}

#[cfg(test)]
#[path = "decision_seal_tests.rs"]
mod tests;
