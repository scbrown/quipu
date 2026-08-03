//! Persisting the write-gate's decision as a signed `aegis:Verdict` fact.
//!
//! `Q-VERDICT-PERSIST`. The gate returned accept-or-reject and recorded nothing:
//! an enforced decision left no auditable trace, so "did this policy ever
//! actually stop anything?" was unanswerable from the graph the policy lives in.
//!
//! ## The ordering problem, which is the whole design
//!
//! A denied write is ROLLED BACK. Verdicts written inside the same savepoint go
//! with it — and the verdict of a denial is precisely the one worth keeping,
//! because an accepted write leaves its own evidence in the facts it wrote while
//! a refused one leaves nothing at all.
//!
//! So verdicts are COLLECTED during evaluation and written AFTERWARDS, in their
//! own transaction, once the savepoint has resolved either way. That is why they
//! are staged on the [`Store`] rather than emitted where they are computed.
//!
//! ## Re-entry
//!
//! Writing a verdict is itself a write, and the gate would evaluate it. Left
//! alone that is at best wasted work and at worst a loop: a policy targeting
//! `aegis:Verdict` would deny the verdict recording the denial. The recording
//! path sets a flag the gate honours, so verdict writes are ungoverned by
//! construction — a deliberate hole, and a narrow one: it applies only to facts
//! this module builds, and only for the duration of the write.

use crate::error::Result;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// The tier a write-gate verdict attests at. The gate reads the committed
/// graph — not a live buffer, not an approximation — so `committed` is what it
/// can honestly claim.
pub const TIER: &str = "committed";

/// A verdict the gate decided but has not yet written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingVerdict {
    /// The policy that was evaluated.
    pub predicate_id: String,
    /// The entity it was evaluated against.
    pub target_ref: String,
    /// `satisfied`, `unsatisfied`, or `unknown` — the last when an evidence
    /// probe found nothing to judge, which the gate already distinguishes and
    /// which must not collapse into a pass.
    pub outcome: String,
}

impl PendingVerdict {
    /// The evidence hash bound into the verdict: `sha256:<hex>` over the
    /// canonical `predicate|target|outcome` triple.
    ///
    /// NOT a hash of the graph state. The gate's evidence is a SPARQL ASK over
    /// the committed store, which has no stable serialisation to hash, and
    /// inventing one that changed with unrelated facts would make every verdict
    /// spuriously stale. Hashing what the verdict actually asserts keeps the
    /// binding honest about its own scope — narrower than hank's, which hashes
    /// the edit text it really did see.
    #[must_use]
    pub fn evidence_hash(&self) -> String {
        // ring, not sha2: the signing path already depends on it, and adding a
        // second hashing crate for one call would be a dependency nobody needs.
        let canonical = format!("{}|{}|{}", self.predicate_id, self.target_ref, self.outcome);
        let digest = ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes());
        format!("sha256:{}", hex::encode(digest.as_ref()))
    }
}

/// Build the datums for one signed verdict, or `None` when the store has no
/// signing identity.
///
/// No identity means no verdict — never an unsigned one. A bare `satisfied`
/// written into the record is forgeable by anyone who can write a fact, and the
/// entire point of the verdict is that it is an attestation rather than a claim.
pub fn datums_for(store: &Store, verdict: &PendingVerdict, timestamp: &str) -> Result<Vec<Datum>> {
    let Some(identity) = store.signing_identity() else {
        return Ok(Vec::new());
    };
    let hash = verdict.evidence_hash();
    let signature = identity.sign_verdict(
        &verdict.predicate_id,
        &verdict.target_ref,
        &verdict.outcome,
        &hash,
        TIER,
    );
    // A stable IRI from the signature, so re-recording the same decision over
    // the same evidence is idempotent by content rather than accumulating a row
    // per evaluation.
    let id = &signature[..signature.len().min(32)];
    let subject = store.intern(&format!("{DEFAULT_BASE_NS}verdict_{id}"))?;

    let mut datums = vec![Datum {
        entity: subject,
        attribute: store.intern(RDF_TYPE)?,
        value: Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Verdict"))?),
        valid_from: timestamp.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    for (field, value) in [
        ("predicateId", verdict.predicate_id.clone()),
        ("targetRef", verdict.target_ref.clone()),
        ("outcome", verdict.outcome.clone()),
        ("evidenceHash", hash),
        ("verifier", identity.verifier.clone()),
        ("signature", signature),
        ("tier", TIER.to_string()),
    ] {
        datums.push(Datum {
            entity: subject,
            attribute: store.intern(&format!("{DEFAULT_BASE_NS}{field}"))?,
            value: Value::Str(value),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    Ok(datums)
}

#[cfg(test)]
#[path = "verdict_facts_tests.rs"]
mod tests;
