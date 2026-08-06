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
    policy_at(store, label, class, effect, TS);
}

fn policy_at(store: &mut Store, label: &str, class: &str, effect: &str, ts: &str) {
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let iri = format!("http://ex/policy/{label}");
    let entity = store.intern(&iri).unwrap();
    let mut datums = vec![crate::store::Datum {
        entity,
        attribute: store.intern(crate::namespace::RDF_TYPE).unwrap(),
        value: crate::types::Value::Ref(store.intern(&format!("{ns}Policy")).unwrap()),
        valid_from: ts.to_string(),
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
            valid_from: ts.to_string(),
            valid_to: None,
            op: crate::types::Op::Assert,
        });
    }
    store.transact(&datums, ts, None, None).unwrap();
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

// ---------------------------------------------------------------------------
// quipu #72 — as-of Σ: fidelity and drift as SEPARATE columns
// ---------------------------------------------------------------------------

/// Re-class a policy's `constraintClass` at a later timestamp, the way a spec
/// edit after the trace window does.
fn reclass(store: &mut Store, label: &str, old_class: &str, new_class: &str, at: &str) {
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let entity = store.intern(&format!("http://ex/policy/{label}")).unwrap();
    let attribute = store.intern(&format!("{ns}constraintClass")).unwrap();
    // A re-class RETRACTS the old value and asserts the new. Asserting alone
    // would leave BOTH — `constraintClass` is an ordinary RDF predicate, not a
    // functional one, so a bare assert adds a second class rather than
    // replacing the first. (Found the hard way: the drift test reported no
    // movement because live Σ still saw `hard`.)
    store
        .transact(
            &[
                crate::store::Datum {
                    entity,
                    attribute,
                    value: crate::types::Value::Str(old_class.to_string()),
                    valid_from: at.to_string(),
                    valid_to: None,
                    op: crate::types::Op::Retract,
                },
                crate::store::Datum {
                    entity,
                    attribute,
                    value: crate::types::Value::Str(new_class.to_string()),
                    valid_from: at.to_string(),
                    valid_to: None,
                    op: crate::types::Op::Assert,
                },
            ],
            at,
            None,
            None,
        )
        .unwrap();
}

#[test]
fn no_as_of_is_behaviour_identical_to_today() {
    // #72 acceptance 2. Live Σ stays the default, and the drift column is empty
    // — "not asked", never "nothing moved".
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [
        rec("a.rs", "notify", "c", "unsatisfied", "warned"),
        rec("b.rs", "allow", "c", "satisfied", "no-action"),
    ];
    let live = replay(&store, &trace, 0).unwrap();
    let explicit = replay_as_of(&store, &trace, 0, None).unwrap();
    assert_eq!(live, explicit, "replay() is replay_as_of(.., None)");
    assert!(
        live.drift.is_empty(),
        "no window asked for, so no drift claimed"
    );
}

#[test]
fn a_reclass_after_the_window_is_drift_not_a_trace_violation() {
    // #72 acceptance 1, the whole point of the issue.
    //
    // The trace ran while `c` was HARD. Afterwards the spec re-classed it to
    // SOFT. Judged against live Σ the trace looks wrong; judged against Σ as of
    // its own window it was right, and the re-class is spec movement.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    reclass(&mut store, "c", "hard", "soft", "2026-06-01T00:00:00Z");

    let trace = [
        rec("a.rs", "notify", "c", "unsatisfied", "warned"),
        rec("b.rs", "allow", "c", "satisfied", "no-action"),
    ];
    // VALID-TIME, not as_of_tx. Retraction records `valid_to` as a timestamp
    // and leaves the row's original `tx` untouched, so there is no retraction
    // transaction anywhere in `facts` — `as_of_tx` therefore cannot tell a fact
    // that was live at tx N from one retracted since. Valid-time can, because
    // the window is stored on the row. See the module note on this limit.
    let as_of = crate::store::AsOf {
        tx: None,
        valid_at: Some("2026-03-01T00:00:00Z".into()),
    };
    let report = replay_as_of(&store, &trace, 0, Some(&as_of)).unwrap();

    // FIDELITY: judged against Σ-then, the constraint was in spec and the trace
    // is evaluated normally — NOT reported as citing something unknown.
    let stats = only(&report);
    assert!(
        stats.in_spec,
        "as of the trace's own window the policy existed; fidelity must judge against THAT"
    );

    // DRIFT: the re-class is reported, in its own column, as spec movement.
    assert_eq!(report.drift.len(), 1, "{:#?}", report.drift);
    let d = &report.drift[0];
    assert_eq!(d.field, "class");
    assert_eq!(
        d.then.as_deref(),
        Some("hard"),
        "enforcement was judged against hard"
    );
    assert_eq!(d.now.as_deref(), Some("soft"));
    assert!(
        d.line().contains("not a trace violation"),
        "phrased as movement, not fault: {}",
        d.line()
    );
}

