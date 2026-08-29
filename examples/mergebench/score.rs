//! Scoring. Every metric is computed from the three graphs, the oracle, and
//! the strategy's own output — never from the generator's intent.
//!
//! The two headline numbers are deliberately in tension. `human_decisions` is
//! what the strategy charged a person; `shacl_violations` is the corruption it
//! admitted without charging anyone. A strategy can always drive either to
//! zero by sacrificing the other, so neither is reportable alone.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::generate::{ConflictClass, Scenario};
use crate::model::{self, Graph, Slot};
use crate::shapes;
use crate::strategies::{self, Outcome};

/// One arm's scored result.
#[derive(serde::Serialize)]
pub struct ArmMetrics {
    /// Arm name, as in `strategies::ARMS`.
    pub arm: String,
    /// False when the arm could not run here. Read this before reading a zero.
    pub available: bool,
    /// Slots handed to a human.
    pub human_decisions: usize,
    /// Human decisions per 1000 applied edits.
    pub decisions_per_1k_edits: f64,
    /// Correctly identified conflicts.
    pub true_positives: usize,
    /// Slots flagged that the oracle says need no decision.
    pub false_positives: usize,
    /// Conflicts the oracle declares and the arm did not flag.
    pub false_negatives: usize,
    /// `tp / (tp + fp)`; null when the arm flagged nothing.
    pub precision: Option<f64>,
    /// `tp / (tp + fn)`; null when the oracle declares no conflict.
    pub recall: Option<f64>,
    /// Harmonic mean of the two, when both are defined.
    pub f1: Option<f64>,
    /// Recall broken out by conflict class — the column that says WHICH
    /// conflicts an arm can see, rather than how many.
    pub recall_by_class: BTreeMap<String, ClassRecall>,
    /// SHACL violations in the auto-merged graph, against the same shapes that
    /// defined the conflicts. This is corruption the arm admitted silently.
    pub shacl_violations: usize,
    /// Whether the auto-merged graph conforms outright.
    pub shacl_conforms: bool,
    /// Triples the oracle-guided reference lands that this arm did not.
    pub triples_lost: usize,
    /// Triples this arm landed that the reference does not.
    pub triples_spurious: usize,
    /// Output lines that were not well-formed RDF.
    pub unparseable_lines: usize,
    /// Merge wall time, microseconds. Excludes SHACL validation.
    pub merge_us: u128,
    /// SHACL validation wall time, microseconds.
    pub validate_us: u128,
}

/// Per-class detection counts.
#[derive(serde::Serialize)]
pub struct ClassRecall {
    /// Conflicts of this class the oracle declares.
    pub declared: usize,
    /// How many of them the arm flagged.
    pub detected: usize,
}

/// Score every arm against one scenario.
#[must_use]
pub fn score(scenario: &Scenario) -> Vec<ArmMetrics> {
    let shapes_ttl = shapes::turtle();
    let validator = quipu::Validator::from_turtle(&shapes_ttl).expect("benchmark shapes parse");
    let truth_slots: BTreeSet<Slot> = scenario.truth.keys().cloned().collect();
    let reference = strategies::ideal(
        &scenario.base,
        &scenario.ours,
        &scenario.theirs,
        &truth_slots,
    );

    let mut classes: BTreeMap<ConflictClass, usize> = BTreeMap::new();
    for class in scenario.truth.values() {
        *classes.entry(*class).or_default() += 1;
    }

    strategies::ARMS
        .iter()
        .map(|arm| {
            let t0 = Instant::now();
            let outcome = strategies::run(arm, &scenario.base, &scenario.ours, &scenario.theirs);
            let merge_us = t0.elapsed().as_micros();

            let t1 = Instant::now();
            let (violations, conforms) = if outcome.available {
                let data = model::to_canonical_nt(&outcome.merged);
                match validator.validate(data.as_bytes()) {
                    Ok(fb) => (fb.violations, fb.conforms),
                    // A validator error is not a conforming graph. Reported as
                    // maximal corruption rather than as a pass, so a broken
                    // validator can never flatter an arm.
                    Err(_) => (usize::MAX, false),
                }
            } else {
                (0, false)
            };
            let validate_us = t1.elapsed().as_micros();

            metrics(
                arm,
                &outcome,
                scenario,
                &truth_slots,
                &classes,
                &reference,
                violations,
                conforms,
                merge_us,
                validate_us,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn metrics(
    arm: &str,
    outcome: &Outcome,
    scenario: &Scenario,
    truth: &BTreeSet<Slot>,
    classes: &BTreeMap<ConflictClass, usize>,
    reference: &Graph,
    shacl_violations: usize,
    shacl_conforms: bool,
    merge_us: u128,
    validate_us: u128,
) -> ArmMetrics {
    let tp = outcome.conflicts.intersection(truth).count();
    let fp = outcome.conflicts.difference(truth).count();
    let fn_ = truth.difference(&outcome.conflicts).count();
    let precision = (tp + fp > 0).then(|| tp as f64 / (tp + fp) as f64);
    let recall = (tp + fn_ > 0).then(|| tp as f64 / (tp + fn_) as f64);
    let f1 = match (precision, recall) {
        (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
        _ => None,
    };

    let mut recall_by_class = BTreeMap::new();
    for (class, declared) in classes {
        let detected = scenario
            .truth
            .iter()
            .filter(|(slot, c)| *c == class && outcome.conflicts.contains(*slot))
            .count();
        recall_by_class.insert(
            class.as_str().to_string(),
            ClassRecall {
                declared: *declared,
                detected,
            },
        );
    }

    ArmMetrics {
        arm: arm.to_string(),
        available: outcome.available,
        human_decisions: outcome.conflicts.len(),
        decisions_per_1k_edits: if scenario.edits == 0 {
            0.0
        } else {
            outcome.conflicts.len() as f64 * 1000.0 / scenario.edits as f64
        },
        true_positives: tp,
        false_positives: fp,
        false_negatives: fn_,
        precision,
        recall,
        f1,
        recall_by_class,
        shacl_violations,
        shacl_conforms,
        triples_lost: reference.difference(&outcome.merged).count(),
        triples_spurious: outcome.merged.difference(reference).count(),
        unparseable_lines: outcome.unparseable_lines,
        merge_us,
        validate_us,
    }
}
