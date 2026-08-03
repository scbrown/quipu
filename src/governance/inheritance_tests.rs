//! Constraint-inheritance tests. Size-exempt (`*_tests.rs`).

use super::*;
use crate::governance::audit::Evaluation;

const TS: &str = "2026-01-01T00:00:00Z";

/// Declare a policy, optionally inherited by delegates.
fn policy(store: &mut Store, label: &str, inherited: bool) {
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let entity = store.intern(&format!("http://ex/policy/{label}")).unwrap();
    let mut datums = vec![crate::store::Datum {
        entity,
        attribute: store.intern(crate::namespace::RDF_TYPE).unwrap(),
        value: crate::types::Value::Ref(store.intern(&format!("{ns}Policy")).unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    }];
    let mut fields = vec![
        (
            "http://www.w3.org/2000/01/rdf-schema#label".to_string(),
            label.to_string(),
        ),
        (format!("{ns}boundary"), "action".to_string()),
    ];
    if inherited {
        fields.push((format!("{ns}inheritedByDelegates"), "true".to_string()));
        fields.push((format!("{ns}onUndecidable"), "escalate".to_string()));
    }
    for (attribute, value) in fields {
        datums.push(crate::store::Datum {
            entity,
            attribute: store.intern(&attribute).unwrap(),
            value: crate::types::Value::Str(value),
            valid_from: TS.to_string(),
            valid_to: None,
            op: crate::types::Op::Assert,
        });
    }
    store.transact(&datums, TS, None, None).unwrap();
}

/// A record under `chain` touching `path`, evaluating `ids`.
fn rec(chain: &[&str], path: &str, ids: &[&str]) -> TraceRecord {
    TraceRecord {
        kind: Some("guard".into()),
        result: Some("allow".into()),
        path: Some(path.into()),
        principal_chain: chain.iter().map(|s| (*s).to_string()).collect(),
        constraints: ids
            .iter()
            .map(|id| Evaluation {
                id: (*id).into(),
                outcome: Some("satisfied".into()),
                response: Some("no-action".into()),
                ..Evaluation::default()
            })
            .collect(),
        ..TraceRecord::default()
    }
}

fn violations(report: &Report) -> Vec<&Discrepancy> {
    report.of(Severity::Violation)
}

#[test]
fn a_constraint_dropped_deeper_on_a_target_it_already_decided_is_a_violation() {
    // The STRONG case: it decided for this target once, so its absence deeper
    // is a drop rather than an inapplicability.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [
        rec(&["orchestrator"], "src/a.rs", &["no-secrets"]),
        rec(&["orchestrator", "worker"], "src/a.rs", &[]),
    ];
    let report = check(&store, &trace).unwrap();
    let found = violations(&report);
    assert_eq!(found.len(), 1, "{:#?}", report.discrepancies);
    assert!(found[0].detail.contains("is a DROP"), "{}", found[0].detail);
    assert!(found[0].detail.contains("or escalate"), "names the rescue");
}

#[test]
fn the_control_a_deeper_action_that_does_evaluate_it_is_fine() {
    // Without this the test above proves only that the pass can complain.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [
        rec(&["orchestrator"], "src/a.rs", &["no-secrets"]),
        rec(&["orchestrator", "worker"], "src/a.rs", &["no-secrets"]),
    ];
    assert!(
        violations(&check(&store, &trace).unwrap()).is_empty(),
        "an inherited constraint that WAS carried down must not be flagged"
    );
}

#[test]
fn a_different_target_deeper_is_not_a_drop() {
    // The constraint decided for a.rs; nothing says it applies to b.rs, and
    // claiming so would be inventing the selector quipu does not have.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [
        rec(&["orchestrator"], "src/a.rs", &["no-secrets"]),
        rec(&["orchestrator", "worker"], "src/b.rs", &[]),
        rec(&["orchestrator", "worker"], "src/b.rs", &["no-secrets"]),
    ];
    assert!(violations(&check(&store, &trace).unwrap()).is_empty());
}

#[test]
fn a_sibling_chain_is_not_below_and_is_not_a_drop() {
    // `is_below` is a prefix test on purpose. A different branch of the tree is
    // not a delegation of this one, and flagging it would make every parallel
    // worker look like a laundering event.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [
        rec(&["orchestrator", "left"], "src/a.rs", &["no-secrets"]),
        rec(&["orchestrator", "right"], "src/a.rs", &[]),
    ];
    assert!(violations(&check(&store, &trace).unwrap()).is_empty());
}

#[test]
fn a_shallower_action_after_a_deeper_one_is_not_a_drop() {
    // Direction matters: returning to the orchestrator is not delegating.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [
        rec(&["orchestrator", "worker"], "src/a.rs", &["no-secrets"]),
        rec(&["orchestrator"], "src/a.rs", &[]),
    ];
    assert!(violations(&check(&store, &trace).unwrap()).is_empty());
}

#[test]
fn a_constraint_that_is_not_declared_inherited_is_not_checked() {
    // Inheritance is a declaration. Assuming it would make every constraint's
    // ordinary scope look like a laundering event.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "local-only", false);
    let trace = [
        rec(&["orchestrator"], "src/a.rs", &["local-only"]),
        rec(&["orchestrator", "worker"], "src/a.rs", &[]),
    ];
    let report = check(&store, &trace).unwrap();
    assert!(violations(&report).is_empty());
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .any(|d| d.detail.contains("means none has said")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn a_spec_declaring_no_inheritance_says_so_rather_than_reporting_clean() {
    // Otherwise a deployment that never declared inheritance reads as one where
    // inheritance was checked and found clean.
    let store = Store::open_in_memory().unwrap();
    let report = check(&store, &[]).unwrap();
    assert!(report.conforms());
    assert!(!report.is_complete());
}

#[test]
fn a_constraint_that_stops_at_the_delegation_boundary_is_the_weak_finding() {
    // Might be laundering, might be a selector that matched nothing deeper, and
    // the record cannot tell. Reported — but not as a contradiction, because
    // calling it one would flag every constraint that simply did not apply.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [
        rec(&["orchestrator", "worker"], "src/a.rs", &["no-secrets"]),
        rec(&["orchestrator", "worker", "deep"], "src/b.rs", &[]),
    ];
    let report = check(&store, &trace).unwrap();
    assert!(
        violations(&report).is_empty(),
        "{:#?}",
        report.discrepancies
    );
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .any(|d| d.detail.contains("cannot tell them apart")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn a_constraint_evaluated_at_a_root_with_no_delegation_says_nothing() {
    // Inheritance was never exercised, so there is nothing to report and a
    // finding here would be noise on every single-agent deployment.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let trace = [rec(&["solo"], "src/a.rs", &["no-secrets"])];
    let report = check(&store, &trace).unwrap();
    assert!(report.conforms());
    assert!(
        report
            .of(Severity::Incompleteness)
            .iter()
            .all(|d| d.constraint.is_none()),
        "no per-constraint finding: {:#?}",
        report.discrepancies
    );
}

#[test]
fn check_jsonl_carries_the_unreadable_count() {
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-secrets", true);
    let jsonl = concat!(
        r#"{"kind":"guard","path":"a.rs","principal_chain":["o"],"constraints":[{"id":"no-secrets","outcome":"satisfied","response":"no-action"}]}"#,
        "\n",
        r#"{"kind":"guard","path":"a.rs","principal_chain":["o","w"]}"#,
        "\n",
        "not json\n"
    );
    let report = check_jsonl(&store, jsonl).unwrap();
    assert_eq!(report.records_unreadable, 1);
    assert!(!report.conforms(), "{:#?}", report.discrepancies);
}
