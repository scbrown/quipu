//! Tests for write-gate verdict persistence. Size-exempt (`*tests.rs`).

use super::*;
use crate::error::Error;
use crate::sparql::{self, QueryResult};

const TS: &str = "2026-01-01T00:00:00Z";
const DOC_TYPE: &str = "http://ex/Doc";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
/// A `deny` claim: the target must carry an `rdfs:label`.
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";

fn signed_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    let dir = tempfile::tempdir().unwrap();
    let identity =
        crate::signing::SigningIdentity::load(&dir.path().join("k.pk8"), "quipu").unwrap();
    store.set_signing_identity(std::sync::Arc::new(identity));
    store
}

fn datum(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn define_policy(store: &mut Store, iri: &str) {
    let policy_class = format!("{DEFAULT_BASE_NS}Policy");
    let class_ref = Value::Ref(store.intern(&policy_class).unwrap());
    let datums = vec![
        datum(store, iri, RDF_TYPE, class_ref),
        datum(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}targets"),
            Value::Str(DOC_TYPE.to_string()),
        ),
        datum(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}claim"),
            Value::Str(REQUIRE_LABEL.to_string()),
        ),
        datum(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}boundary"),
            Value::Str("action".to_string()),
        ),
        datum(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}effect"),
            Value::Str("deny".to_string()),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();
}

fn verdict_outcomes(store: &Store) -> Vec<String> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?o WHERE {{ ?v a a:Verdict ; a:outcome ?o }}"
    );
    match sparql::query(store, &q).unwrap() {
        QueryResult::Select { rows, .. } => rows
            .iter()
            .filter_map(|r| match r.get("o") {
                Some(Value::Str(s)) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_denied_write_still_records_its_verdict() {
    // THE case this design exists for. A denial rolls the savepoint back, and a
    // verdict written inside it would roll back too — losing the record of the
    // one decision that left no other evidence. An accepted write at least
    // leaves the facts it wrote; a refused one leaves nothing.
    let mut store = signed_store();
    define_policy(&mut store, "http://ex/P1");

    let bad = vec![datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        Value::Ref(store.intern(DOC_TYPE).unwrap()),
    )];
    assert!(matches!(
        store.transact(&bad, TS, None, None),
        Err(Error::PolicyDenied(_))
    ));

    // The write itself is gone...
    let gone = sparql::query(&store, "ASK { <http://ex/d1> ?p ?o }").unwrap();
    assert!(matches!(gone, QueryResult::Ask(false)));
    // ...and the verdict survived the rollback that removed it.
    assert_eq!(verdict_outcomes(&store), vec!["unsatisfied".to_string()]);
}

#[test]
fn an_accepted_write_records_a_satisfied_verdict() {
    // The other half. Without this the gate could only ever prove what it
    // stopped, and "did this policy ever pass anything?" would need the absence
    // of a denial as evidence — which is not evidence.
    let mut store = signed_store();
    define_policy(&mut store, "http://ex/P1");
    let good = vec![
        datum(
            &store,
            "http://ex/d2",
            RDF_TYPE,
            Value::Ref(store.intern(DOC_TYPE).unwrap()),
        ),
        datum(&store, "http://ex/d2", RDFS_LABEL, Value::Str("hi".into())),
    ];
    store.transact(&good, TS, None, None).unwrap();
    assert_eq!(verdict_outcomes(&store), vec!["satisfied".to_string()]);
}

#[test]
fn a_verdict_is_signed_and_bound_to_its_evidence() {
    let mut store = signed_store();
    define_policy(&mut store, "http://ex/P1");
    let good = vec![
        datum(
            &store,
            "http://ex/d2",
            RDF_TYPE,
            Value::Ref(store.intern(DOC_TYPE).unwrap()),
        ),
        datum(&store, "http://ex/d2", RDFS_LABEL, Value::Str("hi".into())),
    ];
    store.transact(&good, TS, None, None).unwrap();

    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?s ?h ?v ?t WHERE \
         {{ ?x a a:Verdict ; a:signature ?s ; a:evidenceHash ?h ; a:verifier ?v ; a:tier ?t }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query(&store, &q).unwrap() else {
        panic!("select");
    };
    assert_eq!(rows.len(), 1);
    let field = |k: &str| match rows[0].get(k) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{k} missing: {other:?}"),
    };
    assert!(!field("s").is_empty(), "signed");
    assert!(field("h").starts_with("sha256:"), "evidence-bound");
    assert_eq!(field("v"), "quipu");
    // The gate reads the COMMITTED graph, so `committed` is what it can claim.
    assert_eq!(field("t"), "committed");
}

#[test]
fn an_unsigned_store_records_no_verdict_rather_than_an_unsigned_one() {
    // A bare "satisfied" in the record is forgeable by anyone who can write a
    // fact. The whole point of a verdict is that it is an attestation.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store, "http://ex/P1");
    let good = vec![
        datum(
            &store,
            "http://ex/d2",
            RDF_TYPE,
            Value::Ref(store.intern(DOC_TYPE).unwrap()),
        ),
        datum(&store, "http://ex/d2", RDFS_LABEL, Value::Str("hi".into())),
    ];
    store.transact(&good, TS, None, None).unwrap();
    assert!(verdict_outcomes(&store).is_empty());
}

#[test]
fn recording_a_verdict_does_not_recurse() {
    // Writing a verdict is itself a write the gate would evaluate. Left alone
    // that is a loop: a policy over aegis:Verdict would deny the verdict
    // recording its own denial.
    let mut store = signed_store();
    define_policy(&mut store, "http://ex/P1");
    let bad = vec![datum(
        &store,
        "http://ex/d1",
        RDF_TYPE,
        Value::Ref(store.intern(DOC_TYPE).unwrap()),
    )];
    let _ = store.transact(&bad, TS, None, None);
    // Exactly one verdict: the denial's. Not one per nested re-evaluation, and
    // reaching here at all means no unbounded recursion.
    assert_eq!(verdict_outcomes(&store).len(), 1);
}

#[test]
fn the_evidence_hash_changes_with_the_outcome() {
    // Tamper- and replay-resistance, at the scope this verdict actually has:
    // the hash binds what the verdict ASSERTS, so the same policy on the same
    // target with a different outcome is a different, separately signed fact.
    let satisfied = PendingVerdict {
        predicate_id: "p".into(),
        target_ref: "t".into(),
        outcome: "satisfied".into(),
    };
    let unsatisfied = PendingVerdict {
        outcome: "unsatisfied".into(),
        ..satisfied.clone()
    };
    assert_ne!(satisfied.evidence_hash(), unsatisfied.evidence_hash());
    assert_eq!(satisfied.evidence_hash(), satisfied.evidence_hash());
}
