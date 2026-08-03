//! Replay — what a recorded window says about promoting a rule.
//!
//! The advise→enforce promotion ladder (`hank/docs/work-scoped-governance.md`
//! §Evals) asks a question no static analysis can answer: *if this rule started
//! blocking, what would it have stopped?* SARC's operating point θ has the same
//! shape — a declared false-positive tolerance is a number, and only measurement
//! makes it a calibration.
//!
//! This re-checks a recorded trace against the **current** Σ and reports, per
//! constraint, what actually happened and what would have happened at
//! `enforce`. It is deterministic arithmetic over records, not a simulation:
//! nothing is re-evaluated, because the predicate needed the file as it stood
//! and that file is gone.
//!
//! ## Five things it measures, and why each is a promotion gate
//!
//! - **Liveness.** Did the rule ever fire? A rule promoted to `enforce` without
//!   ever having fired in `advise` has been tested by nothing.
//! - **Both outcomes.** Did it record `satisfied` *and* `unsatisfied`? A check
//!   that only ever says one of the two is either vacuous or universal, and both
//!   are indistinguishable from broken until you look.
//! - **New blocks.** How many actions would `enforce` have refused that `advise`
//!   let through? The number an operator is actually deciding about.
//! - **Recoverability.** After a refusal, did work on the same target ever
//!   succeed? A rule nobody has ever got past is not a constraint, it is an
//!   outage with a reason attached.
//! - **Blast radius.** Over how many distinct targets? One rule firing forty
//!   times on one file is a different risk from forty files.
//!
//! ## What replay cannot tell you, stated before any number is read
//!
//! **It measures only traffic that happened.** A rule that would block a kind of
//! edit nobody attempted in this window shows zero new blocks and is not
//! therefore safe.
//!
//! **It counts false-positive CANDIDATES, never false positives.** A block is
//! wrong only if the action was legitimate, and no record contains that
//! judgement — it needs a human. Reporting `new_blocks` as an FP rate would be
//! the number-shaped fiction the operating point exists to replace.
//!
//! **It measures no false negatives at all.** Actions the rule let through
//! without firing are indistinguishable in the record from actions it correctly
//! approved. Nothing here bounds what was missed.

use std::collections::{BTreeMap, BTreeSet};

use super::audit::TraceRecord;
use super::audit_spec::{self, Constraint};
use crate::error::Result;
use crate::store::Store;

/// What one constraint did over the window, and what it would have done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleStats {
    /// The constraint id, as the trace cites it.
    pub id: String,
    /// Evaluations recorded.
    pub evaluated: usize,
    /// Times the predicate held.
    pub satisfied: usize,
    /// Times it did not.
    pub unsatisfied: usize,
    /// Times the runtime actually refused the action.
    pub blocked: usize,
    /// Times it fired and only advised.
    pub advisory: usize,
    /// Times `enforce` would have refused, per the CURRENT Σ.
    pub would_block: usize,
    /// Distinct targets it fired on.
    pub targets: usize,
    /// Targets that saw a later non-refused action. See [`Self::recoverable`].
    pub recovered: usize,
    /// Whether Σ still declares this constraint at all.
    pub in_spec: bool,
}

impl RuleStats {
    /// Did it ever fire? A rule promoted without having fired is untested.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.unsatisfied > 0
    }

    /// Did it record both outcomes? A one-sided check is vacuous or universal,
    /// and until you look, neither is distinguishable from broken.
    #[must_use]
    pub fn is_two_sided(&self) -> bool {
        self.satisfied > 0 && self.unsatisfied > 0
    }

    /// Actions `enforce` would refuse that this window let through.
    #[must_use]
    pub fn new_blocks(&self) -> usize {
        self.would_block.saturating_sub(self.blocked)
    }

    /// Whether every target it refused later saw work succeed.
    ///
    /// `true` when it refused nothing — there is nothing to have got past, and
    /// reporting an unfired rule as unrecoverable would bury the real ones.
    #[must_use]
    pub fn recoverable(&self) -> bool {
        self.blocked == 0 || self.recovered > 0
    }

    /// Why this rule is not ready to promote, or `None` if nothing objects.
    ///
    /// "Nothing objects" is deliberately not "safe": the window measures only
    /// traffic that happened, and a rule can pass every gate here by never
    /// having been exercised in the way that would break it.
    #[must_use]
    pub fn blocker(&self) -> Option<String> {
        if !self.in_spec {
            return Some(format!(
                "'{}' is not in Σ — it is enforcing outside the specification, \
                 so there is nothing to promote it TO.",
                self.id
            ));
        }
        if !self.is_live() {
            return Some(format!(
                "'{}' never fired in this window, so promoting it would be \
                 enabling a rule nothing has tested.",
                self.id
            ));
        }
        if !self.is_two_sided() {
            return Some(format!(
                "'{}' recorded only `unsatisfied` — no evaluation in this window \
                 ever passed it. A check that always fails is a check that is \
                 either universal or broken, and the record cannot tell which.",
                self.id
            ));
        }
        if !self.recoverable() {
            return Some(format!(
                "'{}' refused {} action(s) and no target it refused ever saw \
                 work succeed afterwards. A rule nobody has got past is an \
                 outage with a reason attached.",
                self.id, self.blocked
            ));
        }
        None
    }

    /// The operator-facing line for this rule.
    #[must_use]
    pub fn line(&self) -> String {
        let verdict = self.blocker().unwrap_or_else(|| {
            format!(
                "no gate objects — flipping to enforce would refuse {} more \
                 action(s) over this window",
                self.new_blocks()
            )
        });
        format!(
            "{id}: {evaluated} evaluation(s), {unsatisfied} fired over {targets} \
             target(s), {blocked} blocked, {would_block} would block at enforce. {verdict}",
            id = self.id,
            evaluated = self.evaluated,
            unsatisfied = self.unsatisfied,
            targets = self.targets,
            blocked = self.blocked,
            would_block = self.would_block,
        )
    }
}

