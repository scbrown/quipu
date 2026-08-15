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
        ..Placement::default()
    }
}

fn escalation(point: &str, window: Option<&str>, timeout: Option<&str>) -> Placement {
    Placement {
        boundary: Some("action".to_string()),
        class: Some("escalation".to_string()),
        point: Some(point.to_string()),
        reversibility_window: window.map(str::to_string),
        on_timeout: timeout.map(str::to_string),
        ..Placement::default()
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

// ── Value checks for the safety-critical enums ───────────────────────────────
//
// These live here, not only in the shape, because SHACL runs only under
// `shacl.validate_on_write` (default FALSE) and validates episode ingest rather
// than every transact. The shape tests in governance_tests.rs prove the SHAPE
// rejects these; these prove the WRITE PATH does, which is the claim that
// matters.

fn with_timeout(timeout: &str) -> Placement {
    Placement {
        on_timeout: Some(timeout.to_string()),
        ..Placement::default()
    }
}

fn with_layer(layer: &str) -> Placement {
    Placement {
        hosted_at_layer: Some(layer.to_string()),
        ..Placement::default()
    }
}

#[test]
fn on_timeout_allow_is_refused_by_the_check_not_only_by_the_shape() {
    let why = with_timeout("allow").violation("p").unwrap();
    assert!(why.contains("onTimeout"), "{why}");
    assert!(
        why.contains("no-op"),
        "the reason names the failure: an escalation that passes under load: {why}"
    );
    assert!(with_timeout("deny").violation("p").is_none());
}

#[test]
fn the_prompt_layer_is_refused_by_the_check_too() {
    let why = with_layer("prompt").violation("p").unwrap();
    assert!(why.contains("hostedAtLayer"), "{why}");
    assert!(why.contains("I6"), "the reason cites the invariant: {why}");
    for ok in ["orchestration", "tool", "policy"] {
        assert!(
            with_layer(ok).violation("p").is_none(),
            "{ok} is a permitted layer"
        );
    }
}

#[test]
fn the_value_checks_apply_outside_the_action_boundary_too() {
    // Both are checked BEFORE the boundary exemption, deliberately: a
    // transition-boundary escalation with onTimeout "allow" fails just as
    // silently as an action-boundary one, and the exemption exists only for
    // PLACEMENT, which a non-action policy genuinely does not have.
    let transition = Placement {
        boundary: Some("transition".to_string()),
        on_timeout: Some("allow".to_string()),
        ..Placement::default()
    };
    assert!(
        transition.violation("p").is_some(),
        "a bad onTimeout is not excused by the boundary"
    );
}

#[test]
fn a_bad_value_is_refused_at_the_write_path() {
    // Liveness for the value checks, through the real transact.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let err = define(
        &mut store,
        "http://ex/p",
        &[
            ("targets", "CodeModule"),
            ("claim", "ASK { ?s ?p ?o }"),
            ("boundary", "action"),
            ("constraintClass", "escalation"),
            ("verificationPoint", "PAG"),
            ("reversibilityWindowSeconds", "600"),
            ("onTimeout", "allow"),
        ],
    );
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "onTimeout \"allow\" must be refused on the write path, got {err:?}"
    );
}

#[test]
fn a_forbidden_hosting_layer_is_refused_through_the_read_path() {
    // quipu-sio: the placement SELECT once omitted ?layer from its projection,
    // so `read_placements` never saw hostedAtLayer and the prompt-layer refusal
    // was dead code — reachable only by constructing a Placement directly. This
    // goes through the real transact, so it fails if the projection regresses.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let err = define(
        &mut store,
        "http://ex/p",
        &[
            ("targets", "CodeModule"),
            ("claim", "ASK { ?s ?p ?o }"),
            ("boundary", "action"),
            ("constraintClass", "hard"),
            ("verificationPoint", "PAG"),
            ("hostedAtLayer", "prompt"),
        ],
    );
    let Err(Error::PolicyDenied(why)) = err else {
        panic!("hostedAtLayer \"prompt\" must be refused through read_placements, got {err:?}");
    };
    assert!(
        why.contains("hostedAtLayer") && why.contains("prompt"),
        "the refusal names the field and value: {why}"
    );
}

// ── Class↔effect conformance (quipu-tw9) ─────────────────────────────────────
//
// The gate escalates on the EFFECT while the window/onTimeout rules key on the
// CLASS. A policy with effect "escalate" but class "hard" escalated at runtime
// with no declared window, got a zero window, and became an instant permanent
// denial. The mismatch is refused at definition time instead.

