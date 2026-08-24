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

#[test]
fn desired_crew_shape_accepts_a_scoped_composable_plan() {
    let valid = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:administrator a aegis:CrewRole ; rdfs:label "administrator" .
        aegis:lead a aegis:CrewRole ; rdfs:label "lead" .
        aegis:worker a aegis:CrewRole ; rdfs:label "worker" .
        aegis:desired-crew a aegis:DesiredCrewShape ;
            rdfs:label "Desired crew" ; aegis:crewPlanStatus "active" ;
            aegis:scopeKind "fleet" ; aegis:targetSize 11 ;
            aegis:hasCrewSlot aegis:admin-slot, aegis:lead-slot, aegis:worker-slot .
        aegis:admin-slot a aegis:DesiredCrewSlot ; rdfs:label "root" ;
            aegis:requiresRole aegis:administrator ; aegis:desiredCount 1 ;
            aegis:minimumCount 1 ; aegis:elastic false ;
            aegis:desiredHarness "claude" .
        aegis:lead-slot a aegis:DesiredCrewSlot ; rdfs:label "lead" ;
            aegis:requiresRole aegis:lead ; aegis:desiredCount 1 ;
            aegis:minimumCount 1 ; aegis:elastic false ;
            aegis:desiredHarness "claude" ; aegis:reportsToSlot aegis:admin-slot .
        aegis:worker-slot a aegis:DesiredCrewSlot ; rdfs:label "workers" ;
            aegis:requiresRole aegis:worker ; aegis:desiredCount 9 ;
            aegis:minimumCount 0 ; aegis:elastic true ;
            aegis:desiredHarness "codex" ; aegis:reportsToSlot aegis:lead-slot ;
            aegis:consolidatesInto aegis:lead-slot .
    "#;
    assert!(quipu::validate_shapes(SHAPES, valid).unwrap().conforms);
}

#[test]
fn desired_crew_shape_refuses_bad_floor_and_unknown_harness() {
    let invalid = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:administrator a aegis:CrewRole ; rdfs:label "administrator" .
        aegis:bad-plan a aegis:DesiredCrewShape ; rdfs:label "Bad plan" ;
            aegis:crewPlanStatus "active" ; aegis:scopeKind "fleet" ;
            aegis:targetSize 1 ; aegis:hasCrewSlot aegis:bad-root .
        aegis:bad-root a aegis:DesiredCrewSlot ; rdfs:label "bad root" ;
            aegis:requiresRole aegis:administrator ; aegis:desiredCount 1 ;
            aegis:minimumCount -1 ; aegis:elastic false ;
            aegis:desiredHarness "other" .
    "#;
    let report = quipu::validate_shapes(SHAPES, invalid).unwrap();
    assert!(!report.conforms);
    assert!(report.violations >= 2);
}
