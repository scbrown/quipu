//! Backtest a candidate golden path over recorded history.
//!
//! Given an exemplar trajectory and the steps a candidate omits, compile the
//! candidate's v1 pattern and replay it over every other trajectory whose
//! work item shares a topic with the exemplar's: who would have conformed,
//! and how did their work items close? The report is what a human promotes
//! on, so `0 matches` and `cannot evaluate` are kept apart — the same
//! discipline as the governance backtest.

use serde::Serialize;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

use super::PathVocab;
use super::grammar::{GRAMMAR_VERSION, MatchOutcome, StepSig, match_pattern};
use super::read::{ref_values, require_entity, step_sig, steps_of, str_value};

/// How one historical trajectory fared against the candidate pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RowResult {
    Conforms,
    DeviatesAt {
        pattern_index: usize,
    },
    /// Every step of the trajectory is unevaluable (or it has none) — this
    /// row is evidence of missing data, not of non-conformance.
    CannotEvaluate,
}

/// One historical trajectory in the backtest.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestRow {
    pub trajectory: String,
    pub workitem: String,
    /// The work item's close disposition; absence means open.
    pub outcome: Option<String>,
    pub result: RowResult,
    /// Steps that carried no v1 signature and were skipped by matching.
    pub unevaluated_steps: usize,
}

/// The backtest report a promotion cites.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestReport {
    /// The grammar version this replay evaluated with (the carry rule).
    pub grammar: &'static str,
    pub exemplar: String,
    /// The compiled candidate pattern.
    pub pattern: Vec<StepSig>,
    /// The topics applicability matched on.
    pub topics: Vec<String>,
    pub rows: Vec<BacktestRow>,
    pub conformers_done: usize,
    pub conformers_total: usize,
    pub deviators_done: usize,
    pub deviators_total: usize,
    pub cannot_evaluate: usize,
}

/// Backtest the candidate obtained from `exemplar_iri` by omitting
/// `omitted_step_iris`.
///
/// Refuses when the pattern cannot be compiled: a kept step with no
/// `actionKind` has no v1 signature, and a pattern with a hole in it would
/// silently match something other than the path a human thinks they are
/// promoting.
pub fn backtest(
    store: &Store,
    exemplar_iri: &str,
    omitted_step_iris: &[String],
    vocab: &PathVocab,
) -> Result<BacktestReport> {
    let exemplar_id = require_entity(store, exemplar_iri)?;
    let steps = steps_of(store, vocab, exemplar_id)?;
    if steps.is_empty() {
        return Err(Error::InvalidValue(format!(
            "exemplar trajectory has no steps: {exemplar_iri}"
        )));
    }

    // Compile the pattern: kept steps, in order, each with a signature.
    let mut pattern = Vec::new();
    let mut signatureless = Vec::new();
    for step in &steps {
        if omitted_step_iris.contains(&step.iri) {
            continue;
        }
        match step_sig(store, vocab, step.id)? {
            Some(sig) => pattern.push(sig),
            None => signatureless.push(step.iri.clone()),
        }
    }
    if !signatureless.is_empty() {
        return Err(Error::InvalidValue(format!(
            "cannot compile the candidate pattern: kept step(s) with no actionKind \
             (no {} signature): {}. Omit them or record their action kinds — a pattern \
             with holes would match something other than what is being promoted",
            GRAMMAR_VERSION,
            signatureless.join(", ")
        )));
    }
    if pattern.is_empty() {
        return Err(Error::InvalidValue(
            "the candidate pattern is empty after omissions — nothing to backtest".to_string(),
        ));
    }

    // Applicability: trajectories of work items sharing a topic with the
    // exemplar's work item. The deterministic core only — no similarity.
    let exemplar_wi = ref_values(store, exemplar_id, &vocab.trajectory_of)?;
    let topics: Vec<String> = match exemplar_wi.first() {
        Some(wi) => topic_strings(store, vocab, *wi)?,
        None => Vec::new(),
    };
    if topics.is_empty() {
        return Err(Error::InvalidValue(format!(
            "cannot establish applicability: the exemplar's work item records no topic \
             (no `about` value reachable from {exemplar_iri}) — a backtest over an \
             unbounded population would count everything and mean nothing"
        )));
    }

    let mut rows = Vec::new();
    let (mut c_done, mut c_total, mut d_done, mut d_total, mut cannot) = (0, 0, 0, 0, 0);
    for (traj_id, traj_iri, wi_id, wi_iri) in
        applicable_trajectories(store, vocab, exemplar_id, &topics)?
    {
        let steps = steps_of(store, vocab, traj_id)?;
        let sigs: Vec<Option<StepSig>> = steps
            .iter()
            .map(|s| step_sig(store, vocab, s.id))
            .collect::<Result<_>>()?;
        let unevaluated = sigs.iter().filter(|s| s.is_none()).count();
        let outcome = str_value(store, wi_id, &vocab.outcome)?;

        let result = if sigs.iter().all(Option::is_none) {
            cannot += 1;
            RowResult::CannotEvaluate
        } else {
            match match_pattern(&pattern, &sigs) {
                MatchOutcome::Conforms => {
                    c_total += 1;
                    if outcome.as_deref() == Some("done") {
                        c_done += 1;
                    }
                    RowResult::Conforms
                }
                MatchOutcome::DeviatesAt { pattern_index, .. } => {
                    d_total += 1;
                    if outcome.as_deref() == Some("done") {
                        d_done += 1;
                    }
                    RowResult::DeviatesAt { pattern_index }
                }
            }
        };
        rows.push(BacktestRow {
            trajectory: traj_iri,
            workitem: wi_iri,
            outcome,
            result,
            unevaluated_steps: unevaluated,
        });
    }

    Ok(BacktestReport {
        grammar: GRAMMAR_VERSION,
        exemplar: exemplar_iri.to_string(),
        pattern,
        topics,
        rows,
        conformers_done: c_done,
        conformers_total: c_total,
        deviators_done: d_done,
        deviators_total: d_total,
        cannot_evaluate: cannot,
    })
}

