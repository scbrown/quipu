//! Placement-rule tests.
//!
//! The rules are pure functions over a [`Placement`], so the table below tests
//! them directly; the write-path wiring is exercised separately in
//! `governance_tests.rs`. Every case here is a REJECTION with a named reason —
//! a validation that cannot reject asserts nothing, so the conformant cases are
//! paired with the near-miss that must fail.

use super::*;

fn action(class: Option<&str>, point: Option<&str>) -> Placement {
    Placement {
        boundary: Some("action".to_string()),
        class: class.map(str::to_string),
        point: point.map(str::to_string),
        reversibility_window: None,
        on_timeout: None,
        ambiguous: Vec::new(),
    }
}

fn escalation(point: &str, window: Option<&str>, timeout: Option<&str>) -> Placement {
    Placement {
        boundary: Some("action".to_string()),
        class: Some("escalation".to_string()),
        point: Some(point.to_string()),
        reversibility_window: window.map(str::to_string),
        on_timeout: timeout.map(str::to_string),
        ambiguous: Vec::new(),
    }
}

#[test]
fn each_class_is_accepted_at_every_point_table_3_permits() {
    for point in HARD_POINTS {
        assert!(
            action(Some("hard"), Some(point)).violation("p").is_none(),
            "hard must be permitted at {point}"
        );
    }
    for point in SOFT_POINTS {
        assert!(
            action(Some("soft"), Some(point)).violation("p").is_none(),
            "soft must be permitted at {point}"
        );
    }
    for point in ESCALATION_POINTS {
        assert!(
            escalation(point, Some("600"), Some("deny"))
                .violation("p")
                .is_none(),
            "escalation must be permitted at {point}"
        );
    }
}

#[test]
fn a_hard_constraint_at_the_post_action_auditor_is_rejected() {
    // The case this module exists for. A hard rule evaluated after the action
    // completed documents the violation it was meant to prevent — and reads as
    // governed the whole time.
    let why = action(Some("hard"), Some("PAA"))
        .violation("aegis:p")
        .unwrap();
    assert!(
        why.contains("aegis:p"),
        "the reason names the policy: {why}"
    );
    assert!(
        why.contains("cannot prevent it"),
        "the reason must say WHY, not just that it is disallowed: {why}"
    );
    assert!(
        why.contains("PAG"),
        "the reason must name a point that would work: {why}"
    );
}

#[test]
fn a_soft_constraint_at_the_pre_action_gate_is_rejected() {
    let why = action(Some("soft"), Some("PAG")).violation("p").unwrap();
    assert!(
        why.contains("no completed-action data"),
        "reason should explain the missing evidence: {why}"
    );
}

#[test]
fn an_escalation_mid_flight_is_rejected() {
    // The ATM has no seam at which to suspend and await a ruling.
    let why = escalation("ATM", Some("600"), Some("deny"))
        .violation("p")
        .unwrap();
    assert!(why.contains("suspend"), "{why}");
}

#[test]
fn an_action_policy_without_a_class_is_rejected() {
    // SARC I2: `effect` alone cannot say what KIND of bound this is.
    let why = action(None, Some("PAG")).violation("p").unwrap();
    assert!(why.contains("constraintClass"), "{why}");
    assert!(
        why.contains("hard, soft, escalation"),
        "the reason lists the permitted values: {why}"
    );
}

#[test]
fn an_action_policy_without_a_verification_point_is_rejected() {
    let why = action(Some("hard"), None).violation("p").unwrap();
    assert!(why.contains("verificationPoint"), "{why}");
    // Naming the permitted points for the DECLARED class, not all five.
    assert!(why.contains("PAG"), "{why}");
    assert!(
        !why.contains("PAA"),
        "a hard constraint must not be told PAA is available: {why}"
    );
}

#[test]
fn an_escalation_without_a_reversibility_window_is_rejected() {
    // SARC I4. This is the acceptance criterion for Q-SARC-PLACEMENT.
    let why = escalation("PAG", None, Some("deny"))
        .violation("p")
        .unwrap();
    assert!(why.contains("reversibilityWindowSeconds"), "{why}");
    assert!(
        why.contains("deferred autonomy"),
        "the reason should name what an unbounded escalation actually is: {why}"
    );
}

