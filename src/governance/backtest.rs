//! Backtest — replaying a candidate policy over recorded history BEFORE it exists.
//!
//! Step 3 of `docs/design/policy-by-example.md`: *"this rule would have fired N
//! times in the last window — here are the hits."* The human sees the
//! false-positive surface before the rule is created, so threshold and shape
//! come from measurement rather than intuition.
//!
//! ## Why this is not `replay.rs`, and what it reuses instead
//!
//! [`super::replay`] is deterministic arithmetic over a RECORDED trace: it
//! re-reads what constraints already in Σ already did, and re-evaluates
//! nothing, because the evidence its predicates needed is gone. A candidate
//! policy appears in no trace — it did not exist to be evaluated — so trace
//! replay of a candidate would report zero forever, and a zero that means
//! "unmeasurable" printed as "0 hits" is exactly the fiction this module must
//! not produce.
//!
//! What the store DOES hold is its own bitemporal history: every transaction,
//! every fact's assert/retract tx, and an as-of-tx query mode
//! ([`crate::sparql::TemporalContext::as_of_tx`] — the same machinery
//! `replay_as_of` reads Σ-then through). For a candidate whose evidence is a
//! SPARQL claim, that IS a true retrospective evaluation: at each transaction
//! in the window, bind the claim to each target-typed entity that transaction
//! touched and ask it against the graph as it stood — precisely what the write
//! gate would have asked, had the policy existed then. So the backtest is a
//! thin composition of the gate's claim contract (`$target`, unsatisfied ⇒
//! fires) with the store's as-of read, not a second replay engine.
//!
//! ## What it honestly cannot do, said before any number
//!
//! - A candidate with **no SPARQL claim** (a structural selector/predicate
//!   policy) needs the file as it stood, and the store has neither the file
//!   nor the parser — `replay.rs`'s own limit. That candidate is reported
//!   **unevaluable**, loudly, never as "0 hits".
//! - Hits are counted for entities the window's transactions actually touched
//!   (the gate evaluates touched entities, nothing else). Traffic that never
//!   happened is not measured, and a quiet window is not evidence of safety.
//! - Every hit is a false-positive CANDIDATE. Whether a firing would have been
//!   wrong needs a human judgment no record contains; presenting the hit list
//!   is the point, pronouncing on it is not this module's right.

use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult, TemporalContext};
use crate::store::Store;

/// A candidate policy as the backtest needs it — the fields the write gate
/// would compile, read from a DRAFT that is deliberately not yet in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The policy IRI the draft declares.
    pub policy_iri: String,
    /// `aegis:targets` — which entity type the rule selects.
    pub target_type_iri: Option<String>,
    /// `aegis:claim` — the compliant condition, `None` when the draft carries
    /// none (which makes it unevaluable here, not silently clean).
    pub claim: Option<String>,
}

impl Candidate {
    /// Read a candidate from draft Turtle (the scaffold's output, or a
    /// hand-edited copy of it) WITHOUT ingesting anything: the whole point of
    /// a pre-creation backtest is that the store is untouched until the human
    /// has seen the hit list.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] when the Turtle does not parse, declares no
    /// `aegis:Policy`, or declares more than one (whose numbers would then be
    /// inseparable in the report).
    pub fn from_turtle(turtle: &str) -> Result<Self> {
        use oxrdf::{NamedOrBlankNode, Term, Triple};
        use oxrdfio::{RdfFormat, RdfParser};

        let policy_type = format!("{DEFAULT_BASE_NS}Policy");
        let targets_p = format!("{DEFAULT_BASE_NS}targets");
        let claim_p = format!("{DEFAULT_BASE_NS}claim");

        let mut policies: Vec<String> = Vec::new();
        let mut triples: Vec<(String, String, String)> = Vec::new();
        for quad in RdfParser::from_format(RdfFormat::Turtle).for_reader(turtle.as_bytes()) {
            let triple = Triple::from(
                quad.map_err(|e| Error::InvalidValue(format!("candidate Turtle: {e}")))?,
            );
            let NamedOrBlankNode::NamedNode(subject) = &triple.subject else {
                // A blank-node policy could not be cited by a refusal anyway.
                continue;
            };
            match &triple.object {
                Term::NamedNode(n) if triple.predicate.as_str() == RDF_TYPE => {
                    if n.as_str() == policy_type {
                        policies.push(subject.as_str().to_string());
                    }
                }
                Term::Literal(lit) => triples.push((
                    subject.as_str().to_string(),
                    triple.predicate.as_str().to_string(),
                    lit.value().to_string(),
                )),
                _ => {}
            }
        }
        let [policy_iri] = policies.as_slice() else {
            return Err(Error::InvalidValue(format!(
                "candidate Turtle declares {} aegis:Policy node(s); the backtest \
                 needs exactly one, or its numbers would answer for nobody. \
                 Draft one policy per file (quipu policy draft).",
                policies.len()
            )));
        };
        let field = |p: &str| {
            triples
                .iter()
                .find(|(s, pred, _)| s == policy_iri && pred == p)
                .map(|(_, _, v)| v.clone())
        };
        Ok(Self {
            policy_iri: policy_iri.clone(),
            target_type_iri: field(&targets_p),
            claim: field(&claim_p),
        })
    }
}

