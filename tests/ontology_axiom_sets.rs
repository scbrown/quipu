#![cfg(feature = "owl")]

use quipu::owl::Ontology;
use quipu::store::Store;

#[test]
fn production_functional_property_set_is_parseable_and_complete() {
    let ttl = include_str!("../ontologies/aegis-functional-properties.ttl");
    let ontology = Ontology::from_turtle(ttl).expect("functional-property ontology parses");
    let summary = ontology.axiom_summary();

    assert_eq!(summary["functional_properties"], 6);
    assert_eq!(summary["disjoint_with"], 0);
    assert!(
        ontology
            .axioms
            .functional_properties
            .contains("http://www.w3.org/2000/01/rdf-schema#comment")
    );
}

#[test]
fn production_disjoint_set_is_parseable_and_complete() {
    let ttl = include_str!("../ontologies/aegis-disjoint-classes.ttl");
    let ontology = Ontology::from_turtle(ttl).expect("disjoint-class ontology parses");
    let summary = ontology.axiom_summary();

    assert_eq!(summary["disjoint_with"], 4);
    assert_eq!(summary["functional_properties"], 0);
    assert!(ontology.axioms.disjoint_with.contains(&(
        "http://aegis.gastown.local/ontology/CrewMember".into(),
        "http://aegis.gastown.local/ontology/Dashboard".into(),
    )));
    assert!(!ontology.axioms.disjoint_with.contains(&(
        "http://aegis.gastown.local/ontology/Host".into(),
        "http://aegis.gastown.local/ontology/Service".into(),
    )));
}

#[test]
fn production_topology_type_set_has_only_the_safe_range() {
    let ttl = include_str!("../ontologies/aegis-topology-types.ttl");
    let ontology = Ontology::from_turtle(ttl).expect("topology-type ontology parses");
    let summary = ontology.axiom_summary();

    assert_eq!(summary["ranges"], 1);
    assert_eq!(summary["domains"], 0);
}

#[test]
fn production_cardinality_axioms_reject_ambiguous_comments_by_default() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .load_ontology(
            "aegis-functional-properties",
            include_str!("../ontologies/aegis-functional-properties.ttl"),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
    store.invalidate_owl_cache();

    let err = quipu::rdf::ingest_rdf(
        &mut store,
        &br#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            <http://example.org/node> rdfs:comment "first", "second" ."#[..],
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:01Z",
        None,
        None,
    )
    .expect_err("two current descriptions in one write must be refused");

    assert!(err.to_string().contains("OWL constraint violation"));
    assert!(err.to_string().contains("max 1"));
}
