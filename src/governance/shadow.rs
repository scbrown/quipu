//! Shadow gate — judge a CANDIDATE policy set over recorded history, never
//! writing (aegis-xfuch4.2).
//!
//! `quipu gate shadow --rules <candidate.ttl>` answers the rollout question
//! before a rule exists: over this window, which committed writes would the
//! candidate have refused that the governing set admitted, and the reverse?
//!
//! ## One evaluator
//!
//! Both sides are judged by the live gate's own code: [`guard::judge_claim_as_enforced`]
//! and [`guard::escalation`] through an as-of [`EvalCtx`]. The shadow cannot
//! drift from the gate it models, which is the defect `policy backtest` had
//! (effect filter, evidence probe, graph-scoped types).
//!
//! ## What each transaction is judged against
//!
//! - **Baseline** = the policy set as it stood when that transaction was
//!   gated. Policies are data and change; the live gate caches its registry
//!   and invalidates it AFTER a governance write commits, so transaction N is
//!   judged by the set as of N-1. A transaction that amends a policy is a
//!   **policy boundary**, reported by id, so a diff that straddles a policy
//!   change is visibly not the candidate's.
//! - **Candidate** = the baseline with the candidate layered on (same IRI
//!   replaces), or the candidate alone under [`Mode::Replace`].
//! - Both read the post-state of N (`as_of_tx = N`), the view the gate had
//!   inside its savepoint, and the router as of N at N's own timestamp.
//!
//! ## What it honestly cannot see, said before any number
//!
//! - **Refused writes.** A refusal rolls its delta back (GS2), so it is not in
//!   the log. They are COUNTED from `write.refused` events and reported as not
//!   replayable, never folded into zero. xfuch4.1 (denial quarantine) closes this.
//! - **Idempotent no-op assertions.** The gate evaluates every staged datum; the
//!   log keeps only rows that were written. An entity touched only by a no-op
//!   re-assertion is invisible here.
//! - **Whether enforcement was on.** The config is not in the store. A baseline
//!   that would refuse a committed write says the gate was off, or an approval
//!   existed, or history differs; that count is the fidelity signal, reported.
//! - Recorded verdicts join to the judged transaction by `aegis:gatedTx` when
//!   present, and otherwise by the historical convention (the verdict is written
//!   in the next transaction). The match rate is reported beside every diff.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;

use super::backtest::Window;
use super::guard::{self, ClaimOutcome, EvalCtx, PolicyRegistry};
use crate::error::{Error, Result};
use crate::namespace::RDF_TYPE;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// How the candidate combines with the governing set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Candidate policies are added; one with an existing IRI replaces it.
    Add,
    /// The candidate IS the whole set.
    Replace,
}

impl Mode {
    /// The CLI spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Replace => "replace",
        }
    }
}

/// A candidate policy set, compiled by the live registry's own compile path.
#[derive(Debug, Clone)]
pub struct Candidate {
    registry: PolicyRegistry,
}

impl Candidate {
    /// Compile candidate Turtle WITHOUT touching the judged store: it is loaded
    /// into a private in-memory store and compiled there by
    /// [`PolicyRegistry::build`], so a candidate compiles exactly as a live
    /// policy would.
    ///
    /// # Errors
    /// Unparseable Turtle, or no action-boundary `aegis:Policy` carrying
    /// `aegis:targets` and `aegis:claim` (the only kind the gate enforces).
    pub fn from_turtle(turtle: &str) -> Result<Self> {
        let mut scratch = Store::open_in_memory()?;
        crate::rdf::ingest_rdf(
            &mut scratch,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "1970-01-01T00:00:00Z",
            Some("shadow"),
            Some("candidate"),
        )?;
        let registry = PolicyRegistry::build(&scratch)?;
        if registry.sorted().is_empty() {
            return Err(Error::InvalidValue(
                "the candidate declares no action-boundary aegis:Policy with \
                 aegis:targets and aegis:claim, so the write gate would enforce \
                 nothing from it. Nothing to shadow."
                    .into(),
            ));
        }
        Ok(Self { registry })
    }

