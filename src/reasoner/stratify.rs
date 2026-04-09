//! Stratification check for a loaded ruleset.
//!
//! **Placeholder** — the stratifier lands in the next commit. For now this
//! module exists so `parse.rs` can compile and the public API is stable.

use super::{Result, parse::RuleSet};

/// Assign strata to a ruleset, rejecting negation cycles.
///
/// **Not yet implemented** — see the next commit in the Phase 2 series.
pub fn stratify(_ruleset: &RuleSet) -> Result<Vec<Vec<usize>>> {
    Ok(Vec::new())
}
