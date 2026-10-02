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

fn text_rule_case_fixture(kind: &str, cases: &str) -> String {
    format!(
        r#"
            @prefix aegis: <http://aegis.gastown.local/ontology/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            aegis:test-rule a aegis:{kind} ;
                rdfs:label "Example rule" ;
                aegis:regex "example" ;
                aegis:enforcementTier "advise" .
            {cases}
        "#
    )
}

#[test]
fn text_rule_cases_are_optional_and_allow_multiple_strings() {
    for kind in ["TextRule", "InternalIdentifierPattern"] {
        for cases in [
            "",
            r#"aegis:test-rule aegis:mustMatch "example", "another example" ;
                aegis:mustNotMatch "near miss", ""^^xsd:string ."#,
        ] {
            let data = text_rule_case_fixture(kind, cases);
            assert!(quipu::validate_shapes(SHAPES, &data).unwrap().conforms);
        }
    }
}

#[test]
fn text_rule_cases_reject_non_string_values_for_both_polarities() {
    for kind in ["TextRule", "InternalIdentifierPattern"] {
        for predicate in ["mustMatch", "mustNotMatch"] {
            for value in ["42", "aegis:example", r#""example"@en"#] {
                let cases = format!("aegis:test-rule aegis:{predicate} {value} .");
                let data = text_rule_case_fixture(kind, &cases);
                assert!(
                    !quipu::validate_shapes(SHAPES, &data).unwrap().conforms,
                    "{kind}.{predicate} must reject {value}"
                );
            }
        }
    }
}

