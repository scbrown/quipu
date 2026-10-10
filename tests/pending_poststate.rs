//! Distinguish transactional policy post-state from monotone SHACL repair.
use quipu::error::Error;
use quipu::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use quipu::sparql::{self, QueryResult};
use quipu::types::{Op, Value};
use quipu::{Datum, Store};

const TS: &str = "2026-10-09T00:00:00Z";
const NS: &str = "https://example.org/poststate/";

fn datum(store: &Store, subject: &str, predicate: &str, value: Value) -> Datum {
    Datum {
        entity: store.intern(subject).unwrap(),
        attribute: store.intern(predicate).unwrap(),
        value,
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn reference(store: &Store, iri: &str) -> Value {
    Value::Ref(store.intern(iri).unwrap())
}

fn ask(store: &Store, query: &str) -> bool {
    matches!(sparql::query(store, query).unwrap(), QueryResult::Ask(true))
}

#[test]
fn policy_sees_linked_evidence_in_the_same_transaction_but_not_a_future_one() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    let policy = format!("{NS}policy");
    let parent_type = format!("{NS}Parent");
    let claim = format!("ASK {{ $target <{NS}child> ?c . ?c <{NS}name> ?n }}");
    let rules = vec![
        datum(
            &store,
            &policy,
            RDF_TYPE,
            reference(&store, &format!("{DEFAULT_BASE_NS}Policy")),
        ),
        datum(
            &store,
            &policy,
            &format!("{DEFAULT_BASE_NS}targets"),
            Value::Str(parent_type.clone()),
        ),
        datum(
            &store,
            &policy,
            &format!("{DEFAULT_BASE_NS}claim"),
            Value::Str(claim),
        ),
        datum(
            &store,
            &policy,
            &format!("{DEFAULT_BASE_NS}boundary"),
            Value::Str("action".into()),
        ),
        datum(
            &store,
            &policy,
            &format!("{DEFAULT_BASE_NS}effect"),
            Value::Str("deny".into()),
        ),
    ];
    store.transact(&rules, TS, None, None).unwrap();
    let parent = format!("{NS}parent");
    let child = format!("{NS}child");
    let proposed = vec![
        datum(&store, &parent, RDF_TYPE, reference(&store, &parent_type)),
        datum(
            &store,
            &parent,
            &format!("{NS}child"),
            reference(&store, &child),
        ),
    ];
    let result = store.transact(&proposed, TS, None, None);
    assert!(
        matches!(result, Err(Error::PolicyDenied(_))),
        "future evidence must not satisfy the current write: {result:?}"
    );
    assert!(
        !ask(&store, &format!("ASK {{ <{parent}> ?p ?o }}")),
        "refusal must roll back parent and link"
    );
    let evidence = datum(
        &store,
        &child,
        &format!("{NS}name"),
        Value::Str("positive control".into()),
    );
    let mut complete = proposed.clone();
    complete.push(evidence.clone());
    store
        .transact(&complete, TS, None, None)
        .expect("same transaction evidence is visible to the policy");
    assert!(ask(
        &store,
        &format!("ASK {{ <{parent}> <{NS}child> ?c . ?c <{NS}name> ?n }}")
    ));
    // A different parent can also use evidence already committed in the store.
    let second = format!("{NS}second");
    let committed_evidence = vec![
        datum(&store, &second, RDF_TYPE, reference(&store, &parent_type)),
        datum(
            &store,
            &second,
            &format!("{NS}child"),
            reference(&store, &child),
        ),
    ];
    store
        .transact(&committed_evidence, TS, None, None)
        .expect("committed linked evidence remains visible");
    assert!(ask(
        &store,
        &format!("ASK {{ <{second}> <{NS}child> ?c . ?c <{NS}name> ?n }}")
    ));
}

#[test]
#[cfg(feature = "shacl")]
fn contextual_shacl_repairs_class_without_promoting_new_context_violations() {
    let mut store = Store::open_in_memory().unwrap();
    let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <https://example.org/poststate/> .
ex:ParentShape a sh:NodeShape; sh:targetClass ex:Parent;
 sh:property [sh:path ex:child; sh:minCount 1; sh:class ex:Child].
ex:ChildShape a sh:NodeShape; sh:targetClass ex:Child;
 sh:property [sh:path ex:name; sh:minCount 1].
"#;
    let child = "@prefix ex: <https://example.org/poststate/> . ex:c a ex:Child .";
    let parent = "@prefix ex: <https://example.org/poststate/> . ex:p a ex:Parent; ex:child ex:c .";
    // Pre-existing incomplete context: the child was admitted before this shape.
    quipu::rdf::ingest_rdf(
        &mut store,
        child.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        None,
        Some("poststate-control"),
    )
    .unwrap();
    assert!(
        !quipu::validate_shapes(shapes, parent).unwrap().conforms,
        "payload-alone class negative control"
    );
    let incomplete = format!("{parent}\n{child}");
    assert!(
        !quipu::validate_shapes(shapes, &incomplete)
            .unwrap()
            .conforms,
        "same-payload child minCount negative control"
    );
    let complete = format!(
        "{incomplete}\n<https://example.org/poststate/c> <https://example.org/poststate/name> \"positive\" ."
    );
    assert!(
        quipu::validate_shapes(shapes, &complete).unwrap().conforms,
        "positive control must actually satisfy both shapes"
    );
    assert!(
        quipu::shacl_context::validate_with_store_context(&store, shapes, parent)
            .unwrap()
            .conforms,
        "context repairs class; new child minCount violation is intentionally outside the payload ceiling"
    );
}
