//! Audit-checker tests. Size-exempt (`*_tests.rs`).
//!
//! Every pass gets both halves: a case that must be flagged and the control that
//! must not. A checker only tested on its RED cases proves it can complain, not
//! that it can tell the difference.

use super::*;
use crate::governance::audit_spec::{Constraint, Spec};

fn constraint(id: &str, class: &str, point: &str, effect: &str) -> Constraint {
    Constraint {
        iri: format!("http://ex/policy/{id}"),
        id: id.to_string(),
        class: Some(class.to_string()),
        point: Some(point.to_string()),
        effect: Some(effect.to_string()),
        hosted_at_layer: None,
        inherited_by_delegates: None,
    }
}

fn spec_of(constraints: &[Constraint]) -> Spec {
    constraints
        .iter()
        .map(|c| (c.id.clone(), c.clone()))
        .collect()
}

/// A record with one evaluation, attributed completely so the attribution pass
/// stays quiet unless a test is about it.
fn record(mode: &str, result: &str, evaluation: Evaluation) -> TraceRecord {
    TraceRecord {
        kind: Some("guard".into()),
        result: Some(result.into()),
        mode: Some(mode.into()),
        constraints: vec![evaluation],
        principal_chain: vec!["orchestrator".into(), "worker".into()],
        planner: Some("orchestrator".into()),
        executor: Some("worker".into()),
        tool: Some("Edit".into()),
        ..TraceRecord::default()
    }
}

fn evaluation(id: &str, class: &str, point: &str, outcome: &str, response: &str) -> Evaluation {
    Evaluation {
        id: id.into(),
        class: Some(class.into()),
        verification_point: Some(point.into()),
        hosted_at: None,
        outcome: Some(outcome.into()),
        response: Some(response.into()),
    }
}

/// Run the passes without a store, so a test is about one pass and not about
/// SPARQL. `check` composes the same four over a loaded Σ.
fn run(spec: &Spec, trace: &[TraceRecord]) -> Report {
    let mut report = Report {
        records_checked: trace.len(),
        constraints_in_scope: spec.len(),
        ..Report::default()
    };
    super::passes::coverage(spec, trace, &mut report);
    super::passes::placement(spec, trace, &mut report);
    super::passes::outcome(spec, trace, &mut report);
    super::passes::attribution(trace, &mut report);
    report
}

fn violations(report: &Report, pass: Pass) -> Vec<&Discrepancy> {
    report
        .discrepancies
        .iter()
        .filter(|d| d.pass == pass && d.severity == Severity::Violation)
        .collect()
}

// ── The conformant baseline ──────────────────────────────────────────────────

#[test]
fn a_conformant_window_has_no_violations_and_no_incompleteness() {
    // The control that everything else is measured against. Without it, every
    // RED test below could be passing because the checker flags everything.
    let spec = spec_of(&[constraint("no-ticket", "hard", "PAG", "deny")]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("no-ticket", "hard", "PAG", "unsatisfied", "blocked"),
    )];
    let report = run(&spec, &trace);
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    assert!(report.is_complete(), "{:#?}", report.discrepancies);
    assert!(report.summary().contains("T ⊨ Σ"), "{}", report.summary());
}

// ── Pass 1: coverage ─────────────────────────────────────────────────────────

#[test]
fn a_constraint_enforced_outside_the_spec_is_a_violation() {
    // SARC I1: the constraint set has to be explicit. A rule that blocks edits
    // while being invisible to Σ cannot be audited, reviewed or retired.
    let spec = spec_of(&[]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("local-only", "hard", "PAG", "unsatisfied", "blocked"),
    )];
    let report = run(&spec, &trace);
    let found = violations(&report, Pass::Coverage);
    assert_eq!(found.len(), 1, "{:#?}", report.discrepancies);
    assert!(
        found[0].detail.contains("author it in quipu"),
        "names the remedy"
    );
}