#[test]
fn directive_issuer_accepts_legacy_text_and_an_entity_iri() {
    // The REJECT half: what the write gate enforces. The full file also carries
    // the emit-mode DirectiveTraceabilityShape (aegis-4c3ppi), which reports this
    // untraced fixture by design; that is `quipu validate`'s job, not this test's.
    let reject = quipu::shacl::split_shapes_by_policy(SHAPES).reject;
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
        quipu::validate_shapes(&reject, &fixture("\"Stiwi\""))
            .unwrap()
            .conforms
    );
    assert!(
        quipu::validate_shapes(&reject, &fixture("aegis:Stiwi"))
            .unwrap()
            .conforms
    );
    assert!(
        !quipu::validate_shapes(&reject, &fixture("42"))
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
fn ci_job_accepts_a_typed_local_equivalent_and_gated_paths() {
    let data = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:test-job a aegis:CiJob ; rdfs:label "shape checks" ;
            aegis:localEquivalent aegis:test-command ;
            aegis:gatesPath "shapes/", "tests/" .
        aegis:test-command a aegis:LocalCommand ; rdfs:label "local shape checks" ;
            aegis:commandText "just test --test aegis_ontology_shapes" .
    "#;
    assert!(quipu::validate_shapes(SHAPES, data).unwrap().conforms);
}

#[test]
fn ci_job_rejects_untyped_or_ambiguous_local_equivalents() {
    let data = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:test-job a aegis:CiJob ; rdfs:label "shape checks" ;
            aegis:localEquivalent aegis:one, aegis:two .
    "#;
    assert!(!quipu::validate_shapes(SHAPES, data).unwrap().conforms);
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

const CRED_PREFIXES: &str = r#"
    @prefix aegis: <http://aegis.gastown.local/ontology/> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    aegis:h1 a aegis:Host ; rdfs:label "h1" .
"#;

#[test]
fn credential_inventory_fields_are_optional_and_constrained_when_present() {
    // aegis-zjpqjr: legacy Credential nodes carry none of these; the seeded ones carry all.
    let valid = format!(
        "{CRED_PREFIXES}
        aegis:legacy a aegis:Credential ; rdfs:label \"legacy\" .
        aegis:seeded a aegis:Credential ; rdfs:label \"seeded\" ; aegis:status \"retired\" ;
            aegis:expiresAt \"unknown\" ; aegis:keeper \"dearing\" ; aegis:probe \"none\" ;
            aegis:heldOn aegis:h1 .
        aegis:dated a aegis:Credential ; rdfs:label \"dated\" ;
            aegis:expiresAt \"2026-10-01T00:00:00-04:00\" .
        aegis:v1 a aegis:Verification ; rdfs:label \"v1\" ; aegis:result \"works\" ;
            aegis:verifiedAt \"2026-09-23T21:30:00-04:00\" ."
    );
    assert!(quipu::validate_shapes(SHAPES, &valid).unwrap().conforms);

    for bad in [
        "aegis:c a aegis:Credential ; rdfs:label \"c\" ; aegis:status \"deleted\" .",
        "aegis:c a aegis:Credential ; rdfs:label \"c\" ; aegis:expiresAt \"next tuesday\" .",
        "aegis:c a aegis:Credential ; rdfs:label \"c\" ; aegis:heldOn aegis:not-a-host .",
        "aegis:v a aegis:Verification ; rdfs:label \"v\" ; aegis:result \"probably\" .",
        "aegis:v a aegis:Verification ; rdfs:label \"v\" ; aegis:verifiedAt \"yesterday\" .",
    ] {
        let data = format!("{CRED_PREFIXES}\n{bad}");
        assert!(
            !quipu::validate_shapes(SHAPES, &data).unwrap().conforms,
            "should be refused: {bad}"
        );
    }
}

// aegis:leadFor (aegis-cpfw7a): a lead paired 1:1 with a keeper.
fn lead_for_fixture(body: &str) -> String {
    format!(
        r#"
            @prefix aegis: <http://aegis.gastown.local/ontology/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            aegis:lead a aegis:CrewRole ; rdfs:label "lead" .
            aegis:keeper a aegis:CrewRole ; rdfs:label "keeper" .
            aegis:worker a aegis:CrewRole ; rdfs:label "worker" .
            aegis:ian a aegis:CrewMember ; rdfs:label "ian" ; aegis:hasRole aegis:lead .
            aegis:harding a aegis:CrewMember ; rdfs:label "harding" ; aegis:hasRole aegis:lead .
            aegis:wu a aegis:CrewMember ; rdfs:label "wu" ; aegis:hasRole aegis:keeper .
            aegis:dearing a aegis:CrewMember ; rdfs:label "dearing" ; aegis:hasRole aegis:keeper .
            aegis:kelly a aegis:CrewMember ; rdfs:label "kelly" ; aegis:hasRole aegis:worker .
            aegis:products rdfs:label "products" .
            aegis:a-service rdfs:label "a service" ; aegis:hasRole aegis:keeper .
            {body}
        "#
    )
}

/// The report's violations, as (focus, path) pairs.
fn lead_for_violations(body: &str) -> Vec<(String, Option<String>)> {
    quipu::validate_shapes(SHAPES, &lead_for_fixture(body))
        .unwrap()
        .results
        .into_iter()
        .map(|r| (r.focus_node, r.path))
        .collect()
}

#[test]
fn lead_for_accepts_the_two_planned_pairings() {
    assert_eq!(
        lead_for_violations(""),
        vec![],
        "control: the fixture alone conforms"
    );
    assert_eq!(
        lead_for_violations(
            "aegis:ian aegis:leadFor aegis:wu . aegis:harding aegis:leadFor aegis:dearing ."
        ),
        vec![]
    );
}

#[test]
fn lead_for_refuses_every_broken_pairing() {
    const NS: &str = "http://aegis.gastown.local/ontology/";
    for (why, body, path) in [
        (
            "subject is not a lead",
            "aegis:kelly aegis:leadFor aegis:wu .",
            "hasRole",
        ),
        (
            "target is not a keeper",
            "aegis:ian aegis:leadFor aegis:kelly .",
            "leadFor",
        ),
        (
            "target is not a CrewMember",
            "aegis:ian aegis:leadFor aegis:products .",
            "leadFor",
        ),
        // Claims the keeper role but is not a CrewMember: only sh:class catches it.
        (
            "a keeper that is not a CrewMember",
            "aegis:ian aegis:leadFor aegis:a-service .",
            "leadFor",
        ),
        (
            "a lead with two keepers",
            "aegis:ian aegis:leadFor aegis:wu , aegis:dearing .",
            "leadFor",
        ),
        (
            "a keeper with two leads",
            "aegis:ian aegis:leadFor aegis:wu . aegis:harding aegis:leadFor aegis:wu .",
            "leadFor",
        ),
    ] {
        let v = lead_for_violations(body);
        assert!(!v.is_empty(), "{why} must not conform");
        assert!(
            v.iter()
                .all(|(_, p)| p.as_deref().is_some_and(|p| p.ends_with(path))
                    || p.as_deref()
                        .is_some_and(|p| p.contains(&format!("{NS}leadFor")))),
            "{why}: every violation must come from the leadFor pairing, got {v:?}"
        );
    }
}

#[test]
fn lead_for_shapes_route_to_the_rejecting_document_together() {
    // Emit only OBSERVES; 1:1 must gate the write. All three shapes must also
    // land in ONE routed document, or the sh:node reference dangles.
    let split = quipu::shacl::split_shapes_by_policy(SHAPES);
    for shape in [
        "aegis:LeadForShape",
        "aegis:KeeperRoleShape",
        "aegis:LeadForKeeperShape",
    ] {
        assert!(
            split.reject.contains(&format!("{shape} a sh:NodeShape")),
            "{shape} must reject"
        );
        assert!(
            !split.emit.contains(&format!("{shape} a sh:NodeShape")),
            "{shape} must not emit"
        );
    }
    assert!(
        split.emit.contains("aegis:HasRoleShape a sh:NodeShape"),
        "control: the splitter does route emit shapes"
    );
}

/// aegis-1mv0to: the production DirectiveTraceabilityShape REPORTS an untraced
/// directive as a warning and does not make the data non-conforming. Validated
/// against the FULL file (both halves), the path share import and compose take.
/// If this fails, a release's repository share quarantines again.
#[test]
fn directive_traceability_is_a_warning_on_the_full_shapes_file() {
    let data = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:untraced-directive a aegis:Directive ;
            rdfs:label "Untraced directive" ;
            aegis:issuedBy "Stiwi" .
    "#;
    let feedback = quipu::validate_shapes(SHAPES, data).unwrap();
    assert!(
        feedback.conforms,
        "an untraced directive must not block: {:?}",
        feedback.results
    );
    assert_eq!(feedback.violations, 0);
    assert!(
        feedback
            .results
            .iter()
            .any(|r| r.severity.contains("Warning")
                && r.source_shape
                    .as_deref()
                    .is_some_and(|s| s.contains("DirectiveTraceabilityShape"))),
        "the traceability gap must still be reported: {:?}",
        feedback.results
    );
}
