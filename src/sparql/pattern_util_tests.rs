//! A node bound by an earlier pattern must reach the next pattern's SQL as a
//! named term (aegis-o3l46b).
//!
//! The first engine returned `None` for a bound subject or predicate ("We'd
//! need store to resolve, skip for now"), so every pattern after the first in
//! a nested-loop join scanned ALL facts with its predicate and filtered in
//! Rust. On a 2.6M-fact store one bound subject cost ~0.85 s against ~9 ms for
//! the named-IRI form, and a 22-row join stalled the store for seconds.

use oxrdfio::RdfFormat;
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, Variable};

use super::pattern_util::{Bindings, resolve_predicate_pattern, resolve_subject_pattern};
use super::query;
use crate::rdf::ingest_rdf;
use crate::store::Store;
use crate::types::Value;

const EX: &str = "http://example.org/";

#[test]
fn a_bound_subject_or_predicate_resolves_to_its_iri() {
    let store = Store::open_in_memory().unwrap();
    let alice = store.intern(&format!("{EX}alice")).unwrap();
    let knows = store.intern(&format!("{EX}knows")).unwrap();
    let mut bindings = Bindings::new();
    bindings.insert("s".into(), Value::Ref(alice));
    bindings.insert("p".into(), Value::Ref(knows));

    let s = TermPattern::Variable(Variable::new_unchecked("s"));
    assert_eq!(
        resolve_subject_pattern(&store, &s, &bindings).unwrap(),
        Some(format!("{EX}alice"))
    );
    let p = NamedNodePattern::Variable(Variable::new_unchecked("p"));
    assert_eq!(
        resolve_predicate_pattern(&store, &p, &bindings).unwrap(),
        Some(format!("{EX}knows"))
    );

    // Control: a named term resolves as before.
    let named = TermPattern::NamedNode(NamedNode::new_unchecked(format!("{EX}bob")));
    assert_eq!(
        resolve_subject_pattern(&store, &named, &bindings).unwrap(),
        Some(format!("{EX}bob"))
    );
}

#[test]
fn unbound_sentinel_and_literal_bindings_stay_unresolved() {
    let store = Store::open_in_memory().unwrap();
    let mut bindings = Bindings::new();
    bindings.insert("never".into(), Value::Ref(-1));
    bindings.insert("lit".into(), Value::Str("x".into()));
    for name in ["unbound", "never", "lit"] {
        let v = TermPattern::Variable(Variable::new_unchecked(name));
        assert_eq!(
            resolve_subject_pattern(&store, &v, &bindings).unwrap(),
            None,
            "{name}"
        );
    }
}

#[test]
fn a_bound_join_returns_exactly_the_bound_subjects_rows() {
    // The second predicate has many OTHER subjects. With the bound subject
    // pushed into SQL, only alice's rows may come back, and all of them must.
    let mut store = Store::open_in_memory().unwrap();
    let mut turtle = String::from("@prefix ex: <http://example.org/> .\n");
    turtle.push_str("ex:alice ex:path \"docs/a.md\" ; ex:label \"A1\" , \"A2\" .\n");
    for i in 0..200 {
        turtle.push_str(&format!("ex:other{i} ex:label \"O{i}\" .\n"));
    }
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?e ?l WHERE { ?e ex:path "docs/a.md" ; ex:label ?l } ORDER BY ?l"#,
    )
    .unwrap();
    let labels: Vec<_> = result
        .rows()
        .iter()
        .map(|r| r.get("l").cloned().unwrap())
        .collect();
    assert_eq!(
        labels,
        vec![Value::Str("A1".into()), Value::Str("A2".into())]
    );
}
