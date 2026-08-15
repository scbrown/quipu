//! Tests for the write-path policy guard. Kept in a separate file so the
//! production `guard.rs` stays under the file-size limit (this file is
//! exempt as a `*tests.rs`); wired via `#[path]` from `guard.rs`.

use crate::error::Error;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const TS: &str = "2026-01-01T00:00:00Z";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const DOC_TYPE: &str = "http://ex/Doc";
const NOTE_TYPE: &str = "http://ex/Note";
/// A `deny` claim: the target must carry an `rdfs:label`.
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";

fn assert_datum(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn type_ref(store: &Store, type_iri: &str) -> Value {
    Value::Ref(store.intern(type_iri).unwrap())
}

fn retract_datum(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Retract,
    }
}

/// Define an action-boundary policy with the given `effect`: entities of
/// `target_type` must satisfy `claim`.
fn define_policy_with_effect(
    store: &mut Store,
    policy_iri: &str,
    target_type: &str,
    claim: &str,
    effect: &str,
) {
    let policy_class = format!("{DEFAULT_BASE_NS}Policy");
    let datums = vec![
        assert_datum(store, policy_iri, RDF_TYPE, type_ref(store, &policy_class)),
        assert_datum(
            store,
            policy_iri,
            &format!("{DEFAULT_BASE_NS}targets"),
            Value::Str(target_type.to_string()),
        ),
        assert_datum(
            store,
            policy_iri,
            &format!("{DEFAULT_BASE_NS}claim"),
            Value::Str(claim.to_string()),
        ),
        assert_datum(
            store,
            policy_iri,
            &format!("{DEFAULT_BASE_NS}boundary"),
            Value::Str("action".to_string()),
        ),
        assert_datum(
            store,
            policy_iri,
            &format!("{DEFAULT_BASE_NS}effect"),
            Value::Str(effect.to_string()),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();
}

/// Define an action-boundary `deny` policy.
fn define_policy(store: &mut Store, policy_iri: &str, target_type: &str, claim: &str) {
    define_policy_with_effect(store, policy_iri, target_type, claim, "deny");
}

fn ask(store: &Store, q: &str) -> bool {
    matches!(sparql::query(store, q).unwrap(), QueryResult::Ask(true))
}

fn has_any_fact(store: &Store, subject: &str) -> bool {
    ask(store, &format!("ASK {{ <{subject}> ?p ?o }}"))
}

#[test]
fn deny_blocks_noncompliant_write() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

    // A Doc with no label violates the deny policy.
    let bad = vec![assert_datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        type_ref(&store, DOC_TYPE),
    )];
    let err = store.transact(&bad, TS, None, None);
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "expected policy denial, got {err:?}"
    );
    assert!(
        !has_any_fact(&store, "http://ex/d1"),
        "a denied write must leave the store byte-identical (no facts)"
    );

    // A Doc WITH a label, staged in one txn, satisfies the claim.
    let good = vec![
        assert_datum(&store, "http://ex/d2", RDF_TYPE, type_ref(&store, DOC_TYPE)),
        assert_datum(&store, "http://ex/d2", RDFS_LABEL, Value::Str("hi".into())),
    ];
    store
        .transact(&good, TS, None, None)
        .expect("a compliant write passes the gate");
    assert!(has_any_fact(&store, "http://ex/d2"));
}

#[test]
fn require_approval_blocks_fail_closed() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    // A require-approval policy must not pass silently — the write is
    // refused because this seam cannot grant the approval.
    define_policy_with_effect(
        &mut store,
        "http://ex/PA",
        DOC_TYPE,
        REQUIRE_LABEL,
        "require-approval",
    );

    let bad = vec![assert_datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        type_ref(&store, DOC_TYPE),
    )];
    let err = store.transact(&bad, TS, None, None);
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "require-approval must fail closed, got {err:?}"
    );
    assert!(!has_any_fact(&store, "http://ex/d1"));
}

#[test]
fn advisory_effects_do_not_block() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    // A `warn` policy is advisory — a non-compliant write still lands.
    define_policy_with_effect(&mut store, "http://ex/PW", DOC_TYPE, REQUIRE_LABEL, "warn");

    let bad = vec![assert_datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        type_ref(&store, DOC_TYPE),
    )];
    store
        .transact(&bad, TS, None, None)
        .expect("an advisory `warn` policy never blocks");
    assert!(has_any_fact(&store, "http://ex/d1"));
}

#[test]
fn retracting_a_required_fact_is_denied() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

    // A compliant Doc with a label.
    let good = vec![
        assert_datum(&store, "http://ex/d1", RDF_TYPE, type_ref(&store, DOC_TYPE)),
        assert_datum(&store, "http://ex/d1", RDFS_LABEL, Value::Str("hi".into())),
    ];
    store.transact(&good, TS, None, None).unwrap();

    // Retracting the label leaves the Doc non-compliant → denied, and the
    // label survives the rollback.
    let strip = vec![retract_datum(
        &store,
        "http://ex/d1",
        RDFS_LABEL,
        Value::Str("hi".into()),
    )];
    let err = store.transact(&strip, TS, None, None);
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "a retraction that violates a policy must be denied, got {err:?}"
    );
    assert!(
        ask(
            &store,
            "ASK { <http://ex/d1> <http://www.w3.org/2000/01/rdf-schema#label> ?l }"
        ),
        "the required fact must survive the rolled-back retraction"
    );
}

