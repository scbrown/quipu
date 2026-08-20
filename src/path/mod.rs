//! Golden paths — trajectory pruning, backtesting, and drafting.
//!
//! The quipu-owned mechanism half of the golden-paths design
//! (`docs/design/golden-paths-blessing.md`): a completed work item with
//! verified results leaves a Trajectory of Steps; the provenance cone says
//! which steps the verified result depended on; the backtest replays a pruned
//! candidate over recorded history; the draft emits the `GoldenPath` as Turtle
//! for human review. The vocabulary is camayoc's (its `ontology/core.ttl`),
//! declared in the store's base namespace.
//!
//! Nothing here writes to the store: `cone` and `backtest` are reads, and
//! `draft` emits Turtle exactly as the governance drafting scaffold does —
//! born for review, never self-asserted into the graph.

pub mod backtest;
pub mod cone;
pub mod draft;
pub mod grammar;
mod read;
#[cfg(test)]
mod testutil;

pub use backtest::{BacktestReport, BacktestRow, RowResult, backtest};
pub use cone::{ConeOptions, ConeReport, ConeStep, ConeVerdict, cone};
pub use draft::{DraftOptions, draft};
pub use grammar::{GRAMMAR_VERSION, MatchOutcome, StepSig, match_pattern};

/// The golden-path vocabulary, resolved against a base namespace.
///
/// The namespace is a parameter, never a hardcoded hostname — the same rule
/// camayoc's ontology states for itself. Every function in this module takes
/// the vocabulary rather than assuming it.
#[derive(Debug, Clone)]
pub struct PathVocab {
    pub step_of: String,
    pub step_order: String,
    pub action_kind: String,
    pub action_target: String,
    pub verified_by: String,
    pub falsifier: String,
    pub trajectory_of: String,
    pub about: String,
    pub outcome: String,
    pub pruned_from: String,
    pub omits_step: String,
    pub omitted_step: String,
    pub omission_authority: String,
    pub omission_ruling: String,
    pub dead_end: String,
    pub source_kind: String,
}

impl PathVocab {
    /// Resolve the vocabulary against `base_ns` (e.g. the store's
    /// `[quipu] base_ns`).
    #[must_use]
    pub fn new(base_ns: &str) -> Self {
        let t = |local: &str| format!("{base_ns}{local}");
        Self {
            step_of: t("stepOf"),
            step_order: t("stepOrder"),
            action_kind: t("actionKind"),
            action_target: t("actionTarget"),
            verified_by: t("verifiedBy"),
            falsifier: t("falsifier"),
            trajectory_of: t("trajectoryOf"),
            about: t("about"),
            outcome: t("outcome"),
            pruned_from: t("prunedFrom"),
            omits_step: t("omitsStep"),
            omitted_step: t("omittedStep"),
            omission_authority: t("omissionAuthority"),
            omission_ruling: t("omissionRuling"),
            dead_end: t("deadEnd"),
            source_kind: t("sourceKind"),
        }
    }
}
