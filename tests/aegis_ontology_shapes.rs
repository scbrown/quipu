#![cfg(feature = "shacl")]

const SHAPES: &str = include_str!("../shapes/aegis-ontology.shapes.ttl");

#[test]
fn text_rules_require_the_projected_catalogue_fields() {
    let valid = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

        aegis:test-rule a aegis:TextRule ;
            rdfs:label "Test rule" ;
            aegis:regex "example" ;
            aegis:enforcementTier "advise" .
    "#;
    assert!(quipu::validate_shapes(SHAPES, valid).unwrap().conforms);

    let missing_regex = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

        aegis:test-rule a aegis:TextRule ;
            rdfs:label "Test rule" ;
            aegis:enforcementTier "advise" .
    "#;
    assert!(
        !quipu::validate_shapes(SHAPES, missing_regex)
            .unwrap()
            .conforms
    );
}

#[test]
fn internal_identifier_patterns_are_declared_as_text_rules() {
    assert!(SHAPES.contains("aegis:InternalIdentifierPattern rdfs:subClassOf aegis:TextRule ."));
}
