//! Reasoner evaluation engine.
//!
//! **Placeholder** — the datafrog-backed evaluator lands in a later commit
//! in the Phase 2 series. For now this module exists so the public API is
//! stable and the CLI scaffolding can compile against it.

use super::{Result, parse::RuleSet};
use crate::store::Store;

/// Summary of a single `evaluate` call.
#[derive(Debug, Clone, Default)]
pub struct EvalReport {
    /// Number of derived facts asserted in this pass.
    pub asserted: usize,
    /// Number of derived facts retracted in this pass.
    pub retracted: usize,
    /// Strata actually executed.
    pub strata_run: usize,
    /// Per-rule derivation counts keyed by rule id.
    pub per_rule: Vec<(String, usize)>,
}

/// Run the ruleset to a fixed point and write derived facts back through
/// `Store::transact()`.
///
/// **Not yet implemented** — returns an empty report. The real evaluator
/// is the next piece of the Phase 2 rollout.
pub fn evaluate(_store: &mut Store, _ruleset: &RuleSet, _timestamp: &str) -> Result<EvalReport> {
    Ok(EvalReport::default())
}