/// The transaction window to replay, inclusive on both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// First transaction id considered.
    pub from_tx: i64,
    /// Last transaction id considered.
    pub to_tx: i64,
}

impl Window {
    /// The last `n` committed transactions (all of them when fewer exist).
    ///
    /// # Errors
    /// Store errors reading the transaction log.
    pub fn last(store: &Store, n: i64) -> Result<Self> {
        let to_tx = store.latest_tx_id()?;
        Ok(Self {
            from_tx: (to_tx - n.max(1) + 1).max(1),
            to_tx,
        })
    }
}

/// One firing the candidate would have produced: which write, when, on what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The transaction whose write would have triggered the evaluation.
    pub tx: i64,
    /// That transaction's timestamp.
    pub timestamp: String,
    /// The entity the claim was unsatisfied for.
    pub target_iri: String,
}

impl Hit {
    /// The operator-facing line for this hit.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "tx {tx} ({ts}): would have fired on {target}",
            tx = self.tx,
            ts = self.timestamp,
            target = self.target_iri
        )
    }
}

/// What the window said about the candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestReport {
    /// The candidate's IRI.
    pub policy_iri: String,
    /// The window replayed.
    pub window: Window,
    /// Transactions inspected.
    pub transactions: usize,
    /// Claim evaluations actually run — (tx, touched target-typed entity)
    /// pairs, the same population the gate would have evaluated.
    pub evaluations: usize,
    /// Every firing, in tx order. THIS is the false-positive surface: each
    /// entry is a candidate FP until a human judges it.
    pub hits: Vec<Hit>,
    /// `Some(reason)` when the candidate could not be evaluated at all.
    /// Checked before `hits` by every honest reader: an unevaluable candidate
    /// has an empty hit list that means NOTHING WAS MEASURED, not "clean".
    pub unevaluable: Option<String>,
}

impl BacktestReport {
    /// The summary an operator reads before deciding to create the rule.
    /// Distinguishing "0 hits" from "cannot evaluate" is its whole contract.
    #[must_use]
    pub fn summary(&self) -> String {
        if let Some(reason) = &self.unevaluable {
            return format!(
                "CANNOT EVALUATE '{policy}' over tx {from}..={to}: {reason} \
                 This is not \"0 hits\" — nothing was measured, and creating \
                 the rule means deploying it against a surface nobody has seen.",
                policy = self.policy_iri,
                from = self.window.from_tx,
                to = self.window.to_tx,
            );
        }
        format!(
            "backtest of '{policy}' over tx {from}..={to}: this rule would have \
             fired {hits} time(s) across {evals} evaluation(s) in {txs} \
             transaction(s). Born advisory, each firing is a warn; every hit is \
             a false-positive CANDIDATE until a human judges it. Measures only \
             writes that happened — a quiet window bounds no false negatives.",
            policy = self.policy_iri,
            from = self.window.from_tx,
            to = self.window.to_tx,
            hits = self.hits.len(),
            evals = self.evaluations,
            txs = self.transactions,
        )
    }
}