/// The result of a replay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Per-constraint statistics, in id order.
    pub rules: Vec<RuleStats>,
    /// Records read.
    pub records: usize,
    /// Lines that were not readable.
    pub unreadable: usize,
}

impl ReplayReport {
    /// Rules with no gate objecting.
    #[must_use]
    pub fn promotable(&self) -> Vec<&RuleStats> {
        self.rules
            .iter()
            .filter(|r| r.blocker().is_none())
            .collect()
    }

    /// A summary that carries the honest limits with it.
    ///
    /// The caveat is part of the summary rather than a footnote, because a
    /// promotion number read without it is read as a safety claim.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "replay over {records} record(s), {unreadable} unreadable: \
             {promotable} of {total} rule(s) have no gate objecting. Measures \
             only traffic that happened; counts blocks, never false positives \
             (that needs a human); bounds no false negatives at all.",
            records = self.records,
            unreadable = self.unreadable,
            promotable = self.promotable().len(),
            total = self.rules.len(),
        )
    }
}

/// Whether the current Σ says this constraint refuses at `enforce`.
///
/// The declared class wins over the effect when both are present, matching how
/// the runtime decides: a `soft` constraint never blocks whatever its effect
/// says, because a soft constraint that blocks is a hard one with a misleading
/// name.
fn blocks_at_enforce(constraint: Option<&Constraint>) -> bool {
    let Some(constraint) = constraint else {
        return false;
    };
    if constraint.class.as_deref() == Some("soft") {
        return false;
    }
    matches!(
        constraint.effect.as_deref(),
        Some("deny" | "require-approval" | "escalate")
    )
}

/// Re-check `trace` against the Σ currently in `store`.
pub fn replay(store: &Store, trace: &[TraceRecord], unreadable: usize) -> Result<ReplayReport> {
    let spec = audit_spec::load(store)?;
    let mut stats: BTreeMap<String, RuleStats> = BTreeMap::new();
    // Targets each rule refused, and every target that later saw a clean record.
    let mut refused: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut fired_on: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut cleared_after: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for record in trace {
        // A record that refused nothing clears every target refused BEFORE it —
        // order matters, which is why this walks the trace rather than
        // aggregating first. A target cleared before its refusal proves nothing.
        if !record.refused()
            && let Some(path) = record.path.as_deref()
        {
            for (id, targets) in &refused {
                if targets.contains(path) {
                    cleared_after
                        .entry(id.clone())
                        .or_default()
                        .insert(path.into());
                }
            }
        }

        for evaluation in &record.constraints {
            let constraint = spec.get(&evaluation.id);
            let entry = stats
                .entry(evaluation.id.clone())
                .or_insert_with(|| RuleStats {
                    id: evaluation.id.clone(),
                    in_spec: constraint.is_some(),
                    ..RuleStats::default()
                });
            entry.evaluated += 1;
            match evaluation.outcome.as_deref() {
                Some("satisfied") => entry.satisfied += 1,
                Some("unsatisfied") => entry.unsatisfied += 1,
                // `unknown` is counted as neither. A constraint that could not be
                // evaluated has told you nothing, and folding it into either
                // column is how an unevaluated check reads as a passing one.
                _ => {}
            }
            if evaluation.outcome.as_deref() != Some("unsatisfied") {
                continue;
            }
            if let Some(path) = record.path.as_deref() {
                fired_on
                    .entry(evaluation.id.clone())
                    .or_default()
                    .insert(path.into());
            }
            if evaluation.response.as_deref() == Some("blocked") {
                entry.blocked += 1;
                if let Some(path) = record.path.as_deref() {
                    refused
                        .entry(evaluation.id.clone())
                        .or_default()
                        .insert(path.into());
                }
            } else {
                entry.advisory += 1;
            }
            if blocks_at_enforce(constraint) {
                entry.would_block += 1;
            }
        }
    }

    for (id, entry) in &mut stats {
        entry.targets = fired_on.get(id).map_or(0, BTreeSet::len);
        entry.recovered = cleared_after.get(id).map_or(0, BTreeSet::len);
    }
    Ok(ReplayReport {
        rules: stats.into_values().collect(),
        records: trace.len(),
        unreadable,
    })
}

/// Replay a raw JSONL trace.
pub fn replay_jsonl(store: &Store, jsonl: &str) -> Result<ReplayReport> {
    let (records, unreadable) = super::audit::parse_trace(jsonl);
    replay(store, &records, unreadable)
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
