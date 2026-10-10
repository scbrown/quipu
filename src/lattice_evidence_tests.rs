//! Evidence behind "the label homomorphism is machine-checked" (aegis-xfuch4.4).
//!
//! Runs every law in `lattice_props.rs` twice, with a deterministic seed:
//!
//! * on the real `Freshness` axis, where all seven must hold;
//! * on [`Widening`], a deliberately WRONG axis whose meet is `max`, the
//!   widening sign error the design warns about (quipu #66).
//!
//! The expected outcome is asserted, so it cannot rot: the five algebraic laws
//! still HOLD under `max` (they pin structure, not direction), and the two
//! direction laws must FAIL. If a refactor ever made the direction laws vacuous,
//! this test fails, because they would stop catching the sign error.
//!
//! With `QUIPU_EVIDENCE_DIR` set it writes `lattice-proptest.json` and a
//! markdown table there; `QUIPU_EVIDENCE_CASES` sets cases per law (default 256,
//! proptest's own default).

use std::cell::Cell;
use std::fmt::Debug;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestError, TestRng, TestRunner};
use serde_json::{Value, json};

use super::props::{self, Axis, arb_graphset, arb_world};
use super::{Freshness, Meet};
use crate::error::Result;

/// The sabotaged axis: freshness composed by `max`. Same order, wrong direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Widening(Freshness);

impl Meet for Widening {
    fn meet(&self, other: &Self) -> Result<Self> {
        Ok(*self.max(other))
    }
}

const FRESHNESS: &[Freshness] = &[Freshness::Stale, Freshness::Recomputing, Freshness::Fresh];
const WIDENING: &[Widening] = &[
    Widening(Freshness::Stale),
    Widening(Freshness::Recomputing),
    Widening(Freshness::Fresh),
];

/// The generators, as a reader would check them against the code.
const STRATEGY: &str = "world: each of 8 graph ids independently uniform over \
    {undeclared, stale, recomputing, fresh}; dataset: a uniform-size subset of the 8 ids \
    (size 0..8); g: uniform 0..8. No input filtering (no prop_assume).";

/// One law's run on one axis.
struct Run {
    law: &'static str,
    kind: &'static str,
    cases: u32,
    /// Generated cases evaluated before the first failure (all of them on a pass).
    evaluated: u64,
    /// Of those, the cases where the law's assertion actually ran (a guarded
    /// direction law passes vacuously when a side is undeclared).
    exercised: u64,
    /// `None` on a pass.
    failure: Option<Failure>,
}

struct Failure {
    /// 1-based index of the first failing generated case.
    first_at: u64,
    /// Law evaluations spent shrinking after the first failure.
    shrink_calls: u64,
    minimal: String,
    reason: String,
}

fn run<S>(
    law: &'static str,
    kind: &'static str,
    cases: u32,
    strategy: S,
    check: impl Fn(&S::Value) -> std::result::Result<bool, TestCaseError>,
) -> Run
where
    S: Strategy,
    S::Value: Debug,
{
    let config = Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha));
    let calls = Cell::new(0_u64);
    let exercised = Cell::new(0_u64);
    let first_fail: Cell<Option<u64>> = Cell::new(None);
    let result = runner.run(&strategy, |value| {
        calls.set(calls.get() + 1);
        match check(&value) {
            Ok(did) => {
                if did && first_fail.get().is_none() {
                    exercised.set(exercised.get() + 1);
                }
                Ok(())
            }
            Err(e) => {
                if first_fail.get().is_none() {
                    first_fail.set(Some(calls.get()));
                }
                Err(e)
            }
        }
    });
    let failure = match result {
        Ok(()) => None,
        Err(TestError::Fail(reason, minimal)) => {
            let first_at = first_fail.get().unwrap_or(0);
            Some(Failure {
                first_at,
                shrink_calls: calls.get().saturating_sub(first_at),
                minimal: format!("{minimal:?}"),
                reason: reason.to_string(),
            })
        }
        Err(TestError::Abort(reason)) => panic!("{law}: proptest aborted: {reason}"),
    };
    Run {
        law,
        kind,
        cases,
        evaluated: first_fail.get().unwrap_or_else(|| calls.get()),
        exercised: exercised.get(),
        failure,
    }
}