#[test]
fn an_escalating_effect_with_a_non_escalation_class_is_rejected() {
    for effect in ["escalate", "require-approval"] {
        for class in ["hard", "soft"] {
            let point = if class == "hard" { "PAG" } else { "PAA" };
            let mut p = action(Some(class), Some(point));
            p.effect = Some(effect.to_string());
            let why = p.violation("p").unwrap();
            assert!(
                why.contains(effect) && why.contains(class),
                "the refusal names the mismatched pair: {why}"
            );
            assert!(
                why.contains("escalation"),
                "and the remedy — the class that carries the bounds: {why}"
            );
        }
    }
    // The conformant pairing still lands: escalating effect, escalation class.
    let mut ok = escalation("PAG", Some("600"), Some("deny"));
    ok.effect = Some("escalate".to_string());
    assert!(ok.violation("p").is_none());
    // And a non-escalating effect on a hard class is untouched.
    let mut deny = action(Some("hard"), Some("PAG"));
    deny.effect = Some("deny".to_string());
    assert!(deny.violation("p").is_none());
}

#[test]
fn an_escalating_effect_without_the_escalation_class_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let err = define(
        &mut store,
        "http://ex/p",
        &[
            ("targets", "CodeModule"),
            ("claim", "ASK { ?s ?p ?o }"),
            ("boundary", "action"),
            ("effect", "escalate"),
            ("constraintClass", "hard"),
            ("verificationPoint", "PAG"),
        ],
    );
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "an escalate-effect hard-class policy must be refused at definition time, got {err:?}"
    );
}

// ── Entity-grounded and inexact-tier vocabulary (quipu-3aj) ──────────────────
//
// Design: docs/design/semantic-grounded-edit-policies.md. Two rule families:
// a grounded match type must NAME its id set (a query-less grounded rule
// evaluates against emptiness, where nothing grounds and a deny-on-grounding
// policy allows everything, silently), and a classifier-tier predicate must
// neither hard-deny at the gate nor claim an exact operating point.

fn grounded(match_type: &str, has_query: bool) -> Grounding {
    Grounding {
        match_types: vec![match_type.to_string()],
        has_grounding_query: has_query,
    }
}

#[test]
fn a_grounded_match_type_without_a_grounding_query_is_rejected() {
    for mt in ["must-ground", "must-not-ground"] {
        let why = grounded(mt, false).violation("aegis:pr").unwrap();
        assert!(why.contains("aegis:pr"), "names the predicate: {why}");
        assert!(why.contains(mt), "names the grounded match type: {why}");
        assert!(
            why.contains("groundingQuery"),
            "names the missing field: {why}"
        );
        assert!(
            why.contains("allows everything"),
            "names the SILENT failure mode — empty set, nothing grounds: {why}"
        );
        assert!(why.contains("Declare"), "names the remedy: {why}");
    }
}

#[test]
fn a_grounded_predicate_with_its_query_conforms() {
    // The green half: the rule demands the query, not the impossibility of
    // grounded match types.
    for mt in ["must-ground", "must-not-ground"] {
        assert!(grounded(mt, true).violation("p").is_none());
    }
}

#[test]
fn a_lexical_match_type_needs_no_grounding_query() {
    // The v1 regex predicates must keep landing untouched, and a predicate
    // with no matchType at all (a SPARQL-ASK one) has no direction to ground.
    for mt in ["must-match", "must-not-match", "must-exist"] {
        assert!(grounded(mt, false).violation("p").is_none());
    }
    assert!(Grounding::default().violation("p").is_none());
}

/// A well-calibrated inexact-tier policy: advisory at the PAA, operating
/// point present, tolerant FP number. The conformant case the rejection
/// tests perturb.
fn inexact(tier: &str) -> Placement {
    Placement {
        boundary: Some("action".to_string()),
        class: Some("soft".to_string()),
        point: Some("PAA".to_string()),
        effect: Some("warn".to_string()),
        predicate_tiers: vec![tier.to_string()],
        has_operating_point: true,
        fp_tolerances: vec![0.05],
        ..Placement::default()
    }
}

#[test]
fn a_calibrated_inexact_policy_at_the_paa_conforms() {
    for tier in ["embedding", "model"] {
        assert!(
            inexact(tier).violation("p").is_none(),
            "advisory {tier}-tier with a nonzero operating point must conform"
        );
    }
}