#[test]
fn an_unmoved_spec_reports_no_drift_even_when_asked() {
    // The control. Without this, the drift test could pass because `as_of`
    // reports drift for everything rather than because it detected a re-class.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let trace = [rec("a.rs", "allow", "c", "satisfied", "no-action")];
    let report = replay_as_of(
        &store,
        &trace,
        0,
        Some(&crate::store::AsOf {
            tx: None,
            valid_at: Some("2026-03-01T00:00:00Z".into()),
        }),
    )
    .unwrap();
    assert!(
        report.drift.is_empty(),
        "nothing moved, so nothing is reported: {:#?}",
        report.drift
    );
}

#[test]
fn a_policy_added_after_the_window_was_not_in_scope_then() {
    // A policy that did not exist at trace time could not have been evaluated
    // then. Fidelity must say so rather than counting the trace as having
    // missed a rule that had not been written yet.
    let mut store = Store::open_in_memory().unwrap();
    policy_at(&mut store, "new", "hard", "deny", "2026-06-01T00:00:00Z");

    let trace = [rec("a.rs", "allow", "new", "satisfied", "no-action")];
    let report = replay_as_of(
        &store,
        &trace,
        0,
        Some(&crate::store::AsOf {
            tx: None,
            valid_at: Some("2026-03-01T00:00:00Z".into()),
        }),
    )
    .unwrap();
    let stats = only(&report);
    assert!(
        !stats.in_spec,
        "as of the window, 'new' did not exist — fidelity judges against Σ-then"
    );
    // And live Σ does know it, which is what makes the above meaningful.
    assert!(replay(&store, &trace, 0).unwrap().rules[0].in_spec);
}

#[test]
fn as_of_tx_now_reconstructs_a_retracted_policy_too() {
    // WAS a pinned LIMIT (#72), now a pinned FIX (quipu #83).
    //
    // #72 recorded that `as_of_tx` could not see a fact retracted since: the
    // retraction set `valid_to` to a timestamp and left the row's `tx` alone,
    // so no retracting transaction was recorded, while the query still required
    // present-tense liveness. #83 added `facts.retracted_tx` and made the as-of
    // predicate `valid_to IS NULL OR retracted_tx > N`.
    //
    // Kept rather than deleted, and kept asserting BOTH axes: the limit is the
    // reason this test exists, and a future change that silently reintroduced
    // it would otherwise have nothing watching.
    let mut store = Store::open_in_memory().unwrap();
    policy(&mut store, "c", "hard", "deny");
    let tx = store.latest_tx_id().unwrap();
    reclass(&mut store, "c", "hard", "soft", "2026-06-01T00:00:00Z");

    let by_tx = crate::governance::audit_spec::load_as_of(
        &store,
        Some(&crate::store::AsOf {
            tx: Some(tx),
            valid_at: None,
        }),
    )
    .unwrap();
    assert_eq!(
        by_tx.get("c").and_then(|c| c.class.clone()),
        Some("hard".into()),
        "as_of_tx now reconstructs the retracted value (quipu #83)"
    );

    let by_time = crate::governance::audit_spec::load_as_of(
        &store,
        Some(&crate::store::AsOf {
            tx: None,
            valid_at: Some("2026-03-01T00:00:00Z".into()),
        }),
    )
    .unwrap();
    assert_eq!(
        by_time.get("c").and_then(|c| c.class.clone()),
        Some("hard".into()),
        "valid-time still works, and the two axes now agree"
    );

    // And live Σ still sees the CURRENT value — the fix must not make as-of
    // leak into the default read.
    assert_eq!(
        crate::governance::audit_spec::load(&store)
            .unwrap()
            .get("c")
            .and_then(|c| c.class.clone()),
        Some("soft".into()),
        "the default read is unchanged"
    );
}