    /// The candidate's policy IRIs, sorted.
    #[must_use]
    pub fn policy_iris(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .registry
            .sorted()
            .iter()
            .map(|p| p.iri().to_string())
            .collect();
        set.into_iter().collect()
    }
}

/// What to replay.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// The transaction window, inclusive.
    pub window: Window,
    /// Stop after judging this many transactions (reported as truncation).
    pub max_txs: Option<usize>,
    /// How the candidate combines with the governing set.
    pub mode: Mode,
}

/// The window covering transactions stamped within the last `secs` seconds.
/// Timestamps that are not RFC 3339 (legacy placeholder stamps) are ignored
/// for choosing the start, never for what is judged inside the window.
///
/// # Errors
/// Store errors reading the transaction log.
pub fn window_since(store: &Store, secs: u64) -> Result<Window> {
    let cutoff = crate::time::iso_secs_ago(secs);
    let to_tx = store.latest_tx_id()?;
    let from: Option<i64> = store
        .prepare(
            "SELECT MIN(id) FROM transactions WHERE timestamp >= ?1 \
             AND timestamp GLOB '[0-9][0-9][0-9][0-9]-*'",
        )?
        .query_row(params![cutoff], |r| r.get(0))?;
    Ok(Window {
        // Nothing inside the window: an empty range, never "everything".
        from_tx: from.unwrap_or(to_tx + 1),
        to_tx,
    })
}

/// Parse `90m`, `24h`, `7d` or bare seconds.
///
/// # Errors
/// [`Error::InvalidValue`] on anything else.
pub fn parse_duration(s: &str) -> Result<u64> {
    let bad = || Error::InvalidValue(format!("--since {s}: expected e.g. 90m, 24h, 7d"));
    let (num, mult) = match s.chars().last() {
        Some('s') => (&s[..s.len() - 1], 1),
        Some('m') => (&s[..s.len() - 1], 60),
        Some('h') => (&s[..s.len() - 1], 3_600),
        Some('d') => (&s[..s.len() - 1], 86_400),
        Some(c) if c.is_ascii_digit() => (s, 1),
        _ => return Err(bad()),
    };
    num.parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
        .ok_or_else(bad)
}

/// Per (rule, writer) counts on one side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleStats {
    /// Claim evaluations run.
    pub evaluations: usize,
    /// Claim held.
    pub satisfied: usize,
    /// Claim failed (whatever the effect then did).
    pub unsatisfied: usize,
    /// Evidence probe found nothing to judge.
    pub unknown: usize,
    /// `deny` (or default) with the claim failed: refused outright.
    pub would_refuse: usize,
    /// Escalating effect with the claim failed and no approval: refused
    /// pending a human (a request would have been opened or was pending).
    pub would_escalate: usize,
    /// Escalating effect with the claim failed, admitted by a standing approval.
    pub approved: usize,
    /// Advisory effect with the claim failed: fires, never blocks.
    pub would_warn: usize,
    /// The claim or router errored. Counted, never folded into zero.
    pub unevaluable: usize,
}

/// Which way a transaction's outcome moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    /// The governing set admitted it; the candidate would refuse it. THE
    /// rollout risk.
    NewRefusal,
    /// The governing set would refuse it; the candidate admits it.
    WouldNowAdmit,
}

impl DiffKind {
    /// Stable label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewRefusal => "new-refusal",
            Self::WouldNowAdmit => "would-now-admit",
        }
    }
}

/// One transaction whose outcome differs between the two sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// The transaction.
    pub tx: i64,
    /// Its writer label.
    pub writer: String,
    /// Direction.
    pub kind: DiffKind,
    /// The rules that refused on the refusing side, with their targets.
    pub refused_by: Vec<(String, String)>,
}