#[test]
fn a_refusal_attributed_to_no_constraint_is_a_violation() {
    // "Denied, by nothing in particular" is the record an operator cannot appeal.
    let spec = spec_of(&[constraint("c", "hard", "PAG", "deny")]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("c", "hard", "PAG", "satisfied", "no-action"),
    )];
    let report = run(&spec, &trace);
    assert!(
        violations(&report, Pass::Coverage)
            .iter()
            .any(|d| d.detail.contains("no constraint in the record")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn the_control_an_allow_needs_no_unsatisfied_constraint() {
    let spec = spec_of(&[constraint("c", "hard", "PAG", "deny")]);
    let trace = [record(
        "enforce",
        "allow",
        evaluation("c", "hard", "PAG", "satisfied", "no-action"),
    )];
    assert!(violations(&run(&spec, &trace), Pass::Coverage).is_empty());
}

#[test]
fn a_constraint_never_exercised_is_reported_but_does_not_break_conformance() {
    // Vacuity is the number that decides whether "T ⊨ Σ" means anything — and it
    // is not a contradiction, because a constraint can be correct and simply not
    // have applied in this window.
    let spec = spec_of(&[
        constraint("fired", "hard", "PAG", "deny"),
        constraint("dormant", "hard", "PAG", "deny"),
    ]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("fired", "hard", "PAG", "unsatisfied", "blocked"),
    )];
    let report = run(&spec, &trace);
    assert!(report.conforms(), "vacuity is not a contradiction");
    assert!(!report.is_complete());
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .any(|d| d.constraint.as_deref() == Some("dormant")),
        "{:#?}",
        report.discrepancies
    );
}

// ── Pass 2: class ↔ placement ────────────────────────────────────────────────

