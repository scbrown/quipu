//! `quipu_audit_check(Σ, T)` — deciding whether a trace conforms to the spec.
//!
//! SARC (arXiv:2605.07728) Definition 2, §3.6: given a specification Σ and a
//! trace T, an auditor can mechanically decide `T ⊨ Σ` in `O(|T|·|C|)` **without
//! access to the model, its prompts, or its developers**. That property is the
//! whole reason for everything upstream of it — the constraint metadata of
//! Phase 1, the Σ-derived trace record of Phase 2, the attribution tuple of
//! Phase 6 — and until something computes it, all of that is a schema nobody
//! checks.
//!
//! ## Four passes
//!
//! 1. **Coverage** — is every constraint the trace cites actually in Σ, and does
//!    every refusal cite a constraint?
//! 2. **Class-placement compatibility** — was each constraint evaluated at a
//!    point its class can be enforced at (SARC Table 3), and does the trace's
//!    recorded class and point agree with what Σ declares?
//! 3. **Outcome consistency** — does the response the runtime took match the one
//!    the constraint declares for that outcome, at that mode?
//! 4. **Attribution completeness** — does each record say who is answerable?
//!
//! ## Deterministic, and never an LLM call
//!
//! [SARC] §5.1's design rule, and the same `O(ℓ_tool)` budget discipline hank
//! applies to its own guard. Every pass here is a comparison between two
//! declared values. A checker that asked a model whether a trace looked
//! compliant would be unable to state what it checked, which is the one thing an
//! audit has to be able to do.
//!
//! ## Two severities, because they are two different claims
//!
//! A **violation** is the trace contradicting Σ: a soft constraint that blocked,
//! a hard `deny` that only warned under `enforce`, a record whose declared chain
//! disagrees with the process that ran. An **incompleteness** is the trace not
//! saying enough to decide: no principal chain, no declared class, a constraint
//! in Σ that this window never exercised.
//!
//! Collapsing them would make the checker useless in the direction that matters.
//! Report everything as a violation and an operator learns to ignore the output;
//! report everything as an incompleteness and a soft constraint blocking an edit
//! reads as a formatting note. So [`Report::conforms`] answers the first
//! question and [`Report::is_complete`] the second, and neither is allowed to
//! stand in for the other.
//!
//! ## What this pass honestly cannot check
//!
//! **Per-action coverage.** SARC's coverage pass asks whether every constraint
//! that *applied* to an action was evaluated. Deciding applicability means
//! re-running the selector — a tree-sitter query against the file as it stood —
//! and quipu has neither the file nor the parser. What is checkable here is the
//! converse direction (nothing is cited that Σ does not define) plus vacuity
//! (constraints Σ declares that the window never exercised). The gap is named
//! rather than papered over: a checker that reported "coverage: pass" while
//! testing something weaker would be the more dangerous artifact.

use crate::error::Result;
use crate::store::Store;

mod parse;
mod passes;

pub use parse::{Evaluation, TraceRecord, parse_trace};

/// Which pass produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pass {
    /// Constraints cited versus constraints declared.
    Coverage,
    /// Class ↔ enforcement point, and trace versus Σ.
    Placement,
    /// Response taken versus response declared.
    Outcome,
    /// Who is answerable.
    Attribution,
    /// Whether every executable tool class traverses an enforcement point.
    /// Runs over the dispatch graph rather than over a trace, so its findings
    /// carry no record index — see [`super::inventory`].
    Inventory,
}

impl Pass {
    /// The pass name, for a report line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Pass::Coverage => "coverage",
            Pass::Placement => "placement",
            Pass::Outcome => "outcome",
            Pass::Attribution => "attribution",
            Pass::Inventory => "inventory",
        }
    }
}

/// Whether a finding is the trace *contradicting* Σ or merely *underdetermining*
/// it. See the module doc — these are two different claims and the checker is
/// not allowed to let one stand in for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The trace contradicts Σ.
    Violation,
    /// The trace does not say enough to decide.
    Incompleteness,
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discrepancy {
    /// Which pass found it.
    pub pass: Pass,
    /// How much it means.
    pub severity: Severity,
    /// Index of the trace record, or `None` for a whole-window finding.
    pub record: Option<usize>,
    /// The constraint it is about, when it is about one.
    pub constraint: Option<String>,
    /// What is wrong, and what would resolve it.
    pub detail: String,
}

/// The result of a check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Every finding, in pass order.
    pub discrepancies: Vec<Discrepancy>,
    /// Trace records read.
    pub records_checked: usize,
    /// Lines that were not readable as records. Counted rather than dropped: a
    /// checker that silently skipped them would report conformance over a
    /// window it had only partly read.
    pub records_unreadable: usize,
    /// Constraints in Σ at the time of the check.
    pub constraints_in_scope: usize,
}

impl Report {
    /// Whether the trace contradicts Σ anywhere.
    ///
    /// Deliberately independent of [`Self::is_complete`]: a window with no
    /// contradictions and no attribution still conforms — it just does not say
    /// much. Folding the two would make an unattributed deployment permanently
    /// non-conformant, and an operator would stop reading the answer.
    #[must_use]
    pub fn conforms(&self) -> bool {
        !self
            .discrepancies
            .iter()
            .any(|d| d.severity == Severity::Violation)
    }

    /// Whether the trace says enough for the check to have been meaningful.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self
            .discrepancies
            .iter()
            .any(|d| d.severity == Severity::Incompleteness)
    }

    /// Findings of one severity.
    #[must_use]
    pub fn of(&self, severity: Severity) -> Vec<&Discrepancy> {
        self.discrepancies
            .iter()
            .filter(|d| d.severity == severity)
            .collect()
    }

    /// A one-line summary an operator can read without expanding anything.
    ///
    /// Always names the unreadable count, even at zero. Coverage of the input is
    /// the one number a reader must not have to infer from an omission.
    #[must_use]
    pub fn summary(&self) -> String {
        let violations = self.of(Severity::Violation).len();
        let incomplete = self.of(Severity::Incompleteness).len();
        format!(
            "{verdict}: {violations} violation(s), {incomplete} incompleteness(es) \
             over {records} record(s) against {constraints} constraint(s); \
             {unreadable} line(s) unreadable",
            verdict = if self.conforms() {
                "T ⊨ Σ"
            } else {
                "T ⊭ Σ"
            },
            records = self.records_checked,
            constraints = self.constraints_in_scope,
            unreadable = self.records_unreadable,
        )
    }
}

/// Check a trace against the Σ currently in `store`.
///
/// `unreadable` is the count of input lines that were not parseable as records,
/// carried through from [`parse_trace`] so the report can state its own coverage
/// of the input.
pub fn check(store: &Store, trace: &[TraceRecord], unreadable: usize) -> Result<Report> {
    let spec = super::audit_spec::load(store)?;
    let mut report = Report {
        records_checked: trace.len(),
        records_unreadable: unreadable,
        constraints_in_scope: spec.len(),
        ..Report::default()
    };
    passes::coverage(&spec, trace, &mut report);
    passes::placement(&spec, trace, &mut report);
    passes::outcome(&spec, trace, &mut report);
    passes::attribution(trace, &mut report);
    Ok(report)
}

/// Check a raw JSONL trace against `store`.
#[must_use = "a check whose report is dropped has audited nothing"]
pub fn check_jsonl(store: &Store, jsonl: &str) -> Result<Report> {
    let (records, unreadable) = parse_trace(jsonl);
    check(store, &records, unreadable)
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
