//! Replay tests. Size-exempt (`*_tests.rs`).

use super::*;
use crate::governance::audit::Evaluation;

const TS: &str = "2026-01-01T00:00:00Z";

/// A record for one constraint against one path.
fn rec(path: &str, result: &str, id: &str, outcome: &str, response: &str) -> TraceRecord {
    TraceRecord {
        kind: Some("guard".into()),
        result: Some(result.into()),
        mode: Some("advise".into()),
        path: Some(path.into()),
        constraints: vec![Evaluation {
            id: id.into(),
            class: Some("hard".into()),
            verification_point: Some("PAG".into()),
            hosted_at: None,
            outcome: Some(outcome.into()),
            response: Some(response.into()),
        }],
        ..TraceRecord::default()
    }
}

/// A clean record touching `path` and evaluating nothing.
fn clean(path: &str) -> TraceRecord {
    TraceRecord {
        kind: Some("guard".into()),
        result: Some("allow".into()),
        path: Some(path.into()),
        ..TraceRecord::default()
    }
}

/// Declare a policy in the store so Σ knows the constraint.
fn policy(store: &mut Store, label: &str, class: &str, effect: &str) {
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let iri = format!("http://ex/policy/{label}");
    let entity = store.intern(&iri).unwrap();
    let mut datums = vec![crate::store::Datum {
        entity,
        attribute: store.intern(crate::namespace::RDF_TYPE).unwrap(),
        value: crate::types::Value::Ref(store.intern(&format!("{ns}Policy")).unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    }];
    for (attribute, value) in [
        ("http://www.w3.org/2000/01/rdf-schema#label", label),
        (&format!("{ns}boundary"), "action"),
        (&format!("{ns}constraintClass"), class),
        (&format!("{ns}verificationPoint"), "PAG"),
        (&format!("{ns}effect"), effect),
    ] {
        datums.push(crate::store::Datum {
            entity,
            attribute: store.intern(attribute).unwrap(),
            value: crate::types::Value::Str(value.to_string()),
            valid_from: TS.to_string(),
            valid_to: None,
            op: crate::types::Op::Assert,
        });
    }
    store.transact(&datums, TS, None, None).unwrap();
}

fn only(report: &ReplayReport) -> &RuleStats {
    assert_eq!(report.rules.len(), 1, "{:#?}", report.rules);
    &report.rules[0]
}

#[test]
fn a_rule_that_never_fired_is_not_promotable() {
    // Promoting it would be enabling a rule nothing has tested.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [rec("a.rs", "allow", "c", "satisfied", "no-action")];
    let report = replay(&store, &trace, 0).unwrap();
    let stats = only(&report);
    assert!(!stats.is_live());
    assert!(stats.blocker().unwrap().contains("never fired"));
}

#[test]
fn a_rule_that_only_ever_fails_is_not_promotable() {
    // A check that always fails is universal or broken, and the record cannot
    // tell which.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("a.rs", "notify", "c", "unsatisfied", "warned"),
        rec("b.rs", "notify", "c", "unsatisfied", "warned"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert!(stats.is_live());
    assert!(!stats.is_two_sided());
    assert!(stats.blocker().unwrap().contains("always fails"));
}

#[test]
fn a_two_sided_live_rule_has_no_gate_objecting() {
    // The GREEN case. Without it every test here could be passing because the
    // harness rejects everything.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("a.rs", "notify", "c", "unsatisfied", "warned"),
        rec("b.rs", "allow", "c", "satisfied", "no-action"),
    ];
    let report = replay(&store, &trace, 0).unwrap();
    let stats = only(&report);
    assert!(stats.blocker().is_none(), "{:?}", stats.blocker());
    assert_eq!(report.promotable().len(), 1);
}

#[test]
fn new_blocks_counts_what_enforce_would_add_not_what_already_happened() {
    // The number an operator is actually deciding about.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("a.rs", "notify", "c", "unsatisfied", "warned"),
        rec("b.rs", "notify", "c", "unsatisfied", "warned"),
        rec("c.rs", "allow", "c", "satisfied", "no-action"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert_eq!(stats.would_block, 2);
    assert_eq!(stats.blocked, 0);
    assert_eq!(stats.new_blocks(), 2);
    assert_eq!(stats.targets, 2, "distinct targets it fired on");
}

#[test]
fn a_soft_constraint_would_never_block_however_its_effect_reads() {
    // A soft constraint that blocks is a hard one with a misleading name, so
    // replay must not promise blocks the runtime would refuse to make.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "soft", "deny");
    let trace = [rec("a.rs", "notify", "c", "unsatisfied", "warned")];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert_eq!(stats.would_block, 0);
    assert_eq!(stats.new_blocks(), 0);
}

#[test]
fn an_advisory_effect_would_not_start_blocking() {
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "warn");
    let trace = [rec("a.rs", "notify", "c", "unsatisfied", "warned")];
    assert_eq!(replay(&store, &trace, 0).unwrap().rules[0].would_block, 0);
}

#[test]
fn a_rule_nobody_has_ever_got_past_is_flagged() {
    // An outage with a reason attached. Live and two-sided, so the earlier gates
    // are satisfied and recoverability is the one doing the work — otherwise
    // this test would be passing on the wrong blocker.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("b.rs", "allow", "c", "satisfied", "no-action"),
        rec("a.rs", "deny", "c", "unsatisfied", "blocked"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert!(stats.is_live() && stats.is_two_sided());
    assert!(!stats.recoverable());
    assert!(
        stats.blocker().unwrap().contains("outage"),
        "{:?}",
        stats.blocker()
    );
}

#[test]
fn a_refusal_that_was_later_escaped_is_recoverable() {
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("a.rs", "deny", "c", "unsatisfied", "blocked"),
        clean("a.rs"),
        rec("b.rs", "allow", "c", "satisfied", "no-action"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert_eq!(stats.recovered, 1);
    assert!(stats.recoverable());
    assert!(stats.blocker().is_none(), "{:?}", stats.blocker());
}

#[test]
fn a_clean_record_before_the_refusal_does_not_count_as_escaping_it() {
    // Order matters. A target cleared BEFORE its refusal proves nothing about
    // whether anyone got past the rule, and counting it would turn every rule
    // that ever allowed anything into a recoverable one.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        clean("a.rs"),
        rec("z.rs", "allow", "c", "satisfied", "no-action"),
        rec("a.rs", "deny", "c", "unsatisfied", "blocked"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert_eq!(stats.recovered, 0);
    assert!(!stats.recoverable());
}

#[test]
fn an_unknown_outcome_counts_as_neither_side() {
    // A constraint that could not be evaluated has told you nothing, and folding
    // it into either column is how an unevaluated check reads as a passing one.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("a.rs", "allow", "c", "unknown", "no-action"),
        rec("b.rs", "notify", "c", "unsatisfied", "warned"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert_eq!(stats.evaluated, 2);
    assert_eq!(stats.satisfied, 0);
    assert_eq!(stats.unsatisfied, 1);
    assert!(!stats.is_two_sided(), "unknown is not a pass");
}

#[test]
fn a_rule_outside_the_spec_has_nothing_to_be_promoted_to() {
    let store = Store::open_in_memory().unwrap();
    let trace = [
        rec("a.rs", "notify", "local", "unsatisfied", "warned"),
        rec("b.rs", "allow", "local", "satisfied", "no-action"),
    ];
    let stats = replay(&store, &trace, 0).unwrap().rules.remove(0);
    assert!(!stats.in_spec);
    assert!(stats.blocker().unwrap().contains("not in Σ"));
}

#[test]
fn the_summary_carries_its_own_limits() {
    // A promotion number read without them is read as a safety claim, so the
    // caveat travels with the number rather than sitting in a footnote.
    let store = Store::open_in_memory().unwrap();
    let summary = replay(&store, &[], 0).unwrap().summary();
    assert!(summary.contains("only traffic that happened"), "{summary}");
    assert!(summary.contains("never false positives"), "{summary}");
    assert!(summary.contains("no false negatives"), "{summary}");
}

#[test]
fn replay_jsonl_reads_the_spec_from_the_store() {
    // The liveness half: everything above builds records in memory, and this is
    // what proves the entry point parses a real spool and loads a real Σ.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "no-ticket", "hard", "deny");
    let jsonl = concat!(
        r#"{"kind":"guard","mode":"advise","result":"notify","path":"a.rs","constraints":[{"id":"no-ticket","outcome":"unsatisfied","response":"warned"}]}"#,
        "\n",
        r#"{"kind":"guard","mode":"advise","result":"allow","path":"b.rs","constraints":[{"id":"no-ticket","outcome":"satisfied","response":"no-action"}]}"#,
        "\n",
        "not json\n"
    );
    let report = replay_jsonl(&store, jsonl).unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(
        report.unreadable, 1,
        "the garbage line is counted, not dropped"
    );
    let stats = only(&report);
    assert_eq!(stats.would_block, 1, "Σ came from the store");
    assert!(stats.line().contains("no gate objects"), "{}", stats.line());
}
