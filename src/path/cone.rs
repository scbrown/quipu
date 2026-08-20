//! The provenance cone: which steps did the verified result depend on?
//!
//! A step is IN the cone when something it produced flows — along the
//! derivation predicates — into a falsifier-gated verification of the
//! trajectory's result. A step outside the cone contributed nothing the
//! verified result depends on: mechanically prunable. A step with no
//! recorded derivation edges CANNOT be evaluated, and says so.
//!
//! ## Why forward reachability, not speculative removal
//!
//! The design (`golden-paths-blessing.md` §3) sketched this on
//! `Store::speculate`: remove step S, ask whether the verification chain
//! survives. Building it surfaced why that test is unsound here: retraction
//! removes only S's OWN facts, so a chain that continues through edges owned
//! by S's output artifacts survives S's removal — and a load-bearing step
//! reads as prunable, which is exactly the direction this analysis must
//! never fail in. Forward reachability ("did anything S produced flow into
//! the verified result") is the relation the design's prose actually
//! defines, and it is what this implements, on the same bounded BFS as
//! `quipu impact`.

use serde::Serialize;

use crate::error::{Error, Result};
use crate::impact::{ImpactOptions, impact};
use crate::store::Store;

use super::PathVocab;
use super::read::{admissible_verifications, ref_values, require_entity, steps_of};

/// Default BFS depth for the derivation walk.
pub const DEFAULT_CONE_HOPS: usize = 8;

/// Options for a cone computation.
#[derive(Debug, Clone)]
pub struct ConeOptions {
    /// Derivation predicates to walk, in addition to `verifiedBy` (which is
    /// always followed — it is the edge that reaches the verification).
    pub via: Vec<String>,
    /// Depth bound for the derivation walk.
    pub hops: usize,
}

impl Default for ConeOptions {
    fn default() -> Self {
        Self {
            via: Vec::new(),
            hops: DEFAULT_CONE_HOPS,
        }
    }
}

/// One step's cone verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ConeVerdict {
    /// Something this step produced flows into a verified result. Pruning it
    /// requires a human Decision.
    InCone,
    /// Nothing this step produced reaches a verified result: mechanically
    /// prunable.
    OutOfCone,
    /// The step records no outgoing derivation edges, so the question cannot
    /// be answered. Not prunable — missing data degrades toward human
    /// ruling, never toward guessing.
    CannotEvaluate,
}

/// One step in a cone report.
#[derive(Debug, Clone, Serialize)]
pub struct ConeStep {
    pub iri: String,
    pub order: Option<i64>,
    pub verdict: ConeVerdict,
    /// The human-readable basis for the verdict.
    pub reason: String,
}

/// The cone of one trajectory.
#[derive(Debug, Clone, Serialize)]
pub struct ConeReport {
    pub trajectory: String,
    /// The falsifier-gated verifications the cone was computed against.
    pub verifications: Vec<String>,
    /// The derivation predicates that were walked.
    pub derivation_predicates: Vec<String>,
    pub hops: usize,
    pub steps: Vec<ConeStep>,
}