#[test]
fn an_inexact_tier_predicate_must_not_hard_deny_at_the_pag() {
    for tier in ["embedding", "model"] {
        let mut p = inexact(tier);
        p.class = Some("hard".to_string());
        p.point = Some("PAG".to_string());
        p.effect = Some("deny".to_string());
        let why = p.violation("aegis:p").unwrap();
        assert!(why.contains("aegis:p"), "names the policy: {why}");
        assert!(why.contains(tier), "names the tier: {why}");
        assert!(
            why.contains("PAA") && why.contains("escalat"),
            "names both permitted routes — PAA placement or escalation: {why}"
        );
    }
    // The control: EXACT membership stays hard-capable. tree-sitter+graph is
    // deterministic — the whole point of the grounded tier is that it may
    // deny at the gate where similarity may not.
    let mut exact = action(Some("hard"), Some("PAG"));
    exact.effect = Some("deny".to_string());
    exact.predicate_tiers = vec!["tree-sitter+graph".to_string()];
    exact.has_operating_point = true;
    exact.fp_tolerances = vec![0.0];
    assert!(
        exact.violation("p").is_none(),
        "exact grounded membership may hard-deny at the PAG"
    );
}

#[test]
fn an_inexact_tier_policy_without_an_operating_point_is_rejected() {
    let mut p = inexact("embedding");
    p.has_operating_point = false;
    p.fp_tolerances = Vec::new();
    let why = p.violation("aegis:p").unwrap();
    assert!(why.contains("aegis:p"), "{why}");
    assert!(why.contains("operatingPoint"), "{why}");
    assert!(
        why.contains("NONZERO"),
        "the remedy states the calibration must be tolerant, not merely present: {why}"
    );
}

#[test]
fn an_inexact_tier_policy_claiming_exactness_is_rejected() {
    // falsePositiveTolerance 0.0 on a classifier asserts an error rate it
    // cannot have — the exact tiers' claim, worn by an approximate predicate.
    let mut p = inexact("model");
    p.fp_tolerances = vec![0.0];
    let why = p.violation("aegis:p").unwrap();
    assert!(why.contains("falsePositiveTolerance"), "{why}");
    assert!(why.contains("nonzero"), "names the remedy: {why}");

    // And an operating point that simply omits the FP number is refused too:
    // an undeclared tolerance is not a tolerant one.
    let mut p = inexact("embedding");
    p.fp_tolerances = Vec::new();
    let why = p.violation("aegis:p").unwrap();
    assert!(why.contains("falsePositiveTolerance"), "{why}");
}

#[test]
fn the_inexact_rules_apply_outside_the_action_boundary_too() {
    // Like the onTimeout/hostedAtLayer value checks, these run BEFORE the
    // boundary exemption: an uncalibrated classifier is dishonest wherever
    // the policy binds, not only at a dispatch seam.
    let p = Placement {
        boundary: Some("transition".to_string()),
        predicate_tiers: vec!["embedding".to_string()],
        ..Placement::default()
    };
    assert!(
        p.violation("p").is_some(),
        "a transition-boundary embedding policy still needs its operating point"
    );
}

// ── Liveness for the grounded and inexact rules, through the write path ──────

