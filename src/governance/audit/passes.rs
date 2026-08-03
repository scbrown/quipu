//! The four correspondence passes.
//!
//! Each is a comparison between two declared values — never an inference, never
//! a model call. Every finding names the remedy, because a discrepancy report an
//! operator cannot act on is the dashboard anti-pattern with a conformance
//! claim attached.

use std::collections::BTreeSet;

use super::super::audit_spec::{Constraint, Spec};
use super::parse::{Evaluation, TraceRecord};
use super::{Discrepancy, Pass, Report, Severity};

/// Outcomes a trace may record. Anything else is a record the checker cannot
/// interpret, which is an incompleteness rather than a violation: the runtime
/// may simply be newer than the checker.
const OUTCOMES: &[&str] = &["satisfied", "unsatisfied", "unknown"];
/// Responses a trace may record.
const RESPONSES: &[&str] = &["blocked", "warned", "logged", "escalated", "no-action"];

fn finding(
    pass: Pass,
    severity: Severity,
    record: Option<usize>,
    constraint: Option<&str>,
    detail: String,
) -> Discrepancy {
    Discrepancy {
        pass,
        severity,
        record,
        constraint: constraint.map(str::to_string),
        detail,
    }
}

// ── Pass 1: coverage ─────────────────────────────────────────────────────────

/// Constraints cited versus constraints declared, in both directions the
/// checker can honestly test. See the module doc on [`super`] for the direction
/// it cannot — per-action applicability needs the selector.
pub(super) fn coverage(spec: &Spec, trace: &[TraceRecord], report: &mut Report) {
    let mut exercised: BTreeSet<&str> = BTreeSet::new();

    for (index, record) in trace.iter().enumerate() {
        for evaluation in &record.constraints {
            exercised.insert(evaluation.id.as_str());
            if spec.contains_key(&evaluation.id) {
                continue;
            }
            // A constraint enforced but not specified. This is I1 exactly: the
            // constraint set has to be explicit, and a rule that blocks edits
            // while being invisible to Σ cannot be audited, reviewed or
            // retired by anyone reading the spec.
            report.discrepancies.push(finding(
                Pass::Coverage,
                Severity::Violation,
                Some(index),
                Some(&evaluation.id),
                format!(
                    "the trace cites constraint '{}', which is not an \
                     action-boundary aegis:Policy in Σ. Most often this is a \
                     locally-configured rule enforcing outside the specification \
                     — author it in quipu so it can be audited, or stop \
                     enforcing it.",
                    evaluation.id
                ),
            ));
        }

        // A refusal has to be attributable to something. "Denied, by nothing in
        // particular" is the record an operator cannot appeal and an auditor
        // cannot check.
        if record.refused()
            && !record
                .constraints
                .iter()
                .any(|e| e.outcome.as_deref() == Some("unsatisfied"))
        {
            report.discrepancies.push(finding(
                Pass::Coverage,
                Severity::Violation,
                Some(index),
                None,
                "the action was refused but no constraint in the record is \
                 recorded as unsatisfied. A refusal must name the constraint \
                 that produced it."
                    .to_string(),
            ));
        }
    }

    // Vacuity. Not a violation — a constraint can be correct and simply not have
    // applied in this window — but it is the number that decides whether a
    // "T ⊨ Σ" result means anything, and SARC's operating points cannot be
    // calibrated from constraints that never fired.
    for (id, _) in spec
        .iter()
        .filter(|(id, _)| !exercised.contains(id.as_str()))
    {
        report.discrepancies.push(finding(
            Pass::Coverage,
            Severity::Incompleteness,
            None,
            Some(id),
            format!(
                "constraint '{id}' is declared in Σ but was not exercised \
                 anywhere in this window, so the trace says nothing about it. \
                 Widen the window, or add a fixture that fires it."
            ),
        ));
    }
}

// ── Pass 2: class ↔ placement ────────────────────────────────────────────────

/// SARC Table 3, applied to what the trace recorded — plus the trace-versus-Σ
/// comparison, which is where projection drift shows up.
pub(super) fn placement(spec: &Spec, trace: &[TraceRecord], report: &mut Report) {
    for (index, record) in trace.iter().enumerate() {
        for evaluation in &record.constraints {
            placement_of(spec, index, evaluation, report);
        }
    }
}

