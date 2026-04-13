use super::*;

const TEST_ONTOLOGY: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix ex: <http://example.org/> .

ex:Animal a owl:Class .
ex:Mammal a owl:Class ;
    rdfs:subClassOf ex:Animal .
ex:Dog a owl:Class ;
    rdfs:subClassOf ex:Mammal .

ex:Person a owl:Class .
ex:Robot a owl:Class ;
    owl:disjointWith ex:Person .

ex:authors a owl:ObjectProperty .
ex:authoredBy a owl:ObjectProperty ;
    owl:inverseOf ex:authors .

ex:friendOf a owl:ObjectProperty, owl:SymmetricProperty .

ex:ssn a owl:DatatypeProperty, owl:FunctionalProperty .

ex:knows a owl:ObjectProperty ;
    rdfs:domain ex:Person ;
    rdfs:range ex:Person .
"#;

#[test]
fn parse_ontology_extracts_axioms() {
    let ont = Ontology::from_turtle(TEST_ONTOLOGY).unwrap();
    assert_eq!(ont.axioms.subclass_of.len(), 2);
    assert!(ont.axioms.disjoint_with.contains(&(
        "http://example.org/Robot".into(),
        "http://example.org/Person".into()
    )));
    assert!(
        ont.axioms
            .functional_properties
            .contains("http://example.org/ssn")
    );
    assert!(
        ont.axioms
            .symmetric_properties
            .contains("http://example.org/friendOf")
    );
    assert_eq!(ont.axioms.inverse_of.len(), 2); // both directions
    assert_eq!(ont.axioms.domains.len(), 1);
    assert_eq!(ont.axioms.ranges.len(), 1);
}

#[test]
fn materialize_subclass_transitive_closure() {
    let ont = Ontology::from_turtle(TEST_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    // Add an instance: ex:fido a ex:Dog
    let data = r#"
@prefix ex: <http://example.org/> .
ex:fido a ex:Dog .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        data.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let report = ont.materialize(&mut store, "2026-01-01T00:00:00Z").unwrap();
    assert!(
        report.subclass_inferences > 0,
        "expected subclass inferences"
    );

    // Query: fido should be an Animal (via Dog → Mammal → Animal).
    let result = crate::sparql::query(
        &store,
        "ASK { <http://example.org/fido> a <http://example.org/Animal> }",
    )
    .unwrap();
    assert!(
        matches!(result, crate::sparql::QueryResult::Ask(true)),
        "fido should be an Animal via transitive subclass"
    );
}

#[test]
fn validate_rejects_disjoint_class() {
    let ont = Ontology::from_turtle(TEST_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    // Existing: ex:alice a ex:Person
    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        data.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // Proposed: ex:alice a ex:Robot — should violate disjoint constraint.
    let rdf_type_id = store.intern(RDF_TYPE).unwrap();
    let alice_id = store.intern("http://example.org/alice").unwrap();
    let robot_id = store.intern("http://example.org/Robot").unwrap();

    let proposed = vec![Datum {
        entity: alice_id,
        attribute: rdf_type_id,
        value: Value::Ref(robot_id),
        valid_from: "2026-01-02T00:00:00Z".to_string(),
        valid_to: None,
        op: Op::Assert,
    }];

    let violations = ont.validate(&store, &proposed).unwrap();
    assert!(!violations.is_empty(), "expected disjoint class violation");
    assert!(violations[0].message.contains("disjoint"));
}

#[test]
fn validate_rejects_functional_property_second_value() {
    let ont = Ontology::from_turtle(TEST_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    // Existing: ex:alice ex:ssn "123"
    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice ex:ssn "123-45-6789" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        data.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // Proposed: ex:alice ex:ssn "987" — second value on functional property.
    let alice_id = store.intern("http://example.org/alice").unwrap();
    let ssn_id = store.intern("http://example.org/ssn").unwrap();

    let proposed = vec![Datum {
        entity: alice_id,
        attribute: ssn_id,
        value: Value::Str("987-65-4321".into()),
        valid_from: "2026-01-02T00:00:00Z".to_string(),
        valid_to: None,
        op: Op::Assert,
    }];

    let violations = ont.validate(&store, &proposed).unwrap();
    assert!(
        !violations.is_empty(),
        "expected functional property violation"
    );
    assert!(violations[0].message.contains("functional"));
}

#[test]
fn materialize_inverse_property() {
    let ont = Ontology::from_turtle(TEST_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice ex:authors ex:paper1 .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        data.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let report = ont.materialize(&mut store, "2026-01-01T00:00:00Z").unwrap();
    assert!(report.inverse_inferences > 0, "expected inverse inferences");

    // paper1 authoredBy alice should now exist.
    let result = crate::sparql::query(
        &store,
        "ASK { <http://example.org/paper1> <http://example.org/authoredBy> <http://example.org/alice> }",
    )
    .unwrap();
    assert!(
        matches!(result, crate::sparql::QueryResult::Ask(true)),
        "paper1 should have authoredBy alice via inverse"
    );
}

#[test]
fn ontology_axiom_summary() {
    let ont = Ontology::from_turtle(TEST_ONTOLOGY).unwrap();
    let summary = ont.axiom_summary();
    assert_eq!(summary["subclass_of"], 2);
    assert_eq!(summary["disjoint_with"], 1); // stored as 2 pairs, reported as 1
    assert_eq!(summary["functional_properties"], 1);
}

#[test]
fn store_ontology_persistence() {
    let store = Store::open_in_memory().unwrap();
    store
        .load_ontology("test", TEST_ONTOLOGY, "2026-01-01T00:00:00Z")
        .unwrap();

    let list = store.list_ontologies().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].0, "test");

    let combined = store.get_combined_ontologies().unwrap();
    assert!(combined.is_some());

    assert!(store.remove_ontology("test").unwrap());
    assert!(store.list_ontologies().unwrap().is_empty());
}