/// How recorded verdicts lined up with the baseline's judgements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerdictJoin {
    /// Baseline judgements of enforced policies (the ones the gate records).
    pub judged: usize,
    /// A recorded verdict with the same policy, target and outcome.
    pub matched: usize,
    /// A recorded verdict for the same policy and target, different outcome.
    pub outcome_mismatch: usize,
    /// No recorded verdict found for the pair.
    pub unmatched: usize,
    /// Of the joined ones, how many joined by `aegis:gatedTx`.
    pub via_gated_tx: usize,
}

/// The shadow run's findings.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// First and last transaction considered.
    pub from_tx: i64,
    /// Inclusive.
    pub to_tx: i64,
    /// Transactions present in the log within the window.
    pub transactions: usize,
    /// Transactions judged.
    pub judged: usize,
    /// Gate-bypassing bookkeeping writes (verdict and request recording).
    pub bypass_skipped: usize,
    /// Transactions whose timestamp did not parse, judged with the router at 0.
    pub unparsed_time: usize,
    /// Set when `max_txs` stopped the run: the first transaction NOT judged.
    pub truncated_at: Option<i64>,
    /// Transactions that amended the governing policy set.
    pub policy_boundaries: Vec<i64>,
    /// Candidate side, by (rule, writer).
    pub candidate: BTreeMap<(String, String), RuleStats>,
    /// Baseline side, by (rule, writer).
    pub baseline: BTreeMap<(String, String), RuleStats>,
    /// Committed transactions the baseline would itself refuse (fidelity).
    pub baseline_refuses_committed: usize,
    /// Outcome changes, in tx order.
    pub diffs: Vec<Diff>,
    /// The first unevaluable judgements, for diagnosis.
    pub unevaluable_samples: Vec<String>,
    /// `write.refused` events in the window. Not replayable (see module doc).
    pub refusals_not_replayable: usize,
    /// Recorded-verdict join.
    pub verdict_join: VerdictJoin,
}

const SAMPLE_CAP: usize = 20;

/// Replay `candidate` and the governing set over `opts.window`.
///
/// # Errors
/// Store/SQL errors. A claim that fails to evaluate is COUNTED as unevaluable
/// on its rule, never an error and never a pass.
pub fn run(store: &Store, candidate: &Candidate, opts: &Options) -> Result<Report> {
    let mut report = Report {
        from_tx: opts.window.from_tx,
        to_tx: opts.window.to_tx,
        refusals_not_replayable: count_refusals(store, &opts.window)?,
        ..Report::default()
    };
    let Some(rdf_type_id) = store.lookup(RDF_TYPE)? else {
        return Ok(report);
    };
    let gated_index = join::gated_tx_index(store)?;

    let mut baseline = PolicyRegistry::build_at(store, &as_of(opts.window.from_tx - 1))?;
    let mut combined = combine(&baseline, candidate, opts.mode);

    for tx in opts.window.from_tx..=opts.window.to_tx {
        let Some(meta) = store.get_transaction(tx)? else {
            continue;
        };
        report.transactions += 1;
        if is_bypass(&meta) {
            report.bypass_skipped += 1;
            continue;
        }
        if opts.max_txs.is_some_and(|m| report.judged >= m) {
            report.truncated_at = Some(tx);
            break;
        }
        let now = crate::time::epoch_of_rfc3339(&meta.timestamp).unwrap_or_else(|| {
            report.unparsed_time += 1;
            0
        });
        let writer = writer_label(store, &meta)?;
        let ctx = EvalCtx::as_of(store, tx, now);
        let touched = touched_in_tx(store, tx)?;

        let mut side = Side {
            ctx: &ctx,
            rdf_type_id,
            writer: &writer,
            samples: &mut report.unevaluable_samples,
        };
        let base = side.judge(&baseline, &touched, &mut report.baseline)?;
        let cand = side.judge(&combined, &touched, &mut report.candidate)?;
        report.judged += 1;
        if !base.refused_by.is_empty() {
            report.baseline_refuses_committed += 1;
        }
        join::join_verdicts(
            store,
            tx,
            &base.enforced,
            &gated_index,
            &mut report.verdict_join,
        )?;

        match (base.refused_by.is_empty(), cand.refused_by.is_empty()) {
            (true, false) => report.diffs.push(Diff {
                tx,
                writer: writer.clone(),
                kind: DiffKind::NewRefusal,
                refused_by: cand.refused_by,
            }),
            (false, true) => report.diffs.push(Diff {
                tx,
                writer: writer.clone(),
                kind: DiffKind::WouldNowAdmit,
                refused_by: base.refused_by,
            }),
            _ => {}
        }

        // The live gate invalidates its cached registry AFTER a governance
        // write commits, so the NEXT transaction sees the amended set.
        if tx_is_governance_write(store, tx)? {
            let next = PolicyRegistry::build_at(store, &as_of(tx))?;
            if next.sorted() != baseline.sorted() {
                report.policy_boundaries.push(tx);
            }
            baseline = next;
            combined = combine(&baseline, candidate, opts.mode);
        }
    }
    Ok(report)
}

