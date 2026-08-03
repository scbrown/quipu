//! Σ-loading tests. Size-exempt (`*_tests.rs`).

use super::*;
use crate::namespace::RDF_TYPE;
use crate::store::Datum;
use crate::types::Op;

const TS: &str = "2026-01-01T00:00:00Z";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// Write a policy with the given `aegis:` fields.
fn policy(store: &mut Store, iri: &str, fields: &[(&str, &str)]) {
    let entity = store.intern(iri).unwrap();
    let mut datums = vec![Datum {
        entity,
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Policy")).unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    for (name, value) in fields {
        let attribute = if *name == "label" {
            store.intern(RDFS_LABEL).unwrap()
        } else {
            store.intern(&format!("{DEFAULT_BASE_NS}{name}")).unwrap()
        };
        datums.push(Datum {
            entity,
            attribute,
            value: Value::Str((*value).to_string()),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    store.transact(&datums, TS, None, None).unwrap();
}

#[test]
fn a_policy_is_keyed_by_its_label() {
    let mut store = Store::open_in_memory().unwrap();
    policy(
        &mut store,
        "http://ex/policy/p1",
        &[
            ("label", "no-ticket-in-comment"),
            ("boundary", "action"),
            ("constraintClass", "hard"),
            ("verificationPoint", "PAG"),
            ("effect", "deny"),
        ],
    );
    let spec = load(&store).unwrap();
    let c = spec.get("no-ticket-in-comment").expect("keyed by label");
    assert_eq!(c.class.as_deref(), Some("hard"));
    assert_eq!(c.point.as_deref(), Some("PAG"));
    assert_eq!(c.effect.as_deref(), Some("deny"));
    assert_eq!(c.iri, "http://ex/policy/p1");
}

#[test]
fn an_unlabelled_policy_falls_back_to_its_local_name() {
    // The trace cites whatever hank named the rule, and hank's projection falls
    // back the same way. A checker that dropped unlabelled policies would report
    // them as never exercised no matter what the trace said.
    let mut store = Store::open_in_memory().unwrap();
    policy(
        &mut store,
        "http://ex/policy/lonely",
        &[("boundary", "action"), ("constraintClass", "soft")],
    );
    let spec = load(&store).unwrap();
    assert!(spec.contains_key("lonely"), "{:?}", spec.keys());
}

#[test]
fn a_transition_boundary_policy_is_not_in_scope() {
    // It governs a state change rather than a dispatched action, so no trace
    // record could have traversed an enforcement point for it — and reporting
    // it as never exercised would be reporting an absence that is correct.
    let mut store = Store::open_in_memory().unwrap();
    policy(
        &mut store,
        "http://ex/policy/t",
        &[("label", "some-transition"), ("boundary", "transition")],
    );
    assert!(load(&store).unwrap().is_empty());
}

#[test]
fn a_policy_appears_once_however_many_optionals_bind() {
    // The OPTIONAL cross-product would otherwise turn one policy into several
    // constraints, and the vacuity count would be wrong by a multiple.
    let mut store = Store::open_in_memory().unwrap();
    policy(
        &mut store,
        "http://ex/policy/p",
        &[
            ("label", "one"),
            ("boundary", "action"),
            ("constraintClass", "hard"),
            ("verificationPoint", "PAG"),
            ("effect", "deny"),
        ],
    );
    let spec = load(&store).unwrap();
    assert_eq!(spec.len(), 1, "{:?}", spec.keys());
}

#[test]
fn an_empty_store_yields_an_empty_spec_not_an_error() {
    let store = Store::open_in_memory().unwrap();
    assert!(load(&store).unwrap().is_empty());
}