fn run_axis<T: Axis>(values: &[T], cases: u32) -> Vec<Run> {
    let two = || (arb_world(values), arb_graphset(), arb_graphset());
    let one = || (arb_world(values), arb_graphset());
    vec![
        run("homomorphism", "algebraic", cases, two(), |(w, a, b)| {
            props::homomorphism(w, a, b)
        }),
        run("idempotent", "algebraic", cases, one(), |(w, a)| {
            props::idempotent(w, a)
        }),
        run("commutative", "algebraic", cases, two(), |(w, a, b)| {
            props::commutative(w, a, b)
        }),
        run(
            "associative",
            "algebraic",
            cases,
            (
                arb_world(values),
                arb_graphset(),
                arb_graphset(),
                arb_graphset(),
            ),
            |(w, a, b, c)| props::associative(w, a, b, c),
        ),
        run(
            "empty_dataset_is_the_identity",
            "algebraic",
            cases,
            one(),
            |(w, a)| props::empty_is_identity(w, a),
        ),
        run(
            "composition_never_widens",
            "direction",
            cases,
            two(),
            |(w, a, b)| props::composition_never_widens(w, a, b),
        ),
        run(
            "adding_a_graph_never_widens",
            "direction",
            cases,
            (arb_world(values), arb_graphset(), 0u8..props::UNIVERSE),
            |(w, a, g)| props::adding_a_graph_never_widens(w, a, *g),
        ),
    ]
}

fn to_json(r: &Run) -> Value {
    json!({
        "law": r.law,
        "kind": r.kind,
        "cases_requested": r.cases,
        "cases_evaluated": r.evaluated,
        "assertion_exercised": r.exercised,
        "outcome": if r.failure.is_some() { "fail" } else { "pass" },
        "failure": r.failure.as_ref().map(|f| json!({
            "first_failing_case": f.first_at,
            "shrink_calls": f.shrink_calls,
            "minimal_counterexample": f.minimal,
            "reason": f.reason,
        })),
    })
}

fn markdown(real: &[Run], sabotaged: &[Run], cases: u32) -> String {
    let mut out = format!(
        "## Label lattice: property-test evidence\n\n\
         {cases} cases per law, deterministic seed (ChaCha). Strategy: {STRATEGY}\n\n\
         | law | kind | real meet: cases / exercised | widening meet (max) | shrink calls | minimal counterexample |\n\
         |---|---|---|---|---|---|\n"
    );
    for (r, s) in real.iter().zip(sabotaged) {
        let real_cell = match &r.failure {
            None => format!("pass {} / {}", r.evaluated, r.exercised),
            Some(_) => "FAIL".to_string(),
        };
        let (sab, shrink, minimal) = match &s.failure {
            None => (
                "passes (blind to direction)".to_string(),
                String::new(),
                String::new(),
            ),
            Some(f) => (
                format!("fails at case {}", f.first_at),
                f.shrink_calls.to_string(),
                format!("`{}`", f.minimal),
            ),
        };
        out.push_str(&format!(
            "| {} | {} | {real_cell} | {sab} | {shrink} | {minimal} |\n",
            r.law, r.kind
        ));
    }
    out
}

#[test]
fn the_direction_laws_catch_a_widening_meet_and_the_algebraic_ones_cannot() {
    let cases: u32 = std::env::var("QUIPU_EVIDENCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    let real = run_axis(FRESHNESS, cases);
    let sabotaged = run_axis(WIDENING, cases);

    for r in &real {
        assert!(r.failure.is_none(), "{} fails on the REAL meet", r.law);
        assert_eq!(
            r.evaluated,
            u64::from(cases),
            "{}: every case evaluated",
            r.law
        );
        assert!(r.exercised > 0, "{}: never exercised its assertion", r.law);
    }
    for s in &sabotaged {
        match s.kind {
            // Pinned as a FACT about the suite, not a goal: if one of these
            // starts failing under max, the evidence text above is stale.
            "algebraic" => assert!(
                s.failure.is_none(),
                "{} now catches the widening meet; update the evidence text",
                s.law
            ),
            _ => assert!(
                s.failure.is_some(),
                "{} no longer catches the widening meet: the suite is blind to the sign error",
                s.law
            ),
        }
    }

    if let Ok(dir) = std::env::var("QUIPU_EVIDENCE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let doc = json!({
            "subject": "label lattice homomorphism and composition laws (src/lattice_props.rs)",
            "cases_per_law": cases,
            "seed": "proptest TestRng::deterministic_rng(ChaCha)",
            "strategy": STRATEGY,
            "real_meet": real.iter().map(to_json).collect::<Vec<_>>(),
            "widening_meet_control": sabotaged.iter().map(to_json).collect::<Vec<_>>(),
        });
        std::fs::write(
            dir.join("lattice-proptest.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("lattice-proptest.md"),
            markdown(&real, &sabotaged, cases),
        )
        .unwrap();
    }
}