/// Stage an `aegis:Predicate` with the given string fields, as one transaction.
fn define_predicate(
    store: &mut Store,
    iri: &str,
    fields: &[(&str, &str)],
) -> crate::error::Result<i64> {
    let predicate_class = format!("{DEFAULT_BASE_NS}Predicate");
    let mut datums = vec![Datum {
        entity: store.intern(iri).unwrap(),
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern(&predicate_class).unwrap()),
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

#[test]
fn a_grounded_predicate_without_a_query_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let err = define_predicate(
        &mut store,
        "http://ex/pr",
        &[
            ("name", "no-ticket-in-comment"),
            ("evidenceSource", "graph:work-item-id-set"),
            ("candidateSource", "token"),
            ("matchType", "must-not-ground"),
        ],
    );
    let Err(Error::PolicyDenied(why)) = err else {
        panic!("a query-less grounded predicate must be refused at write, got {err:?}");
    };
    assert!(
        why.contains("groundingQuery") && why.contains("http://ex/pr"),
        "the refusal names the field and the predicate: {why}"
    );
}

#[test]
fn a_grounded_predicate_with_its_query_lands() {
    // The green half of the write-path pair, and the shape of the shipped
    // exemplar (aegis:pred_no_ticket_in_comment_v2).
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    define_predicate(
        &mut store,
        "http://ex/pr",
        &[
            ("name", "no-ticket-in-comment"),
            ("evidenceSource", "graph:work-item-id-set"),
            ("candidateSource", "token"),
            (
                "groundingQuery",
                "SELECT ?id WHERE { ?w a aegis:WorkItem ; aegis:identifier ?id }",
            ),
            ("matchType", "must-not-ground"),
            ("tier", "tree-sitter+graph"),
        ],
    )
    .expect("a grounded predicate naming its id set must land");
}

/// Stage predicate + operating point + composing policy in ONE transaction,
/// so the placement read sees the whole composition in the pending state.
fn define_inexact_policy(
    store: &mut Store,
    tier: &str,
    effect: &str,
    class: &str,
    point: &str,
    fp_tolerance: Option<f64>,
) -> crate::error::Result<i64> {
    let mut datums = Vec::new();
    let ts = |k: &str, store: &mut Store| store.intern(&format!("{DEFAULT_BASE_NS}{k}")).unwrap();

    let pred = store.intern("http://ex/pred").unwrap();
    let pred_class = format!("{DEFAULT_BASE_NS}Predicate");
    let pred_class_id = store.intern(&pred_class).unwrap();
    let rdf_type = store.intern(RDF_TYPE).unwrap();
    let mk = |entity: i64, attribute: i64, value: Value| Datum {
        entity,
        attribute,
        value,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    datums.push(mk(pred, rdf_type, Value::Ref(pred_class_id)));
    let tier_attr = ts("tier", store);
    datums.push(mk(pred, tier_attr, Value::Str(tier.to_string())));

    let op = store.intern("http://ex/op").unwrap();
    if let Some(fp) = fp_tolerance {
        let fp_attr = ts("falsePositiveTolerance", store);
        datums.push(mk(op, fp_attr, Value::Float(fp)));
    }

    let policy = store.intern("http://ex/p").unwrap();
    let policy_class = format!("{DEFAULT_BASE_NS}Policy");
    let policy_class_id = store.intern(&policy_class).unwrap();
    datums.push(mk(policy, rdf_type, Value::Ref(policy_class_id)));
    for (k, v) in [
        ("targets", "CodeModule"),
        ("claim", "c"),
        ("boundary", "action"),
        ("effect", effect),
        ("constraintClass", class),
        ("verificationPoint", point),
    ] {
        let attr = ts(k, store);
        datums.push(mk(policy, attr, Value::Str(v.to_string())));
    }
    let pred_attr = ts("predicate", store);
    datums.push(mk(policy, pred_attr, Value::Ref(pred)));
    let op_attr = ts("operatingPoint", store);
    datums.push(mk(policy, op_attr, Value::Ref(op)));

    store.transact(&datums, TS, None, None)
}

#[test]
fn an_inexact_policy_hard_denying_at_the_gate_is_refused_at_the_write_path() {
    // Liveness through the predicate-tier join: this fails if read_placements
    // stops projecting ?ptier, the same way the ?layer projection once
    // regressed (quipu-sio).
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let err = define_inexact_policy(&mut store, "embedding", "deny", "hard", "PAG", Some(0.08));
    let Err(Error::PolicyDenied(why)) = err else {
        panic!("an embedding-tier hard PAG deny must be refused at write, got {err:?}");
    };
    assert!(
        why.contains("embedding") && why.contains("PAA"),
        "the refusal names the tier and the permitted seam: {why}"
    );
}

#[test]
fn an_inexact_policy_claiming_exactness_is_refused_at_the_write_path() {
    // Liveness through the operating-point join, with the tolerance written
    // as a Float — the direct-transact route for a decimal.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    let err = define_inexact_policy(&mut store, "model", "warn", "soft", "PAA", Some(0.0));
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "a model-tier policy claiming falsePositiveTolerance 0.0 must be refused, got {err:?}"
    );
}

#[test]
fn a_calibrated_inexact_policy_lands_at_the_paa() {
    // The GREEN case for the whole composition: soft, advisory, at the PAA,
    // with a tolerant operating point — the placement design B prescribes.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    define_inexact_policy(&mut store, "embedding", "warn", "soft", "PAA", Some(0.08))
        .expect("an advisory, calibrated embedding-tier policy must land");
}