/// Compute the provenance cone of `trajectory_iri`.
///
/// Refuses (rather than returning an empty report) when the trajectory has
/// no steps or no falsifier-gated verification — a cone against an
/// unverified result would make everything prunable against nothing.
pub fn cone(
    store: &Store,
    trajectory_iri: &str,
    vocab: &PathVocab,
    opts: &ConeOptions,
) -> Result<ConeReport> {
    let traj_id = require_entity(store, trajectory_iri)?;
    let steps = steps_of(store, vocab, traj_id)?;
    if steps.is_empty() {
        return Err(Error::InvalidValue(format!(
            "trajectory has no steps: {trajectory_iri} — nothing to compute a cone over"
        )));
    }
    let verifications = admissible_verifications(store, vocab, &steps)?;
    if verifications.is_empty() {
        return Err(Error::InvalidValue(format!(
            "trajectory {trajectory_iri} has no falsifier-gated verification — not admissible; \
             a cone against an unverified result would mark every step prunable against nothing"
        )));
    }

    let mut derivation_predicates = opts.via.clone();
    if !derivation_predicates.contains(&vocab.verified_by) {
        derivation_predicates.push(vocab.verified_by.clone());
    }
    let walk = ImpactOptions {
        hops: opts.hops,
        predicates: derivation_predicates.clone(),
    };
    let verification_iris: Vec<String> = verifications.iter().map(|(_, v)| v.clone()).collect();

    let mut report_steps = Vec::with_capacity(steps.len());
    for step in &steps {
        let mut outgoing = 0usize;
        for pred in &derivation_predicates {
            outgoing += ref_values(store, step.id, pred)?.len();
        }
        let (verdict, reason) = if outgoing == 0 {
            (
                ConeVerdict::CannotEvaluate,
                "no outgoing derivation edges recorded — cannot say what this step fed".to_string(),
            )
        } else {
            let reached = impact(store, &step.iri, &walk)?;
            match reached
                .reached
                .iter()
                .find(|n| verification_iris.contains(&n.iri))
            {
                Some(hit) => (
                    ConeVerdict::InCone,
                    format!("reaches {} at depth {}", hit.iri, hit.depth),
                ),
                None => (
                    ConeVerdict::OutOfCone,
                    format!(
                        "no verified result reachable within {} hops over {} derivation edge(s)",
                        opts.hops, outgoing
                    ),
                ),
            }
        };
        report_steps.push(ConeStep {
            iri: step.iri.clone(),
            order: step.order,
            verdict,
            reason,
        });
    }

    Ok(ConeReport {
        trajectory: trajectory_iri.to_string(),
        verifications: verification_iris,
        derivation_predicates,
        hops: opts.hops,
        steps: report_steps,
    })
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    #[test]
    fn refuses_a_trajectory_with_no_steps() {
        let (mut store, vocab) = seed_empty();
        intern(&mut store, "http://ex/traj-bare");
        let err = cone(
            &store,
            "http://ex/traj-bare",
            &vocab,
            &ConeOptions::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no steps"), "{err}");
    }

    #[test]
    fn refuses_an_unverified_trajectory() {
        let (mut store, vocab) = seed_unverified_trajectory();
        let err = cone(&store, TRAJ, &vocab, &ConeOptions::default()).unwrap_err();
        assert!(
            err.to_string().contains("no falsifier-gated verification"),
            "{err}"
        );
        let _ = &mut store;
    }

    #[test]
    fn classifies_in_out_and_cannot_evaluate() {
        let (store, vocab) = seed_verified_trajectory();
        let opts = ConeOptions {
            via: vec![PRODUCES.to_string(), CONSUMED_BY.to_string()],
            ..Default::default()
        };
        let report = cone(&store, TRAJ, &vocab, &opts).unwrap();
        let verdicts: Vec<(&str, &ConeVerdict)> = report
            .steps
            .iter()
            .map(|s| (s.iri.as_str(), &s.verdict))
            .collect();
        assert_eq!(
            verdicts,
            vec![
                ("http://ex/s1-implement", &ConeVerdict::InCone),
                ("http://ex/s2-detour", &ConeVerdict::OutOfCone),
                ("http://ex/s3-test", &ConeVerdict::InCone),
                ("http://ex/s4-verify", &ConeVerdict::InCone),
                ("http://ex/s5-mail", &ConeVerdict::CannotEvaluate),
            ],
            "{report:?}"
        );
    }

    #[test]
    fn an_unfalsifiable_verification_does_not_anchor_the_cone() {
        // The detour step gains a verifiedBy edge to a verification WITHOUT a
        // falsifier: it must stay out of the verification set, and the
        // detour's verdict must remain OutOfCone rather than becoming InCone
        // by reaching an assertion in a verification's clothes.
        let (mut store, vocab) = seed_verified_trajectory();
        edge(
            &mut store,
            "http://ex/s2-detour",
            &vocab.verified_by.clone(),
            "http://ex/verif-eyeball",
        );
        let opts = ConeOptions {
            via: vec![PRODUCES.to_string(), CONSUMED_BY.to_string()],
            ..Default::default()
        };
        let report = cone(&store, TRAJ, &vocab, &opts).unwrap();
        assert_eq!(report.verifications, vec!["http://ex/verif".to_string()]);
        let detour = report
            .steps
            .iter()
            .find(|s| s.iri == "http://ex/s2-detour")
            .unwrap();
        assert_eq!(detour.verdict, ConeVerdict::OutOfCone, "{report:?}");
    }
}