/// Replay `candidate` over the store's own recorded history in `window`.
///
/// For each transaction: the entities it touched that carried the target type
/// AS OF that transaction are evaluated against the claim AS OF that
/// transaction — the retrospective twin of `guard::evaluate_write`. The claim
/// unsatisfied is a [`Hit`]; satisfied is counted but silent.
///
/// # Errors
/// Store/SQL errors. A claim that FAILS TO EVALUATE (malformed SPARQL, not an
/// ASK) is not an error: it comes back as an unevaluable report, because the
/// caller is a human deciding whether to create the rule, and "your draft
/// cannot be measured, here is why" is the answer they need.
pub fn backtest(store: &Store, candidate: &Candidate, window: &Window) -> Result<BacktestReport> {
    let mut report = BacktestReport {
        policy_iri: candidate.policy_iri.clone(),
        window: *window,
        transactions: 0,
        evaluations: 0,
        hits: Vec::new(),
        unevaluable: None,
    };
    let refuse = |mut report: BacktestReport, reason: String| {
        report.unevaluable = Some(reason);
        report.hits.clear(); // partial hits under a failed evaluation are noise
        Ok(report)
    };

    let Some(target_type) = candidate.target_type_iri.as_deref() else {
        return refuse(
            report,
            "the candidate declares no aegis:targets, so nothing selects which \
             entities it judges. Add the target type IRI to the draft."
                .into(),
        );
    };
    let Some(claim) = candidate.claim.as_deref() else {
        return refuse(
            report,
            "the candidate carries no SPARQL aegis:claim. A structural \
             selector/predicate policy needs the file as it stood, and the \
             store holds neither the file nor the parser (the same limit \
             replay.rs states) — backtest it where the evidence lives, or give \
             the draft a claim the store can evaluate."
                .into(),
        );
    };
    if !claim.contains("$target") {
        return refuse(
            report,
            "the candidate's claim has no $target placeholder, so it evaluates \
             identically for every entity — the backtest would report either \
             every touched target or none, which measures the claim, not the \
             rule. Write the claim about $target."
                .into(),
        );
    }
    if super::guard::guard_iri(target_type).is_err() {
        return refuse(
            report,
            "the candidate's aegis:targets is not a bare IRI, so it cannot be \
             inlined into the type probe. Remove whitespace and < > \" { } \\."
                .into(),
        );
    }

    for tx in window.from_tx..=window.to_tx {
        let Some(meta) = store.get_transaction(tx)? else {
            continue; // ids are dense in practice, but a gap is not a finding
        };
        report.transactions += 1;
        for entity_iri in touched_entities(store, tx)? {
            // Same injection guard as the gate; an IRI the gate could never
            // bind is one the policy could never fire on.
            if super::guard::guard_iri(&entity_iri).is_err() {
                continue;
            }
            let at = TemporalContext {
                as_of_tx: Some(tx),
                ..TemporalContext::default()
            };
            // Was it a target AS OF this tx? Judged temporally, like the claim:
            // an entity typed later must not backdate firings.
            let typed = format!("ASK {{ <{entity_iri}> <{RDF_TYPE}> <{target_type}> }}");
            if !ask_at(store, &typed, &at)? {
                continue;
            }
            report.evaluations += 1;
            let bound = claim.replace("$target", &format!("<{entity_iri}>"));
            match ask_at(store, &bound, &at) {
                Ok(true) => {} // compliant then; the gate would have stayed silent
                Ok(false) => report.hits.push(Hit {
                    tx,
                    timestamp: meta.timestamp.clone(),
                    target_iri: entity_iri,
                }),
                Err(e) => {
                    return refuse(
                        report,
                        format!(
                            "the claim failed to evaluate ({e}). Fix the ASK in \
                             the draft and backtest again."
                        ),
                    );
                }
            }
        }
    }
    Ok(report)
}

/// The entities transaction `tx` wrote (assert or retract) — the population
/// the gate evaluates, deduplicated, resolved to IRIs.
fn touched_entities(store: &Store, tx: i64) -> Result<Vec<String>> {
    let mut stmt = store.prepare("SELECT DISTINCT e FROM facts WHERE tx = ?1")?;
    let ids = stmt
        .query_map(rusqlite::params![tx], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<i64>, _>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(iri) = store.resolve(id) {
            out.push(iri);
        }
    }
    Ok(out)
}

/// Run an ASK at a temporal context, erroring on anything that is not an ASK.
fn ask_at(store: &Store, query: &str, at: &TemporalContext) -> Result<bool> {
    match sparql::query_temporal(store, query, at)? {
        QueryResult::Ask(b) => Ok(b),
        _ => Err(Error::InvalidValue(
            "candidate claim must be a SPARQL ASK query".into(),
        )),
    }
}

#[cfg(test)]
#[path = "backtest_tests.rs"]
mod tests;