#[test]
fn an_escalation_without_on_timeout_is_rejected() {
    let why = escalation("PAG", Some("600"), None).violation("p").unwrap();
    assert!(why.contains("onTimeout"), "{why}");
    assert!(
        why.contains("no-op"),
        "the reason should name the failure mode — silent pass under load: {why}"
    );
}

#[test]
fn a_non_action_policy_is_exempt() {
    // A committed-tier policy carries a SPARQL claim and no structural
    // placement. Requiring one would reject every legitimate one of them.
    let transition = Placement {
        boundary: Some("transition".to_string()),
        ..Placement::default()
    };
    assert!(transition.violation("p").is_none());
    assert!(
        Placement::default().violation("p").is_none(),
        "a policy with no boundary at all has no dispatch placement to get wrong"
    );
}

#[test]
fn an_unknown_class_is_reported_rather_than_panicking() {
    // The shape's sh:in should have caught this; if it is reached anyway the
    // check must say so, not index past the end of a table.
    let why = action(Some("catastrophic"), Some("PAG"))
        .violation("p")
        .unwrap();
    assert!(why.contains("unknown"), "{why}");
    assert_eq!(permitted_list("catastrophic"), "(unknown class)");
}

#[test]
fn every_rejection_names_the_policy_and_offers_a_way_forward() {
    // The recoverability discipline: a refusal that does not name the fix is a
    // refusal an operator cannot act on.
    let cases = [
        action(Some("hard"), Some("PAA")),
        action(Some("soft"), Some("PAG")),
        action(None, Some("PAG")),
        action(Some("hard"), None),
        escalation("PAG", None, Some("deny")),
        escalation("PAG", Some("600"), None),
    ];
    for case in cases {
        let why = case.violation("aegis:the_policy").unwrap();
        assert!(
            why.contains("aegis:the_policy"),
            "every reason names its subject: {why}"
        );
        assert!(
            why.contains("Declare") || why.contains("Permitted") || why.contains("declare"),
            "every reason names the remedy: {why}"
        );
    }
}

// ── Liveness: the check fires through the REAL write path ────────────────────
//
// The rules above are pure functions, and a pure function that nothing calls is
// the failure mode this repo keeps finding. These tests go through
// `Store::transact`, so they fail if the wiring in `stage_and_guard` is removed
// even while every rule test above still passes. Two-sided: a control that the
// same input passes with the flag off, and a negative that must not fire.

use crate::error::Error;
use crate::store::Store;
use crate::types::Op;

const TS: &str = "2026-01-01T00:00:00Z";

