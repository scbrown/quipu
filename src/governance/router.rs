//! The Escalation Router — turning a refusal into something an operator can act on.
//!
//! `Q-SARC-ER`, closing SARC I4. `require-approval` and `escalate` failed closed
//! at the write gate **with no channel to grant approval**: the write was
//! refused, no record said what would un-refuse it, and no bound said when the
//! absence of a ruling became an answer. A refusal an operator cannot act on is
//! a worse control than an honest advisory, because it looks like governance
//! while functioning as an outage.
//!
//! ## Asynchronous, and honest about it
//!
//! A write gate is synchronous. Nothing here holds a transaction open while a
//! human decides — that would convert an approval gate into a lock on the store.
//! What the router does instead:
//!
//! 1. The refused attempt **mints a `DecisionRequest`** naming the policy, the
//!    target, the operator group that can rule, an evidence hash, and an
//!    `expiresAt` derived from the constraint's reversibility window.
//! 2. A human signs an `aegis:Decision` bound to the same evidence hash.
//! 3. The **next attempt** finds the approval and proceeds.
//!
//! So the "hold" is the agent retrying, not the engine waiting. SARC's §5.3
//! reversibility window still governs: past `expiresAt` an unserviced request is
//! a DENIAL, not a request that quietly stays open. That denial closes the
//! REQUEST, not the channel: the next refused attempt re-mints a fresh request
//! (same deterministic IRI, new `expiresAt`) so an operator who missed the
//! window can still rule. Only a recorded rejection is terminal — that is an
//! answer, not an absence of one.
//!
//! ## Content binding, and why approval cannot be reused
//!
//! A `Decision` satisfies a request only when its `evidenceHash` matches. Change
//! what was approved and the hash changes, so the approval no longer applies —
//! this is the approve-then-sneak-in-changes defence, and it is the same
//! mechanism a Verdict uses, not a second one invented for decisions.
//!
//! ## Signed, and why write access is not authority
//!
//! A decision is an ATTESTATION, not a fact. Decisions are ordinary writable
//! facts, so before quipu-5s5 the agent whose write was refused could write its
//! own `aegis:Decision "approve"` and walk through. The router now trusts a
//! decision only when it is **signed by a registered decider**: the row must
//! carry an `aegis:signature` over [`decision_message`], verifying under the
//! `aegis:publicKey` of an `aegis:VerifierRegistration` whose `aegis:verifier`
//! is the decision's `aegis:by` and whose `aegis:attests` names the policy —
//! the same human-owned root of trust the verdict plane uses (`signing.rs`),
//! not a second one. An unsigned or unverifiable decision is not a wrong
//! ruling; it is no ruling at all, and resolution falls through to
//! `Pending`/`Expired` — which are refusals, so a forgery can never widen
//! access.
//!
//! ## What this is not
//!
//! It is not a queue with a scheduler, and it does not notify anybody. Routing
//! records WHICH group should rule; delivering the request to them is a consumer
//! of this record, not a thing quipu does. Saying otherwise would be the
//! dashboard anti-pattern with extra steps.

use crate::error::Result;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// What the router concluded about an escalated action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ruling {
    /// A human approved it, bound to this evidence. The write proceeds.
    Approved {
        /// Who signed.
        by: String,
    },
    /// A human refused it. Distinct from `Pending`: this is an answer.
    Rejected {
        /// Who signed.
        by: String,
        /// The outcome they gave — `reject` or `changes`.
        outcome: String,
    },
    /// No ruling yet, and the window has not closed. The write is refused, and
    /// a request exists that a human can act on.
    Pending {
        /// Unix seconds at which an unserviced request becomes a denial.
        expires_at: i64,
    },
    /// The reversibility window closed with no ruling. **Denied**, per SARC
    /// §5.3's declared default-deny — never a quiet pass.
    Expired,
}

impl Ruling {
    /// Whether the write may proceed. Only an explicit approval qualifies:
    /// `Pending` and `Expired` are both refusals, and reading either as a pass
    /// is the default-allow-under-load failure the whole design excludes.
    #[must_use]
    pub fn permits(&self) -> bool {
        matches!(self, Ruling::Approved { .. })
    }