#[test]
fn unrelated_edit_of_a_compliant_entity_passes() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

    let good = vec![
        assert_datum(&store, "http://ex/d1", RDF_TYPE, type_ref(&store, DOC_TYPE)),
        assert_datum(&store, "http://ex/d1", RDFS_LABEL, Value::Str("hi".into())),
    ];
    store.transact(&good, TS, None, None).unwrap();

    // Editing an unrelated property leaves the label intact → still compliant.
    let edit = vec![assert_datum(
        &store,
        "http://ex/d1",
        "http://ex/color",
        Value::Str("red".into()),
    )];
    store
        .transact(&edit, TS, None, None)
        .expect("an edit that keeps the entity compliant is not blocked");
    assert!(ask(&store, "ASK { <http://ex/d1> <http://ex/color> ?c }"));
}

#[test]
fn enforcement_off_is_a_noop() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = false;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

    // The same non-compliant write succeeds when enforcement is disabled.
    let bad = vec![assert_datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        type_ref(&store, DOC_TYPE),
    )];
    store
        .transact(&bad, TS, None, None)
        .expect("no enforcement → write is not gated");
    assert!(has_any_fact(&store, "http://ex/d1"));
}

#[test]
fn ungoverned_type_is_not_checked() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

    // A Note has no policy targeting it — the pre-filter skips it entirely.
    let note = vec![assert_datum(
        &store,
        "http://ex/n1",
        RDF_TYPE,
        type_ref(&store, NOTE_TYPE),
    )];
    store
        .transact(&note, TS, None, None)
        .expect("a write touching no governed type is not gated");
    assert!(has_any_fact(&store, "http://ex/n1"));
}

#[test]
fn registry_invalidated_when_a_policy_is_added() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;

    // First enforced write builds an (empty) registry and caches it.
    let n1 = vec![assert_datum(
        &store,
        "http://ex/n1",
        RDF_TYPE,
        type_ref(&store, NOTE_TYPE),
    )];
    store.transact(&n1, TS, None, None).unwrap();

    // Add a policy governing Note — this must invalidate the cache.
    define_policy(&mut store, "http://ex/P2", NOTE_TYPE, REQUIRE_LABEL);

    // A new non-compliant Note is now denied (registry was rebuilt).
    let n2 = vec![assert_datum(
        &store,
        "http://ex/n2",
        RDF_TYPE,
        type_ref(&store, NOTE_TYPE),
    )];
    let err = store.transact(&n2, TS, None, None);
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "a newly-added policy must be honored on the next write, got {err:?}"
    );
}

#[test]
fn a_refusal_under_an_exemplar_carrying_policy_cites_the_exemplar() {
    // Policy-by-example provenance (docs/design/policy-by-example.md): a rule
    // drafted from a motivating case must explain its refusals BY that case, so
    // the refused party reads why the rule exists rather than only that it
    // fired. The citation rides the compiled registry — no per-denial lookup.
    let exemplar = "http://ex/verdict_the_motivating_edit";
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);
    let link = vec![assert_datum(
        &store,
        "http://ex/P1",
        &format!("{DEFAULT_BASE_NS}exemplar"),
        Value::Str(exemplar.into()),
    )];
    store.transact(&link, TS, None, None).unwrap();

    let bad = vec![assert_datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        type_ref(&store, DOC_TYPE),
    )];
    let Err(Error::PolicyDenied(msg)) = store.transact(&bad, TS, None, None) else {
        panic!("the non-compliant write must still be denied");
    };
    assert!(
        msg.contains(exemplar) && msg.contains("motivating case"),
        "the refusal must cite the exemplar by IRI: {msg}"
    );
}

#[test]
fn a_refusal_under_a_hand_authored_policy_cites_nothing() {
    // The paired green case: no exemplar, no citation — an empty suffix, never
    // a placeholder. Citing an absent motivating case would be forged
    // provenance in the message channel.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);
    let bad = vec![assert_datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        type_ref(&store, DOC_TYPE),
    )];
    let Err(Error::PolicyDenied(msg)) = store.transact(&bad, TS, None, None) else {
        panic!("the non-compliant write must be denied");
    };
    assert!(
        !msg.contains("motivating case") && !msg.contains("exemplar"),
        "no exemplar, no citation: {msg}"
    );
}

#[test]
fn adding_an_exemplar_invalidates_the_cached_registry() {
    // The citation must not stay invisible until an unrelated policy write
    // rebuilds the cache: aegis:exemplar is in the is_governance_write list,
    // and this is the observable consequence.
    let exemplar = "http://ex/verdict_late_link";
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

    // Prime the cache with a refusal that carries no citation yet.
    let bad =
        |store: &Store, s: &str| vec![assert_datum(store, s, RDF_TYPE, type_ref(store, DOC_TYPE))];
    let Err(Error::PolicyDenied(first)) =
        store.transact(&bad(&store, "http://ex/d1"), TS, None, None)
    else {
        panic!("first write must be denied");
    };
    assert!(!first.contains(exemplar));

    // Link the exemplar; the NEXT refusal must cite it.
    let link = vec![assert_datum(
        &store,
        "http://ex/P1",
        &format!("{DEFAULT_BASE_NS}exemplar"),
        Value::Str(exemplar.into()),
    )];
    store.transact(&link, TS, None, None).unwrap();
    let Err(Error::PolicyDenied(second)) =
        store.transact(&bad(&store, "http://ex/d2"), TS, None, None)
    else {
        panic!("second write must be denied");
    };
    assert!(
        second.contains(exemplar),
        "the citation must appear on the write AFTER the linkage landed: {second}"
    );
}