/// Stage an `aegis:Policy` with the given SARC fields, as one transaction.
fn define(store: &mut Store, iri: &str, fields: &[(&str, &str)]) -> crate::error::Result<i64> {
    let policy_class = format!("{DEFAULT_BASE_NS}Policy");
    let mut datums = vec![Datum {
        entity: store.intern(iri).unwrap(),
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern(&policy_class).unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    for (k, v) in fields {
        datums.push(Datum {
            entity: store.intern(iri).unwrap(),
            attribute: store.intern(&format!("{DEFAULT_BASE_NS}{k}")).unwrap(),
            value: Value::Str((*v).to_string()),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    store.transact(&datums, TS, None, None)
}

/// A hard constraint declared at the Post-Action Auditor — the malformation.
const HARD_AT_PAA: &[(&str, &str)] = &[
    ("targets", "CodeModule"),
    ("claim", "ASK { ?s ?p ?o }"),
    ("boundary", "action"),
    ("effect", "deny"),
    ("constraintClass", "hard"),
    ("verificationPoint", "PAA"),
];

#[test]
fn a_malformed_policy_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;

    let err = define(&mut store, "http://ex/bad", HARD_AT_PAA);
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "a hard constraint at the PAA must be refused at write, got {err:?}"
    );
    // The rollback contract: a refused definition leaves nothing behind.
    assert!(
        !matches!(
            crate::sparql::query(&store, "ASK { <http://ex/bad> ?p ?o }").unwrap(),
            crate::sparql::QueryResult::Ask(true)
        ),
        "a refused policy definition must leave the store byte-identical"
    );
}

#[test]
fn the_control_the_same_write_lands_with_the_flag_off() {
    // Without this the test above proves only that SOMETHING rejected the
    // write. Same store, same datums, flag off => accepted. That is what makes
    // the rejection attributable to this check.
    let mut store = Store::open_in_memory().unwrap();
    assert!(
        !store.governance_config_mut().validate_placement,
        "the flag must default to off"
    );
    define(&mut store, "http://ex/bad", HARD_AT_PAA)
        .expect("with validation off the same definition lands");
}

#[test]
fn a_well_placed_policy_lands_with_the_flag_on() {
    // The GREEN case. A check that rejects everything is not a check.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    define(
        &mut store,
        "http://ex/good",
        &[
            ("targets", "CodeModule"),
            ("claim", "ASK { ?s ?p ?o }"),
            ("boundary", "action"),
            ("effect", "deny"),
            ("constraintClass", "hard"),
            ("verificationPoint", "PAG"),
        ],
    )
    .expect("hard at PAG is Table 3-conformant and must land");
}

#[test]
fn a_non_policy_write_is_not_touched() {
    // The pre-filter: the check must cost nothing on ordinary traffic, and must
    // certainly not reject it.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let datums = vec![Datum {
        entity: store.intern("http://ex/thing").unwrap(),
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern("http://ex/Widget").unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    store
        .transact(&datums, TS, None, None)
        .expect("an ordinary write must pass untouched");
}

#[test]
fn amending_a_policy_revalidates_it() {
    // A definition is not only its first write. Landing a well-placed policy and
    // then moving it to an incompatible point must be refused too, or the check
    // guards creation and nothing else.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    define(
        &mut store,
        "http://ex/p",
        &[
            ("targets", "CodeModule"),
            ("claim", "ASK { ?s ?p ?o }"),
            ("boundary", "action"),
            ("constraintClass", "soft"),
            ("verificationPoint", "PAA"),
        ],
    )
    .expect("soft at PAA lands");

    // Now assert a second, incompatible point on the same policy.
    let amend = vec![Datum {
        entity: store.intern("http://ex/p").unwrap(),
        attribute: store
            .intern(&format!("{DEFAULT_BASE_NS}constraintClass"))
            .unwrap(),
        value: Value::Str("hard".to_string()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    match store.transact(&amend, TS, None, None) {
        Err(Error::PolicyDenied(why)) => {
            // Asserting `hard` does NOT retract `soft` — both facts are active,
            // and the policy now has two classes. Refusing on the ambiguity is
            // the honest answer; silently resolving to either one would let a
            // re-class land while reporting the other placement as conformant.
            assert!(
                why.contains("distinct values") && why.contains("constraintClass"),
                "the refusal should name the ambiguity, got: {why}"
            );
            assert!(
                why.contains("hard") && why.contains("soft"),
                "and both competing values: {why}"
            );
        }
        other => panic!("re-classing a PAA policy as hard must be refused, got {other:?}"),
    }
}

#[test]
fn a_clean_re_placement_retracting_the_old_value_lands() {
    // The recoverability half. If ambiguity is refused, there has to be a way
    // to legitimately move a policy — otherwise the check strands the operator,
    // which is worse than the thing it prevents.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    define(
        &mut store,
        "http://ex/p",
        &[
            ("targets", "CodeModule"),
            ("claim", "ASK { ?s ?p ?o }"),
            ("boundary", "action"),
            ("constraintClass", "soft"),
            ("verificationPoint", "PAA"),
        ],
    )
    .expect("soft at PAA lands");

    let field = |k: &str| store.intern(&format!("{DEFAULT_BASE_NS}{k}")).unwrap();
    let entity = store.intern("http://ex/p").unwrap();
    let datum = |a: i64, v: &str, op: Op| Datum {
        entity,
        attribute: a,
        value: Value::Str(v.to_string()),
        valid_from: TS.to_string(),
        valid_to: None,
        op,
    };
    // Retract both stale values and assert the new pair, in ONE transaction.
    let move_to_pag = vec![
        datum(field("constraintClass"), "soft", Op::Retract),
        datum(field("verificationPoint"), "PAA", Op::Retract),
        datum(field("constraintClass"), "hard", Op::Assert),
        datum(field("verificationPoint"), "PAG", Op::Assert),
    ];
    store
        .transact(&move_to_pag, TS, None, None)
        .expect("a re-placement that retracts the old values must land");
}