    /// The operator-facing reason, naming what would resolve it.
    #[must_use]
    pub fn reason(&self, policy: &str, target: &str) -> String {
        match self {
            Ruling::Approved { by } => format!("approved by {by}"),
            Ruling::Rejected { by, outcome } => {
                format!("'{target}' was refused by {by} ({outcome}) under policy '{policy}'")
            }
            Ruling::Pending { expires_at } => format!(
                "'{target}' needs a human decision under policy '{policy}'. A \
                 DecisionRequest is open and expires at {expires_at} (unix \
                 seconds), after which it is DENIED. To proceed, have a \
                 registered decider record a signed aegis:Decision with outcome \
                 \"approve\" bound to this request's evidenceHash, then retry."
            ),
            Ruling::Expired => format!(
                "'{target}' needed a human decision under policy '{policy}' and \
                 the reversibility window closed with no ruling, so it is denied. \
                 This is a declared default-deny, not a timeout to retry through: \
                 a new attempt opens a new request."
            ),
        }
    }
}

/// Resolve an escalation for `(policy_iri, target_iri)` at `now`.
///
/// Reads any existing request and any `Decision` bound to its evidence hash.
/// Pure read — minting the request is [`mint_request`], kept separate because
/// the gate stages writes rather than performing them mid-transaction.
pub fn resolve(
    store: &Store,
    policy_iri: &str,
    target_iri: &str,
    now: i64,
) -> Result<Option<Ruling>> {
    let hash = evidence_hash(policy_iri, target_iri);
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?expires WHERE {{ \
            ?r a a:DecisionRequest ; a:forPolicy \"{policy}\" ; \
               a:forTarget \"{target}\" ; a:expiresAt ?expires . \
         }}",
        policy = escape(policy_iri),
        target = escape(target_iri),
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &q)? else {
        return Ok(None);
    };
    // No request => nothing has been escalated yet.
    let Some(expires_at) = rows.iter().filter_map(|r| int_of(r.get("expires"))).max() else {
        return Ok(None);
    };

    // A ruling, if one is bound to THIS evidence. A decision over different
    // evidence is a decision about something else — and an unsigned or
    // unverifiable one is no ruling at all (see the module doc): decisions are
    // ordinary writable facts, so without the signature check the refused
    // agent could approve its own request.
    let dq = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?outcome ?by ?sig WHERE {{ \
            ?d a a:Decision ; a:evidenceHash \"{hash}\" ; a:outcome ?outcome ; a:by ?by . \
            OPTIONAL {{ ?d a:signature ?sig }} \
         }}",
        hash = escape(&hash),
    );
    if let QueryResult::Select { rows, .. } = sparql::query(store, &dq)? {
        // A rejection outranks an approval when both exist. Two humans
        // disagreeing is not a state to resolve by row order, and the safe
        // reading of a disagreement about whether to permit something is "no".
        let mut approval: Option<Ruling> = None;
        for row in &rows {
            let (Some(outcome), Some(by)) = (str_of(row.get("outcome")), str_of(row.get("by")))
            else {
                continue;
            };
            let Some(sig) = str_of(row.get("sig")) else {
                continue;
            };
            if !decision_verifies(store, policy_iri, &hash, &outcome, &by, &sig)? {
                continue;
            }
            match outcome.as_str() {
                "approve" => approval.get_or_insert(Ruling::Approved { by }),
                _ => return Ok(Some(Ruling::Rejected { by, outcome })),
            };
        }
        if let Some(approved) = approval {
            return Ok(Some(approved));
        }
    }

    if now >= expires_at {
        return Ok(Some(Ruling::Expired));
    }
    Ok(Some(Ruling::Pending { expires_at }))
}

/// A request the gate decided to open but has not yet written.
///
/// Staged for the same reason verdicts are: the gate runs inside the savepoint
/// that this refusal is about to roll back, so a request written in place would
/// vanish along with it — leaving the operator a refusal with no request to act
/// on, which is exactly the state the router exists to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// The policy that escalated.
    pub policy_iri: String,
    /// The entity whose write was suspended.
    pub target_iri: String,
    /// The constraint's declared reversibility window, seconds.
    pub window_secs: i64,
    /// When the escalation happened.
    pub now: i64,
}

/// The datums that mint a `DecisionRequest` for this escalation.
///
/// `window_secs` is the constraint's declared reversibility window. There is no
/// default: a policy that escalates without declaring one fails the placement
/// check at definition time, so reaching here without a window means the check
/// was off — and inventing a window would be inventing the bound I4 requires be
/// declared.
pub fn mint_request(
    store: &Store,
    policy_iri: &str,
    target_iri: &str,
    group_iri: Option<&str>,
    window_secs: i64,
    now: i64,
    timestamp: &str,
) -> Result<Vec<Datum>> {
    let hash = evidence_hash(policy_iri, target_iri);
    // Deterministic IRI from the evidence, so a retry that is still refused
    // updates the same request rather than accumulating one per attempt.
    let subject = store.intern(&format!(
        "{DEFAULT_BASE_NS}decision_request_{}",
        &hash[7..hash.len().min(39)]
    ))?;
    let assert = |attribute: i64, value: Value| Datum {
        entity: subject,
        attribute,
        value,
        valid_from: timestamp.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    let mut datums = vec![assert(
        store.intern(RDF_TYPE)?,
        Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}DecisionRequest"))?),
    )];
    for (field, value) in [
        ("forPolicy", Value::Str(policy_iri.to_string())),
        ("forTarget", Value::Str(target_iri.to_string())),
        ("evidenceHash", Value::Str(hash)),
        ("expiresAt", Value::Int(now.saturating_add(window_secs))),
    ] {
        datums.push(assert(
            store.intern(&format!("{DEFAULT_BASE_NS}{field}"))?,
            value,
        ));
    }
    if let Some(group) = group_iri {
        datums.push(assert(
            store.intern(&format!("{DEFAULT_BASE_NS}routedTo"))?,
            Value::Ref(store.intern(group)?),
        ));
    }
    Ok(datums)
}

/// The canonical byte string a decider signs. Deterministic field order so any
/// consumer re-derives the exact same message from the decision's own fields
/// and checks the signature — checked, not trusted, same discipline as
/// [`crate::signing::verdict_message`].
#[must_use]
pub fn decision_message(evidence_hash: &str, outcome: &str, by: &str) -> Vec<u8> {
    format!("decision-v1|{evidence_hash}|{outcome}|{by}").into_bytes()
}

/// Whether a decision row is a trustworthy attestation: its signature must
/// verify over [`decision_message`] under the public key of an
/// `aegis:VerifierRegistration` whose `aegis:verifier` is the decision's `by`
/// and whose `aegis:attests` names the escalating policy. The registry is the
/// same human-owned root of trust the verdict plane uses; quipu never registers
/// deciders itself.
fn decision_verifies(
    store: &Store,
    policy_iri: &str,
    evidence_hash: &str,
    outcome: &str,
    by: &str,
    signature: &str,
) -> Result<bool> {
    let kq = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?k WHERE {{ \
            ?r a a:VerifierRegistration ; a:verifier \"{by}\" ; \
               a:attests \"{policy}\" ; a:publicKey ?k . \
         }}",
        by = escape(by),
        policy = escape(policy_iri),
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &kq)? else {
        return Ok(false);
    };
    let message = decision_message(evidence_hash, outcome, by);
    // Any registered key for this decider may verify — a decider mid key
    // rotation legitimately has two registrations, and requiring "the first
    // row" would make the ruling depend on row order.
    Ok(rows
        .iter()
        .filter_map(|r| str_of(r.get("k")))
        .any(|key| crate::signing::verify_hex(&key, &message, signature)))
}

/// The evidence a decision binds to: `sha256:<hex>` over `policy|target`.
///
/// Scoped to what the request actually identifies. Widening it to the graph
/// state would make every approval spuriously stale the moment anything
/// unrelated changed; narrowing it further would let one approval cover a
/// different target.
#[must_use]
pub fn evidence_hash(policy_iri: &str, target_iri: &str) -> String {
    let canonical = format!("{policy_iri}|{target_iri}");
    let digest = ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes());
    format!("sha256:{}", hex::encode(digest.as_ref()))
}

/// Escape a value for a double-quoted SPARQL literal.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn str_of(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn int_of(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Int(i)) => Some(*i),
        Some(Value::Str(s)) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
