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
fn directive_issuer_accepts_legacy_text_and_an_entity_iri() {
    let fixture = |issuer: &str| {
        format!(
            r#"
                @prefix aegis: <http://aegis.gastown.local/ontology/> .
                @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
                aegis:test-directive a aegis:Directive ;
                    rdfs:label "Test directive" ;
                    aegis:issuedBy {issuer} .
            "#
        )
    };

    assert!(
        quipu::validate_shapes(SHAPES, &fixture("\"Stiwi\""))
            .unwrap()
            .conforms
    );
    assert!(
        quipu::validate_shapes(SHAPES, &fixture("aegis:Stiwi"))
            .unwrap()
            .conforms
    );
    assert!(
        !quipu::validate_shapes(SHAPES, &fixture("42"))
            .unwrap()
            .conforms
    );
}

fn disk_impact_fixture(
    signature: &str,
    filesystem: &str,
    delta: &str,
    observed_at: &str,
) -> String {
    format!(
        r#"
            @prefix aegis: <http://aegis.gastown.local/ontology/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            aegis:test-impact a aegis:CommandDiskImpactObservation ;
                rdfs:label "cargo build disk impact" ;
                aegis:commandSignature {signature} ;
                aegis:filesystemIdentity {filesystem} ;
                aegis:diskDeltaBytes {delta} ;
                aegis:observedAt {observed_at} .
        "#
    )
}

#[test]
fn command_disk_impact_accepts_consumed_and_freed_space_samples() {
    for delta in ["\"1048576\"^^xsd:integer", "\"-4096\"^^xsd:integer"] {
        let data = disk_impact_fixture(
            "\"cargo:build|repo:rust-project|cwd:repo-root\"",
            "\"root:primary\"",
            delta,
            "\"2026-09-02T20:00:00Z\"^^xsd:dateTime",
        );
        assert!(quipu::validate_shapes(SHAPES, &data).unwrap().conforms);
    }
}

#[test]
fn command_disk_impact_rejects_raw_argv_paths_and_wrong_datatypes() {
    let cases = [
        disk_impact_fixture(
            "\"cargo build --release|repo:rust-project|cwd:/workspace/repo\"",
            "\"root:primary\"",
            "\"12\"^^xsd:integer",
            "\"2026-09-02T20:00:00Z\"^^xsd:dateTime",
        ),
        disk_impact_fixture(
            "\"cargo:build|repo:rust-project|cwd:repo-root\"",
            "\"/\"",
            "\"12.5\"^^xsd:decimal",
            "\"not-a-date\"",
        ),
    ];
    for data in cases {
        assert!(!quipu::validate_shapes(SHAPES, &data).unwrap().conforms);
    }
}

#[test]
fn command_disk_impact_requires_each_raw_sample_field() {
    let complete = disk_impact_fixture(
        "\"cargo:build|repo:rust-project|cwd:repo-root\"",
        "\"root:primary\"",
        "\"12\"^^xsd:integer",
        "\"2026-09-02T20:00:00Z\"^^xsd:dateTime",
    );
    for predicate in [
        "aegis:commandSignature",
        "aegis:filesystemIdentity",
        "aegis:diskDeltaBytes",
        "aegis:observedAt",
    ] {
        let without = complete
            .lines()
            .filter(|line| !line.contains(predicate))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!quipu::validate_shapes(SHAPES, &without).unwrap().conforms);
    }
}

#[test]
fn config_file_accepts_exact_path_and_lowercase_sha256() {
    let data = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:test-config a aegis:ConfigFile ;
            rdfs:label "crew configuration" ;
            aegis:configPath "/etc/example/config.toml" ;
            aegis:contentSha256 "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" .
    "#;
    assert!(quipu::validate_shapes(SHAPES, data).unwrap().conforms);
}

#[test]
fn config_file_rejects_noncanonical_or_ambiguous_digest_facts() {
    let invalid = [
        r#"
            @prefix aegis: <http://aegis.gastown.local/ontology/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            aegis:test-config a aegis:ConfigFile ;
                rdfs:label "crew configuration" ;
                aegis:configPath "/etc/example/config.toml" ;
                aegis:contentSha256 "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF" .
        "#,
        r#"
            @prefix aegis: <http://aegis.gastown.local/ontology/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            aegis:test-config a aegis:ConfigFile ;
                rdfs:label "crew configuration" ;
                aegis:configPath "/etc/example/config.toml", "/etc/example/other.toml" ;
                aegis:contentSha256 "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" .
        "#,
    ];
    for data in invalid {
        assert!(!quipu::validate_shapes(SHAPES, data).unwrap().conforms);
    }
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
