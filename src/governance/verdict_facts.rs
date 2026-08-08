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
    /// canonical `predicate|target|outcome|writer|chain` tuple, where `writer`
    /// is the write's actor (empty when none was presented) and `chain` the
    /// comma-joined principal chain in force.
    ///
    /// NOT a hash of the graph state. The gate's evidence is a SPARQL ASK over
    /// the committed store, which has no stable serialisation to hash, and
    /// inventing one that changed with unrelated facts would make every verdict
    /// spuriously stale. Hashing what the verdict actually asserts keeps the
    /// binding honest about its own scope — and since Q-VERDICT-ATTRIB the
    /// verdict asserts its attribution too: a signature that stopped at the
    /// outcome would leave "who" swappable under a valid seal.
    #[must_use]
    pub fn evidence_hash(&self, writer: Option<&str>, chain: &[String]) -> String {
        // ring, not sha2: the signing path already depends on it, and adding a
        // second hashing crate for one call would be a dependency nobody needs.
        let canonical = format!(
            "{}|{}|{}|{}|{}",
            self.predicate_id,
            self.target_ref,
            self.outcome,
            writer.unwrap_or(""),
            chain.join(",")
        );
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
///
/// `Q-VERDICT-ATTRIB`: the verdict carries the write's attribution — the actor
/// presented at the gate and the principal chain in force — inside the evidence
/// hash the signature seals. A denial rolls its delta back (GS2, deliberately),
/// so before this the persisted record of a refusal had no actor at all:
/// "who was refused?" was answerable only from a writer-side trace the store
/// does not own. The attempt still is not kept; who made it now is.
pub fn datums_for(
    store: &Store,
    verdict: &PendingVerdict,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<Vec<Datum>> {
    let Some(identity) = store.signing_identity() else {
        return Ok(Vec::new());
    };
    let chain = store.principal_chain();
    let hash = verdict.evidence_hash(actor, chain);
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
    let mut fields = vec![
        ("predicateId", verdict.predicate_id.clone()),
        ("targetRef", verdict.target_ref.clone()),
        ("outcome", verdict.outcome.clone()),
        ("evidenceHash", hash),
        ("verifier", identity.verifier.clone()),
        ("signature", signature),
        ("tier", TIER.to_string()),
    ];
    // Absence stays visible as absence: an unattributed write records no
    // attribution fields rather than an empty string dressed up as one.
    if let Some(actor) = actor {
        fields.push(("attributedWriter", actor.to_string()));
    }
    if !chain.is_empty() {
        fields.push(("principalChain", chain.join(",")));
    }
    for (field, value) in fields {
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
