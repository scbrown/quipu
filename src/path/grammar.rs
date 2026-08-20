//! Conformance grammar v1 — the versioned step-matching contract.
//!
//! One definition of "trajectory T conforms to path P", shared by the
//! backtest here and yupana's guard, per
//! `docs/design/conformance-grammar.md`. Any change to matching semantics is
//! a new major version: a verdict under new rules is not comparable to a
//! backtest under old ones.

use serde::Serialize;

/// The grammar version everything in this module implements.
pub const GRAMMAR_VERSION: &str = "gp-grammar/1";

/// A step's v1 signature: `(actionKind, targetClass)`, compared exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepSig {
    /// The step's `actionKind` string, case-sensitive.
    pub action_kind: String,
    /// The class of the step's target: an IRI target's deterministically
    /// chosen `rdf:type` (lexicographically smallest when several),
    /// `"untyped"` for an IRI target with no recorded type, `"literal"` for
    /// a literal target, `"none"` when the step has no target.
    pub target_class: String,
}

/// Where a match attempt landed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum MatchOutcome {
    /// Every pattern element was matched, in order.
    Conforms,
    /// Matching stopped: `pattern_index` is the first pattern element that
    /// could not be matched by any remaining evaluable step.
    DeviatesAt {
        pattern_index: usize,
        matched: usize,
    },
}

/// Match `pattern` as a subsequence of `steps` (in order, gaps allowed).
///
/// `steps` entries are `None` for unevaluable steps (no `actionKind`
/// recorded): they never match and never deviate — missing data is not
/// misconduct — and the caller reports them in an `unevaluated` count.
#[must_use]
pub fn match_pattern(pattern: &[StepSig], steps: &[Option<StepSig>]) -> MatchOutcome {
    let mut next = 0usize;
    for step in steps {
        if next == pattern.len() {
            break;
        }
        if let Some(sig) = step
            && *sig == pattern[next]
        {
            next += 1;
        }
    }
    if next == pattern.len() {
        MatchOutcome::Conforms
    } else {
        MatchOutcome::DeviatesAt {
            pattern_index: next,
            matched: next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(kind: &str, class: &str) -> StepSig {
        StepSig {
            action_kind: kind.into(),
            target_class: class.into(),
        }
    }

    #[test]
    fn exact_sequence_conforms() {
        let p = vec![sig("edit", "literal"), sig("run", "literal")];
        let s: Vec<_> = p.iter().cloned().map(Some).collect();
        assert_eq!(match_pattern(&p, &s), MatchOutcome::Conforms);
    }

    #[test]
    fn gaps_are_allowed() {
        let p = vec![sig("edit", "literal"), sig("verify", "literal")];
        let s = vec![
            Some(sig("read", "literal")),
            Some(sig("edit", "literal")),
            Some(sig("run", "literal")),
            Some(sig("verify", "literal")),
        ];
        assert_eq!(match_pattern(&p, &s), MatchOutcome::Conforms);
    }

    #[test]
    fn order_is_not_negotiable() {
        let p = vec![sig("run", "literal"), sig("edit", "literal")];
        let s = vec![Some(sig("edit", "literal")), Some(sig("run", "literal"))];
        assert_eq!(
            match_pattern(&p, &s),
            MatchOutcome::DeviatesAt {
                pattern_index: 1,
                matched: 1
            }
        );
    }

    #[test]
    fn unevaluable_steps_neither_match_nor_deviate() {
        let p = vec![sig("edit", "literal")];
        let s = vec![None, Some(sig("edit", "literal"))];
        assert_eq!(match_pattern(&p, &s), MatchOutcome::Conforms);
    }

    #[test]
    fn empty_trajectory_deviates_at_the_first_element() {
        let p = vec![sig("edit", "literal")];
        assert_eq!(
            match_pattern(&p, &[]),
            MatchOutcome::DeviatesAt {
                pattern_index: 0,
                matched: 0
            }
        );
    }
}