fn as_of(tx: i64) -> crate::sparql::TemporalContext {
    crate::sparql::TemporalContext {
        as_of_tx: Some(tx.max(0)),
        ..crate::sparql::TemporalContext::default()
    }
}

fn combine(baseline: &PolicyRegistry, candidate: &Candidate, mode: Mode) -> PolicyRegistry {
    match mode {
        Mode::Add => baseline.overlaid(&candidate.registry),
        Mode::Replace => candidate.registry.clone(),
    }
}

/// Bookkeeping writes the gate never evaluates (`recording_verdicts`).
fn is_bypass(meta: &crate::types::Transaction) -> bool {
    meta.actor.as_deref() == Some("quipu")
        && matches!(
            meta.source.as_deref(),
            Some("write-gate verdict" | "escalation request")
        )
}

/// The writer: the authenticated principal when the store recorded one,
/// otherwise the declared actor, otherwise `(none)`.
fn writer_label(store: &Store, meta: &crate::types::Transaction) -> Result<String> {
    if let Some(id) = store.transaction_auth(meta.id)? {
        return Ok(format!("principal:{}", id.principal));
    }
    Ok(meta.actor.clone().unwrap_or_else(|| "(none)".into()))
}

/// The (entity, graph) pairs transaction `tx` wrote, asserts and retracts.
fn touched_in_tx(store: &Store, tx: i64) -> Result<Vec<(i64, i64)>> {
    let mut stmt = store.prepare("SELECT DISTINCT e, g FROM facts WHERE tx = ?1 ORDER BY e, g")?;
    let rows = stmt
        .query_map(params![tx], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn tx_is_governance_write(store: &Store, tx: i64) -> Result<bool> {
    let mut stmt = store.prepare("SELECT e, a, v, op FROM facts WHERE tx = ?1")?;
    let mut datums = Vec::new();
    let mut rows = stmt.query(params![tx])?;
    while let Some(r) = rows.next()? {
        datums.push(Datum {
            entity: r.get(0)?,
            attribute: r.get(1)?,
            value: Value::from_bytes(&r.get::<_, Vec<u8>>(2)?)?,
            valid_from: String::new(),
            valid_to: None,
            op: if r.get::<_, i64>(3)? == 0 {
                Op::Retract
            } else {
                Op::Assert
            },
        });
    }
    guard::is_governance_write(store, &datums)
}

fn count_refusals(store: &Store, window: &Window) -> Result<usize> {
    let has_events: bool = store
        .prepare("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='events')")?
        .query_row([], |r| r.get(0))?;
    if !has_events {
        return Ok(0);
    }
    // A refusal is stamped with the last COMMITTED tx at refusal time, so a
    // refusal between N-1 and N carries N-1.
    let n: i64 = store
        .prepare("SELECT COUNT(*) FROM events WHERE type = ?1 AND tx_id >= ?2 AND tx_id <= ?3")?
        .query_row(
            params![
                crate::store::events::REFUSAL_EVENT,
                window.from_tx - 1,
                window.to_tx
            ],
            |r| r.get(0),
        )?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// One side's decision for one transaction.
struct Decision {
    /// (rule, target) pairs that refused.
    refused_by: Vec<(String, String)>,
    /// (rule, target, outcome) for enforced policies: what the gate records.
    enforced: Vec<(String, String, &'static str)>,
}

struct Side<'c, 'a> {
    ctx: &'c EvalCtx<'a>,
    rdf_type_id: i64,
    writer: &'c str,
    samples: &'c mut Vec<String>,
}

impl Side<'_, '_> {
    fn judge(
        &mut self,
        registry: &PolicyRegistry,
        touched: &[(i64, i64)],
        stats: &mut BTreeMap<(String, String), RuleStats>,
    ) -> Result<Decision> {
        let mut decision = Decision {
            refused_by: Vec::new(),
            enforced: Vec::new(),
        };
        for &(entity, graph) in touched {
            let types = guard::entity_type_iris(self.ctx, entity, self.rdf_type_id, graph)?;
            let mut entity_iri: Option<String> = None;
            for t in &types {
                for policy in registry.policies_for(t) {
                    let iri = match &entity_iri {
                        Some(s) => s.clone(),
                        None => {
                            let s = self.ctx.store.resolve(entity)?;
                            entity_iri = Some(s.clone());
                            s
                        }
                    };
                    let row = stats
                        .entry((policy.iri().to_string(), self.writer.to_string()))
                        .or_default();
                    row.evaluations += 1;
                    let outcome = match guard::judge_claim_as_enforced(self.ctx, &iri, policy) {
                        Ok(o) => o,
                        Err(e) => {
                            row.unevaluable += 1;
                            self.sample(policy.iri(), &iri, &e);
                            continue;
                        }
                    };
                    if policy.blocks()
                        && let Some(v) = outcome.verdict()
                    {
                        decision
                            .enforced
                            .push((policy.iri().to_string(), iri.clone(), v));
                    }
                    match outcome {
                        ClaimOutcome::NotEnforced => {}
                        ClaimOutcome::Unknown => row.unknown += 1,
                        ClaimOutcome::Satisfied => row.satisfied += 1,
                        ClaimOutcome::Unsatisfied => {
                            row.unsatisfied += 1;
                            if !policy.blocks() {
                                row.would_warn += 1;
                                continue;
                            }
                            match guard::escalation(self.ctx, &iri, policy) {
                                Ok(esc) if esc.admits() => row.approved += 1,
                                Ok(guard::Escalation::Refused) => {
                                    row.would_refuse += 1;
                                    decision
                                        .refused_by
                                        .push((policy.iri().to_string(), iri.clone()));
                                }
                                Ok(_) => {
                                    row.would_escalate += 1;
                                    decision
                                        .refused_by
                                        .push((policy.iri().to_string(), iri.clone()));
                                }
                                Err(e) => {
                                    row.unevaluable += 1;
                                    self.sample(policy.iri(), &iri, &e);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(decision)
    }

    fn sample(&mut self, policy: &str, target: &str, e: &Error) {
        if self.samples.len() < SAMPLE_CAP {
            self.samples.push(format!(
                "tx {}: {policy} on {target}: {e}",
                self.ctx.at.as_of_tx.unwrap_or(0)
            ));
        }
    }
}

#[path = "shadow_join.rs"]
mod join;

#[cfg(test)]
#[path = "shadow_tests.rs"]
mod tests;