/// String `about` topics of a work item.
fn topic_strings(store: &Store, vocab: &PathVocab, wi: i64) -> Result<Vec<String>> {
    let Some(attr) = store.lookup(&vocab.about)? else {
        return Ok(Vec::new());
    };
    let mut topics = Vec::new();
    for fact in store.entity_facts(wi)? {
        if fact.attribute == attr
            && let Value::Str(s) = &fact.value
        {
            topics.push(s.clone());
        }
    }
    Ok(topics)
}

/// Trajectories (other than the exemplar) whose work item shares one of the
/// exemplar's topics: `(trajectory id, trajectory IRI, workitem id, workitem IRI)`.
fn applicable_trajectories(
    store: &Store,
    vocab: &PathVocab,
    exemplar_id: i64,
    topics: &[String],
) -> Result<Vec<(i64, String, i64, String)>> {
    let Some(traj_of) = store.lookup(&vocab.trajectory_of)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for fact in store.current_facts()? {
        if fact.attribute != traj_of || fact.entity == exemplar_id {
            continue;
        }
        let Value::Ref(wi) = fact.value else {
            continue;
        };
        let wi_topics = topic_strings(store, vocab, wi)?;
        if wi_topics.iter().any(|t| topics.contains(t)) {
            out.push((
                fact.entity,
                store.resolve(fact.entity)?,
                wi,
                store.resolve(wi)?,
            ));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    /// Extend the cone fixture with a work item + topic for the exemplar and
    /// two follower trajectories: one conforming (done), one deviating
    /// (failed), plus an off-topic trajectory that must not be counted and a
    /// signature-less one that must be `CannotEvaluate`.
    fn seed_backtest() -> (Store, PathVocab) {
        let (mut store, vocab) = seed_verified_trajectory();
        let v = vocab.clone();

        edge(&mut store, TRAJ, &v.trajectory_of, "http://ex/wi");
        lit(&mut store, "http://ex/wi", &v.about, "service-deploy");

        // A sixth exemplar step with no actionKind: it has no v1 signature,
        // so a candidate KEEPING it cannot compile.
        edge(&mut store, "http://ex/s6-note", &v.step_of, TRAJ);
        int(&mut store, "http://ex/s6-note", &v.step_order, 6);

        // Conformer: edit(literal-free targets) -> run -> verify, done.
        edge(
            &mut store,
            "http://ex/t-follow",
            &v.trajectory_of,
            "http://ex/wi-follow",
        );
        lit(
            &mut store,
            "http://ex/wi-follow",
            &v.about,
            "service-deploy",
        );
        lit(&mut store, "http://ex/wi-follow", &v.outcome, "done");
        for (iri, n, k) in [
            ("http://ex/f1", 1, "edit"),
            ("http://ex/f2", 2, "run"),
            ("http://ex/f3", 3, "verify"),
        ] {
            edge(&mut store, iri, &v.step_of, "http://ex/t-follow");
            int(&mut store, iri, &v.step_order, n);
            lit(&mut store, iri, &v.action_kind, k);
        }

        // Deviator: verify before run, failed.
        edge(
            &mut store,
            "http://ex/t-stray",
            &v.trajectory_of,
            "http://ex/wi-stray",
        );
        lit(&mut store, "http://ex/wi-stray", &v.about, "service-deploy");
        lit(&mut store, "http://ex/wi-stray", &v.outcome, "failed");
        for (iri, n, k) in [("http://ex/x1", 1, "edit"), ("http://ex/x2", 2, "verify")] {
            edge(&mut store, iri, &v.step_of, "http://ex/t-stray");
            int(&mut store, iri, &v.step_order, n);
            lit(&mut store, iri, &v.action_kind, k);
        }

        // Off-topic: must not enter the population.
        edge(
            &mut store,
            "http://ex/t-other",
            &v.trajectory_of,
            "http://ex/wi-other",
        );
        lit(&mut store, "http://ex/wi-other", &v.about, "retry-loop");

        // On-topic but signature-less: CannotEvaluate, not a deviator.
        edge(
            &mut store,
            "http://ex/t-blind",
            &v.trajectory_of,
            "http://ex/wi-blind",
        );
        lit(&mut store, "http://ex/wi-blind", &v.about, "service-deploy");
        lit(&mut store, "http://ex/wi-blind", &v.outcome, "done");
        edge(&mut store, "http://ex/b1", &v.step_of, "http://ex/t-blind");
        int(&mut store, "http://ex/b1", &v.step_order, 1);

        (store, vocab)
    }

    /// The candidate: omit the detour and the mail step (which has no
    /// signature and would otherwise refuse compilation).
    fn omissions() -> Vec<String> {
        vec![
            "http://ex/s2-detour".to_string(),
            "http://ex/s5-mail".to_string(),
            "http://ex/s6-note".to_string(),
        ]
    }

    #[test]
    fn separates_conformers_deviators_and_cannot_evaluate() {
        let (store, vocab) = seed_backtest();
        let report = backtest(&store, TRAJ, &omissions(), &vocab).unwrap();
        assert_eq!(report.grammar, GRAMMAR_VERSION);
        assert_eq!(report.pattern.len(), 3, "{:?}", report.pattern);
        assert_eq!(report.rows.len(), 3, "{:?}", report.rows);
        assert_eq!((report.conformers_done, report.conformers_total), (1, 1));
        assert_eq!((report.deviators_done, report.deviators_total), (0, 1));
        assert_eq!(report.cannot_evaluate, 1);
        let blind = report
            .rows
            .iter()
            .find(|r| r.trajectory.contains("t-blind"))
            .unwrap();
        assert_eq!(blind.result, RowResult::CannotEvaluate);
    }

    #[test]
    fn off_topic_trajectories_are_not_in_the_population() {
        let (store, vocab) = seed_backtest();
        let report = backtest(&store, TRAJ, &omissions(), &vocab).unwrap();
        assert!(
            !report.rows.iter().any(|r| r.trajectory.contains("t-other")),
            "{:?}",
            report.rows
        );
    }

    #[test]
    fn a_kept_step_without_a_signature_refuses_compilation() {
        let (store, vocab) = seed_backtest();
        // Keep s6-note (no actionKind): the pattern would have a hole.
        let err = backtest(
            &store,
            TRAJ,
            &[
                "http://ex/s2-detour".to_string(),
                "http://ex/s5-mail".to_string(),
            ],
            &vocab,
        )
        .unwrap_err();
        assert!(err.to_string().contains("s6-note"), "{err}");
    }

    #[test]
    fn refuses_when_no_topic_bounds_the_population() {
        let (store, vocab) = seed_verified_trajectory();
        let err = backtest(&store, TRAJ, &omissions(), &vocab).unwrap_err();
        assert!(err.to_string().contains("no topic"), "{err}");
    }
}