#[test]
fn a_soft_constraint_evaluated_at_the_gate_is_a_violation() {
    // SARC Table 3: soft constraints attach cost to partial or completed action
    // data, which the PAG does not have.
    let spec = spec_of(&[]);
    let trace = [record(
        "advise",
        "notify",
        evaluation("todo-needs-ticket", "soft", "PAG", "unsatisfied", "warned"),
    )];
    let report = run(&spec, &trace);
    assert!(
        violations(&report, Pass::Placement)
            .iter()
            .any(|d| d.detail.contains("cannot enforce it")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn the_control_a_soft_constraint_at_the_paa_is_fine() {
    let spec = spec_of(&[]);
    let trace = [record(
        "advise",
        "notify",
        evaluation("todo-needs-ticket", "soft", "PAA", "unsatisfied", "warned"),
    )];
    assert!(violations(&run(&spec, &trace), Pass::Placement).is_empty());
}

#[test]
fn the_placement_table_is_the_one_the_write_gate_uses() {
    // Two copies of SARC Table 3 would eventually disagree — and the
    // disagreement would be between the definition-time check and the
    // audit-time one, the two places that must not.
    for class in ["hard", "soft", "escalation"] {
        let points = crate::governance::placement::points_for(class).unwrap();
        for point in points {
            let trace = [record(
                "enforce",
                "allow",
                evaluation("c", class, point, "satisfied", "no-action"),
            )];
            assert!(
                violations(&run(&spec_of(&[]), &trace), Pass::Placement).is_empty(),
                "{class} at {point} is permitted at write time but flagged at audit time"
            );
        }
    }
}

#[test]
fn a_record_that_disagrees_with_the_spec_about_a_class_is_a_violation() {
    // Projection drift is invisible to every other pass: each half is internally
    // consistent, and only the comparison catches it.
    let spec = spec_of(&[constraint("c", "soft", "PAA", "warn")]);
    let trace = [record(
        "enforce",
        "allow",
        evaluation("c", "hard", "PAG", "satisfied", "no-action"),
    )];
    let report = run(&spec, &trace);
    let found = violations(&report, Pass::Placement);
    assert_eq!(found.len(), 2, "class AND point drift: {found:#?}");
    assert!(found[0].detail.contains("stale projection"));
}

#[test]
fn an_undeclared_class_is_an_incompleteness_not_a_violation() {
    // A locally-configured rule or a pre-Phase-1 catalog is under-described, not
    // wrong. Flagging it as a contradiction would make every legacy deployment
    // permanently non-conformant.
    let spec = spec_of(&[]);
    let mut e = evaluation("c", "hard", "PAG", "satisfied", "no-action");
    e.class = None;
    e.verification_point = None;
    let trace = [record("enforce", "allow", e)];
    let report = run(&spec, &trace);
    assert!(violations(&report, Pass::Placement).is_empty());
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .any(|d| d.pass == Pass::Placement),
        "{:#?}",
        report.discrepancies
    );
}

// ── Pass 3: outcome consistency ──────────────────────────────────────────────

#[test]
fn a_soft_constraint_that_blocked_is_a_violation() {
    // Soft constraints attach cost; they never gate admissibility.
    let spec = spec_of(&[]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("c", "soft", "PAA", "unsatisfied", "blocked"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome)
            .iter()
            .any(|d| d.detail.contains("SOFT")),
    );
}

#[test]
fn a_declared_deny_that_only_warned_under_enforce_is_a_violation() {
    // "A control believed in without being invoked" is the failure the whole
    // enforcement gradient exists to prevent.
    let spec = spec_of(&[constraint("c", "hard", "PAG", "deny")]);
    let trace = [record(
        "enforce",
        "notify",
        evaluation("c", "hard", "PAG", "unsatisfied", "warned"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome)
            .iter()
            .any(|d| d.detail.contains("does not refuse")),
    );
}

#[test]
fn the_control_the_same_record_under_advise_conforms() {
    // Advise has a declared ceiling. A check that ignored the mode would have to
    // pick one of these two records to be wrong about.
    let spec = spec_of(&[constraint("c", "hard", "PAG", "deny")]);
    let trace = [record(
        "advise",
        "notify",
        evaluation("c", "hard", "PAG", "unsatisfied", "warned"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome).is_empty(),
        "a hard deny that warns is CORRECT under advise"
    );
}

#[test]
fn blocking_under_advise_is_a_violation() {
    // The other side of the ceiling.
    let spec = spec_of(&[constraint("c", "hard", "PAG", "deny")]);
    let trace = [record(
        "advise",
        "deny",
        evaluation("c", "hard", "PAG", "unsatisfied", "blocked"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome)
            .iter()
            .any(|d| d.detail.contains("advise")),
    );
}

#[test]
fn a_satisfied_constraint_that_blocked_is_a_violation() {
    let spec = spec_of(&[constraint("c", "hard", "PAG", "deny")]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("c", "hard", "PAG", "satisfied", "blocked"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome)
            .iter()
            .any(|d| d.detail.contains("cannot be the reason")),
    );
}

#[test]
fn a_record_effect_that_blocked_is_a_violation() {
    // The runtime taking an authority the policy did not grant.
    let spec = spec_of(&[constraint("c", "hard", "PAG", "record")]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("c", "hard", "PAG", "unsatisfied", "blocked"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome)
            .iter()
            .any(|d| d.detail.contains("Recording is not gating")),
    );
}

#[test]
fn an_escalation_that_only_warned_is_deferred_autonomy() {
    let spec = spec_of(&[constraint("c", "escalation", "PAG", "escalate")]);
    let trace = [record(
        "enforce",
        "notify",
        evaluation("c", "escalation", "PAG", "unsatisfied", "warned"),
    )];
    assert!(
        violations(&run(&spec, &trace), Pass::Outcome)
            .iter()
            .any(|d| d.detail.contains("deferred autonomy")),
    );
}

#[test]
fn the_control_an_escalation_that_routed_conforms() {
    let spec = spec_of(&[constraint("c", "escalation", "PAG", "escalate")]);
    let trace = [record(
        "enforce",
        "deny",
        evaluation("c", "escalation", "PAG", "unsatisfied", "escalated"),
    )];
    assert!(violations(&run(&spec, &trace), Pass::Outcome).is_empty());
}

#[test]
fn a_constraint_that_fired_and_did_nothing_is_reported() {
    let spec = spec_of(&[]);
    let trace = [record(
        "enforce",
        "allow",
        evaluation("c", "hard", "PAG", "unsatisfied", "no-action"),
    )];
    let report = run(&spec, &trace);
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .any(|d| d.detail.contains("grep for")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn an_outcome_the_checker_does_not_know_is_an_incompleteness() {
    // The runtime may be newer than the checker. Calling that a violation would
    // make every upgrade look like a governance failure.
    let spec = spec_of(&[]);
    let trace = [record(
        "enforce",
        "allow",
        evaluation("c", "hard", "PAG", "deferred", "no-action"),
    )];
    let report = run(&spec, &trace);
    assert!(violations(&report, Pass::Outcome).is_empty());
    assert!(!report.is_complete());
}

// ── Pass 4: attribution ──────────────────────────────────────────────────────

#[test]
fn a_laundered_chain_is_a_violation() {
    let spec = spec_of(&[]);
    let mut r = record(
        "enforce",
        "allow",
        evaluation("c", "hard", "PAG", "satisfied", "no-action"),
    );
    r.attribution_conflict = true;
    let report = run(&spec, &[r]);
    assert!(
        violations(&report, Pass::Attribution)
            .iter()
            .any(|d| d.detail.contains("acting under someone else's chain")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn a_partial_tuple_is_one_finding_that_names_everything_absent() {
    // Four lines saying the same thing about the same record is how a report
    // becomes something nobody reads.
    let spec = spec_of(&[]);
    let mut r = record(
        "enforce",
        "allow",
        evaluation("c", "hard", "PAG", "satisfied", "no-action"),
    );
    r.principal_chain.clear();
    r.planner = None;
    let report = run(&spec, &[r]);
    let found: Vec<_> = report
        .discrepancies
        .iter()
        .filter(|d| d.pass == Pass::Attribution)
        .collect();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].detail.contains("principal_chain"), "{found:#?}");
    assert!(found[0].detail.contains("planner"), "{found:#?}");
    assert_eq!(found[0].severity, Severity::Incompleteness);
}

#[test]
fn a_line_that_enforced_nothing_is_not_held_to_attribution() {
    // The spool carries `command` and `fail_open` lines too; demanding a chain
    // of them would report an absence that is correct.
    let spec = spec_of(&[]);
    let trace = [TraceRecord {
        kind: Some("command".into()),
        ..TraceRecord::default()
    }];
    let report = run(&spec, &trace);
    assert!(
        report
            .discrepancies
            .iter()
            .all(|d| d.pass != Pass::Attribution),
        "{:#?}",
        report.discrepancies
    );
}

// ── Parsing, and the report's own coverage of its input ──────────────────────

#[test]
fn an_unreadable_line_is_counted_never_skipped() {
    // A checker that silently dropped it would claim conformance over a window
    // it had only partly read.
    let (records, unreadable) = parse_trace("{\"kind\":\"guard\"}\nnot json\n\n{}\n");
    assert_eq!(records.len(), 2);
    assert_eq!(
        unreadable, 1,
        "the blank line is not a defect; the garbage is"
    );
}

#[test]
fn an_unknown_field_does_not_stop_the_read() {
    // A checker that refused a spool because hank had added a field would stop
    // auditing at exactly the moment the two repos drifted.
    let (records, unreadable) = parse_trace(r#"{"kind":"guard","brand_new_field":42}"#);
    assert_eq!(unreadable, 0);
    assert_eq!(records[0].kind.as_deref(), Some("guard"));
}

#[test]
fn the_summary_states_the_unreadable_count_even_at_zero() {
    // Coverage of the input is the one number a reader must not have to infer
    // from an omission.
    let report = Report {
        records_checked: 3,
        constraints_in_scope: 1,
        ..Report::default()
    };
    assert!(
        report.summary().contains("0 line(s) unreadable"),
        "{}",
        report.summary()
    );
}

// ── End to end, through the real store ───────────────────────────────────────

#[test]
fn check_jsonl_reads_the_spec_from_the_store() {
    // The liveness half: the passes above run on a hand-built Spec, and this is
    // the one that proves `check` actually loads Σ from the graph.
    let mut store = Store::open_in_memory().unwrap();
    let iri = "http://ex/policy/p";
    let entity = store.intern(iri).unwrap();
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let ts = "2026-01-01T00:00:00Z";
    let mut datums = vec![crate::store::Datum {
        entity,
        attribute: store.intern(crate::namespace::RDF_TYPE).unwrap(),
        value: crate::types::Value::Ref(store.intern(&format!("{ns}Policy")).unwrap()),
        valid_from: ts.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    }];
    for (attribute, value) in [
        ("http://www.w3.org/2000/01/rdf-schema#label", "no-ticket"),
        (&format!("{ns}boundary"), "action"),
        (&format!("{ns}constraintClass"), "hard"),
        (&format!("{ns}verificationPoint"), "PAG"),
        (&format!("{ns}effect"), "deny"),
    ] {
        datums.push(crate::store::Datum {
            entity,
            attribute: store.intern(attribute).unwrap(),
            value: crate::types::Value::Str(value.to_string()),
            valid_from: ts.to_string(),
            valid_to: None,
            op: crate::types::Op::Assert,
        });
    }
    store.transact(&datums, ts, None, None).unwrap();

    let jsonl = r#"{"kind":"guard","mode":"enforce","result":"deny","principal_chain":["a"],"planner":"a","executor":"a","tool":"Edit","constraints":[{"id":"no-ticket","class":"hard","verification_point":"PAG","outcome":"unsatisfied","response":"blocked"}]}"#;
    let report = check_jsonl(&store, jsonl).unwrap();
    assert_eq!(report.constraints_in_scope, 1, "Σ came from the store");
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    assert!(report.is_complete(), "{:#?}", report.discrepancies);
}

#[test]
fn check_jsonl_catches_the_drift_it_exists_for() {
    // The RED half of the end-to-end: the store says soft/PAA, the trace says
    // hard/PAG, and only the comparison against the real Σ can see it.
    let mut store = Store::open_in_memory().unwrap();
    let iri = "http://ex/policy/p";
    let entity = store.intern(iri).unwrap();
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let ts = "2026-01-01T00:00:00Z";
    let mut datums = vec![crate::store::Datum {
        entity,
        attribute: store.intern(crate::namespace::RDF_TYPE).unwrap(),
        value: crate::types::Value::Ref(store.intern(&format!("{ns}Policy")).unwrap()),
        valid_from: ts.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    }];
    for (attribute, value) in [
        ("http://www.w3.org/2000/01/rdf-schema#label", "drifty"),
        (&format!("{ns}boundary"), "action"),
        (&format!("{ns}constraintClass"), "soft"),
        (&format!("{ns}verificationPoint"), "PAA"),
        (&format!("{ns}effect"), "warn"),
    ] {
        datums.push(crate::store::Datum {
            entity,
            attribute: store.intern(attribute).unwrap(),
            value: crate::types::Value::Str(value.to_string()),
            valid_from: ts.to_string(),
            valid_to: None,
            op: crate::types::Op::Assert,
        });
    }
    store.transact(&datums, ts, None, None).unwrap();

    let jsonl = r#"{"kind":"guard","mode":"enforce","result":"allow","principal_chain":["a"],"planner":"a","executor":"a","tool":"Edit","constraints":[{"id":"drifty","class":"hard","verification_point":"PAG","outcome":"satisfied","response":"no-action"}]}"#;
    let report = check_jsonl(&store, jsonl).unwrap();
    assert!(!report.conforms(), "{:#?}", report.discrepancies);
    assert!(report.summary().contains("T ⊭ Σ"), "{}", report.summary());
}

// ── I6: the claimed hosting layer against the recorded one ───────────────────

/// A record whose single evaluation carries a recorded hosting layer.
fn hosted(id: &str, layer: &str) -> TraceRecord {
    let mut e = evaluation(id, "hard", "PAG", "satisfied", "no-action");
    e.hosted_at = Some(layer.into());
    record("enforce", "allow", e)
}

fn with_layer(id: &str, layer: &str) -> Constraint {
    let mut c = constraint(id, "hard", "PAG", "deny");
    c.hosted_at_layer = Some(layer.into());
    c
}

#[test]
fn a_policy_claiming_a_layer_stronger_than_the_one_that_ran_it_is_a_violation() {
    // The I6 failure: it reads as enforced somewhere an agent cannot route
    // around while being enforced somewhere an agent can.
    let spec = spec_of(&[with_layer("secrets-guard", "tool")]);
    let report = run(&spec, &[hosted("secrets-guard", "orchestration")]);
    let found = violations(&report, Pass::Placement);
    assert!(
        found.iter().any(|d| d.detail.contains("route around")),
        "{:#?}",
        report.discrepancies
    );
    assert!(found[0].detail.contains("and be right"), "names the remedy");
}

#[test]
fn the_control_an_honest_layer_claim_is_silent() {
    let spec = spec_of(&[with_layer("p", "orchestration")]);
    let report = run(&spec, &[hosted("p", "orchestration")]);
    assert!(
        violations(&report, Pass::Placement).is_empty(),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn an_understated_layer_claim_is_silent() {
    // The asymmetry: declaring weaker than the truth understates a constraint's
    // own robustness and misleads nobody in a direction that costs them.
    let spec = spec_of(&[with_layer("p", "orchestration")]);
    let report = run(&spec, &[hosted("p", "policy")]);
    assert!(
        violations(&report, Pass::Placement).is_empty(),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn i6_is_undecidable_when_either_half_is_missing() {
    // A claim with no record, or a record with no claim, says nothing. Reporting
    // either as a violation would flag every pre-I6 policy in the catalog.
    let spec = spec_of(&[with_layer("p", "tool")]);
    let no_record = run(
        &spec,
        &[record(
            "enforce",
            "allow",
            evaluation("p", "hard", "PAG", "satisfied", "no-action"),
        )],
    );
    assert!(violations(&no_record, Pass::Placement).is_empty());

    let spec = spec_of(&[constraint("p", "hard", "PAG", "deny")]);
    let no_claim = run(&spec, &[hosted("p", "orchestration")]);
    assert!(violations(&no_claim, Pass::Placement).is_empty());
}

#[test]
fn an_unknown_layer_is_undecidable_not_a_violation() {
    let spec = spec_of(&[with_layer("p", "prompt")]);
    let report = run(&spec, &[hosted("p", "orchestration")]);
    assert!(violations(&report, Pass::Placement).is_empty());
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .any(|d| d.detail.contains("cannot be decided")),
        "{:#?}",
        report.discrepancies
    );
}
