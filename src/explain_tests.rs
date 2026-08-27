use serde_json::Value as Json;

use super::{DEFAULT_EXPLAIN_DEPTH, explain};
use crate::store::Store;

const TS: &str = "2026-01-01T00:00:00Z";

fn ingest(store: &mut Store, ttl: &str) {
    crate::rdf::ingest_rdf(
        store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )
    .unwrap();
}

/// A base fact explains as itself: found, no derivation node.
#[test]
fn base_fact_explains_as_asserted() {
    let mut store = Store::open_in_memory().unwrap();
    ingest(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:a ex:p ex:b .\n",
    );
    let out = explain(
        &store,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        DEFAULT_EXPLAIN_DEPTH,
    )
    .unwrap();
    assert_eq!(out["found"], Json::Bool(true));
    assert!(
        out.get("derivation").is_none(),
        "a base fact has no derivation node: {out}"
    );
}

/// A Datalog-derived fact explains back to its rule and premises.
#[test]
fn rule_derived_fact_explains_to_rule_and_premises() {
    use crate::reasoner::{RULE_NS, evaluate, parse_rules};

    let mut store = Store::open_in_memory().unwrap();
    ingest(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:a ex:p ex:b .\n",
    );
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ; rule:id "R1" ;
    rule:head "<http://example.org/h>(?x, ?y)" ;
    rule:body "<http://example.org/p>(?x, ?y)" .
"#
    );
    // The walk resolves the rule from the STORED shapes — load, then run.
    store.load_shapes("rules", &ttl, TS).unwrap();
    let rs = parse_rules(&ttl, None).unwrap();
    evaluate(&mut store, &rs, TS).unwrap();

    let out = explain(
        &store,
        "http://example.org/a",
        "http://example.org/h",
        "http://example.org/b",
        DEFAULT_EXPLAIN_DEPTH,
    )
    .unwrap();
    assert_eq!(out["source"], Json::String("reasoner:R1".into()));
    assert_eq!(out["derivation"]["kind"], Json::String("rule".into()));
    assert_eq!(out["derivation"]["rule"], Json::String("R1".into()));
    let premises = out["derivation"]["premises"]
        .as_array()
        .expect("premises array");
    assert_eq!(premises.len(), 1);
    assert_eq!(
        premises[0]["fact"]["p"],
        Json::String("http://example.org/p".into()),
        "the premise is the base fact the rule matched"
    );
    assert!(
        premises[0].get("derivation").is_none(),
        "the premise is a base fact"
    );
}

/// An OWL-materialized fact explains to its axiom family and premise chain.
#[test]
fn owl_transitive_fact_explains_to_chain() {
    let mut store = Store::open_in_memory().unwrap();
    ingest(
        &mut store,
        "@prefix ex: <http://example.org/> .\n\
         ex:a ex:dependsOn ex:b .\nex:b ex:dependsOn ex:c .\n",
    );
    const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:dependsOn a owl:ObjectProperty, owl:TransitiveProperty .
"#;
    // Persist (so the walk's Context finds the axioms), then materialize.
    store.load_ontology("t", ONT, TS).unwrap();
    let ont = crate::owl::Ontology::from_turtle(ONT).unwrap();
    ont.materialize(&mut store, TS).unwrap();

    let out = explain(
        &store,
        "http://example.org/a",
        "http://example.org/dependsOn",
        "http://example.org/c",
        DEFAULT_EXPLAIN_DEPTH,
    )
    .unwrap();
    assert_eq!(out["source"], Json::String("owl:materialize".into()));
    assert_eq!(out["derivation"]["kind"], Json::String("owl".into()));
    let families = out["derivation"]["families"]
        .as_array()
        .expect("families array");
    assert!(
        families
            .iter()
            .any(|f| f["family"] == Json::String("transitive".into())),
        "expected the transitive family: {out}"
    );
    let transitive = families
        .iter()
        .find(|f| f["family"] == Json::String("transitive".into()))
        .unwrap();
    assert_eq!(
        transitive["premises"].as_array().unwrap().len(),
        2,
        "a transitive step has exactly two premise links"
    );
}
