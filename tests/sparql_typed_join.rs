//! Typed joins must preserve metadata counts and subclass entailment (aegis-xr20kb).
use std::collections::BTreeMap;

use oxrdfio::RdfFormat;

use quipu::rdf::ingest_rdf;
use quipu::sparql::query;
use quipu::store::Store;
use quipu::types::Value;

fn fixture() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let mut turtle = String::from(
        "@prefix ex: <http://example.org/> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         ex:InteractiveSession rdfs:subClassOf ex:Session .\n\
         ex:missing a ex:Session .\n",
    );
    for i in 0..311 {
        let ty = if i == 310 {
            "InteractiveSession"
        } else {
            "Session"
        };
        turtle.push_str(&format!(
            "ex:session{i} a ex:{ty} ; ex:sourceKind \"observed\" ; ex:actor \"actor{}\" .\n",
            i % 19
        ));
    }
    // More predicate rows outside the type set must not leak into the join.
    for i in 0..1000 {
        turtle.push_str(&format!(
            "ex:noise{i} ex:sourceKind \"noise\" ; ex:actor \"noise\" .\n"
        ));
    }
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-10T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

fn joined_bag(store: &Store, body: &str) -> BTreeMap<(String, String), usize> {
    let result = query(store, &format!("SELECT ?s ?kind WHERE {{ {body} }}")).unwrap();
    let mut bag = BTreeMap::new();
    for row in result.rows() {
        let key = match (&row["s"], &row["kind"]) {
            (Value::Ref(s), Value::Str(kind)) => (store.resolve(*s).unwrap(), kind.clone()),
            other => panic!("unexpected joined terms: {other:?}"),
        };
        *bag.entry(key).or_default() += 1;
    }
    bag
}

#[test]
fn typed_join_orders_preserve_the_same_bag_and_inferred_subclass() {
    let store = fixture();
    let type_first = joined_bag(
        &store,
        "?s a <http://example.org/Session> ; <http://example.org/sourceKind> ?kind",
    );
    let predicate_first = joined_bag(
        &store,
        "?s <http://example.org/sourceKind> ?kind . ?s a <http://example.org/Session>",
    );
    assert_eq!(type_first, predicate_first);
    assert_eq!(type_first.len(), 311);
    assert!(type_first.values().all(|n| *n == 1));
    assert!(type_first.contains_key(&(
        "http://example.org/session310".to_owned(),
        "observed".to_owned(),
    )));
}

#[test]
fn typed_group_counts_only_sessions_with_metadata() {
    let store = fixture();
    let control = query(
        &store,
        "SELECT (COUNT(?s) AS ?n) WHERE { ?s a <http://example.org/Session> }",
    )
    .unwrap();
    assert_eq!(control.rows()[0]["n"], Value::Int(312));
    let kind = query(
        &store,
        "SELECT ?kind (COUNT(DISTINCT ?s) AS ?n) WHERE {
           ?s a <http://example.org/Session> ; <http://example.org/sourceKind> ?kind
         } GROUP BY ?kind",
    )
    .unwrap();
    assert_eq!(kind.rows().len(), 1);
    assert_eq!(kind.rows()[0]["kind"], Value::Str("observed".to_owned()));
    assert_eq!(kind.rows()[0]["n"], Value::Int(311));
    let actors = query(
        &store,
        "SELECT ?actor (COUNT(DISTINCT ?s) AS ?n) WHERE {
           ?s a <http://example.org/Session> ; <http://example.org/actor> ?actor
         } GROUP BY ?actor",
    )
    .unwrap();
    assert_eq!(actors.rows().len(), 19);
    let actual: BTreeMap<_, _> = actors
        .rows()
        .iter()
        .map(|row| match (&row["actor"], &row["n"]) {
            (Value::Str(actor), Value::Int(n)) => (actor.clone(), *n),
            other => panic!("unexpected actor group: {other:?}"),
        })
        .collect();
    let expected = (0..19)
        .map(|i| (format!("actor{i}"), if i < 7 { 17 } else { 16 }))
        .collect();
    assert_eq!(actual, expected);
}
