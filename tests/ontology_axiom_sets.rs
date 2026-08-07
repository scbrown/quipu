#![cfg(feature = "owl")]

use quipu::owl::Ontology;

#[test]
fn production_functional_property_set_is_parseable_and_complete() {
    let ttl = include_str!("../ontologies/aegis-functional-properties.ttl");
    let ontology = Ontology::from_turtle(ttl).expect("functional-property ontology parses");
    let summary = ontology.axiom_summary();

    assert_eq!(summary["functional_properties"], 5);
    assert_eq!(summary["disjoint_with"], 0);
}

#[test]
fn production_disjoint_set_is_parseable_and_complete() {
    let ttl = include_str!("../ontologies/aegis-disjoint-classes.ttl");
    let ontology = Ontology::from_turtle(ttl).expect("disjoint-class ontology parses");
    let summary = ontology.axiom_summary();

    assert_eq!(summary["disjoint_with"], 4);
    assert_eq!(summary["functional_properties"], 0);
}