fn placement_of(spec: &Spec, index: usize, evaluation: &Evaluation, report: &mut Report) {
    let id = evaluation.id.as_str();
    let mut missing: Vec<&str> = Vec::new();
    if evaluation.class.is_none() {
        missing.push("class");
    }
    if evaluation.verification_point.is_none() {
        missing.push("verification_point");
    }
    if !missing.is_empty() {
        report.discrepancies.push(finding(
            Pass::Placement,
            Severity::Incompleteness,
            Some(index),
            Some(id),
            format!(
                "the record does not say the {} of '{id}', so its placement \
                 cannot be checked. Declare aegis:constraintClass and \
                 aegis:verificationPoint on the policy and re-project.",
                missing.join(" or ")
            ),
        ));
    }

    if let (Some(class), Some(point)) = (
        evaluation.class.as_deref(),
        evaluation.verification_point.as_deref(),
    ) {
        match crate::governance::placement::points_for(class) {
            None => report.discrepancies.push(finding(
                Pass::Placement,
                Severity::Violation,
                Some(index),
                Some(id),
                format!(
                    "'{id}' was evaluated as class \"{class}\", which is not one \
                     of hard/soft/escalation. Nothing can decide what placement \
                     or response it requires."
                ),
            )),
            Some(points) if !points.contains(&point) => {
                report.discrepancies.push(finding(
                    Pass::Placement,
                    Severity::Violation,
                    Some(index),
                    Some(id),
                    format!(
                        "'{id}' is class \"{class}\" but was evaluated at \
                         \"{point}\", which cannot enforce it. Permitted: {}.",
                        points.join(", ")
                    ),
                ));
            }
            Some(_) => {}
        }
    }

    // The trace against Σ. A record that disagrees with the store about a
    // constraint's own class is enforcement running on a stale projection, and
    // it is invisible to every other pass: each half is internally consistent.
    let Some(constraint) = spec.get(id) else {
        return;
    };
    drift(
        index,
        id,
        "class",
        evaluation.class.as_deref(),
        constraint.class.as_deref(),
        report,
    );
    drift(
        index,
        id,
        "verification point",
        evaluation.verification_point.as_deref(),
        constraint.point.as_deref(),
        report,
    );
}

fn drift(
    index: usize,
    id: &str,
    field: &str,
    recorded: Option<&str>,
    declared: Option<&str>,
    report: &mut Report,
) {
    let (Some(recorded), Some(declared)) = (recorded, declared) else {
        return;
    };
    if recorded == declared {
        return;
    }
    report.discrepancies.push(finding(
        Pass::Placement,
        Severity::Violation,
        Some(index),
        Some(id),
        format!(
            "'{id}' was evaluated with {field} \"{recorded}\" but Σ declares \
             \"{declared}\". The runtime is enforcing a stale projection — \
             re-sync it, and treat every decision in this window as made under \
             the older definition."
        ),
    ));
}

// ── Pass 3: outcome consistency ──────────────────────────────────────────────

/// Does the response taken match the response declared, at the recorded mode?
///
/// Mode-aware on purpose. `advise` has a declared ceiling — it never blocks —
/// so a hard `deny` constraint that only warned is *correct* under `advise` and
/// a violation under `enforce`. A check that ignored the mode would have to pick
/// one of those to be wrong about.
pub(super) fn outcome(spec: &Spec, trace: &[TraceRecord], report: &mut Report) {
    for (index, record) in trace.iter().enumerate() {
        let mode = record.mode.as_deref().unwrap_or("?");
        for evaluation in &record.constraints {
            outcome_of(spec.get(&evaluation.id), index, mode, evaluation, report);
        }
    }
}

fn outcome_of(
    constraint: Option<&Constraint>,
    index: usize,
    mode: &str,
    evaluation: &Evaluation,
    report: &mut Report,
) {
    let id = evaluation.id.as_str();
    let mut push = |severity, detail| {
        report.discrepancies.push(finding(
            Pass::Outcome,
            severity,
            Some(index),
            Some(id),
            detail,
        ));
    };

    let (Some(outcome), Some(response)) = (
        evaluation.outcome.as_deref(),
        evaluation.response.as_deref(),
    ) else {
        push(
            Severity::Incompleteness,
            format!("'{id}' recorded no outcome or no response, so nothing can be compared."),
        );
        return;
    };
    if !OUTCOMES.contains(&outcome) || !RESPONSES.contains(&response) {
        push(
            Severity::Incompleteness,
            format!(
                "'{id}' recorded outcome \"{outcome}\" / response \"{response}\", \
                 which this checker does not know. The runtime may be newer than \
                 the checker; verify before reading this window as conformant."
            ),
        );
        return;
    }

    if outcome == "satisfied" && response == "blocked" {
        push(
            Severity::Violation,
            format!(
                "'{id}' was satisfied and the action was blocked anyway. A \
                 constraint that holds cannot be the reason a refusal happened."
            ),
        );
    }
    if evaluation.class.as_deref() == Some("soft") && response == "blocked" {
        push(
            Severity::Violation,
            format!(
                "'{id}' is a SOFT constraint and it blocked. Soft constraints \
                 attach cost; they never gate admissibility. Either the class is \
                 wrong or the runtime is enforcing beyond its declaration."
            ),
        );
    }
    if mode == "advise" && response == "blocked" {
        push(
            Severity::Violation,
            format!(
                "'{id}' blocked while the mode was \"advise\". Advise has a \
                 declared ceiling and never refuses — this record is either \
                 mislabelled or the ceiling was not applied."
            ),
        );
    }
    if outcome == "unsatisfied" && response == "no-action" {
        push(
            Severity::Incompleteness,
            format!(
                "'{id}' was unsatisfied and the runtime did nothing about it. \
                 That is the state to grep for, not one to design toward: give \
                 the policy a declared aegis:effect, or retire it."
            ),
        );
    }

    let Some(constraint) = constraint else { return };
    declared_response(&mut push, constraint, mode, outcome, response);
}

/// The response Σ declares, against the one taken.
fn declared_response(
    push: &mut impl FnMut(Severity, String),
    constraint: &Constraint,
    mode: &str,
    outcome: &str,
    response: &str,
) {
    let Some(effect) = constraint.effect.as_deref() else {
        return;
    };
    let id = &constraint.id;

    // `record` means recorded only. Blocking on it is the runtime taking an
    // authority the policy did not grant, and it is worse than the reverse:
    // the operator sees enforcement they never declared.
    if effect == "record" && response == "blocked" {
        push(
            Severity::Violation,
            format!(
                "Σ declares aegis:effect \"record\" for '{id}' and the runtime \
                 blocked. Recording is not gating."
            ),
        );
    }

    // Under-enforcement is only decidable at `enforce`; anywhere else the mode
    // legitimately caps the response.
    if mode != "enforce" || outcome != "unsatisfied" {
        return;
    }
    match effect {
        "deny" if response != "blocked" => push(
            Severity::Violation,
            format!(
                "Σ declares aegis:effect \"deny\" for '{id}', the constraint was \
                 unsatisfied under \"enforce\", and the response was \
                 \"{response}\". A declared deny that does not refuse is a \
                 control believed in without being invoked."
            ),
        ),
        "escalate" | "require-approval" if !matches!(response, "escalated" | "blocked") => push(
            Severity::Violation,
            format!(
                "Σ declares aegis:effect \"{effect}\" for '{id}' and the response \
                 was \"{response}\". Escalation without a route is deferred \
                 autonomy: route it to an operator group, or fail closed."
            ),
        ),
        _ => {}
    }
}

// ── Pass 4: attribution completeness ─────────────────────────────────────────

/// Who is answerable. Scoped to records that evaluated a constraint — holding an
/// attribution requirement against a line that enforced nothing would report an
/// absence that is correct.
pub(super) fn attribution(trace: &[TraceRecord], report: &mut Report) {
    for (index, record) in trace.iter().enumerate() {
        if !record.is_enforcement() {
            continue;
        }
        // The declared chain's tail disagreeing with the process that ran is the
        // observable signature of a laundered chain (SARC §9.5), and it is the
        // one attribution finding that is a contradiction rather than a silence.
        if record.attribution_conflict {
            report.discrepancies.push(finding(
                Pass::Attribution,
                Severity::Violation,
                Some(index),
                None,
                "the declared principal chain's last link is not the executor \
                 that ran the action. Either the dispatch record names the wrong \
                 agent or an agent is acting under someone else's chain; both \
                 make every authority intersection over this record meaningless."
                    .to_string(),
            ));
        }

        let mut missing: Vec<&str> = Vec::new();
        if record.principal_chain.is_empty() {
            missing.push("principal_chain");
        }
        if record.planner.is_none() {
            missing.push("planner");
        }
        if record.executor.is_none() {
            missing.push("executor");
        }
        if record.tool.is_none() {
            missing.push("tool");
        }
        if missing.is_empty() {
            continue;
        }
        // One finding per record listing everything absent, rather than one per
        // absent element: four lines saying the same thing about the same record
        // is how a report becomes something nobody reads.
        report.discrepancies.push(finding(
            Pass::Attribution,
            Severity::Incompleteness,
            Some(index),
            None,
            format!(
                "α is partial — no {}. Without the chain, the effective \
                 authority for this action cannot be derived, so SARC §9.3's \
                 intersection is unavailable to the audit.",
                missing.join(", no ")
            ),
        ));
    }
}
