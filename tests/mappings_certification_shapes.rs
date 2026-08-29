#![cfg(feature = "shacl")]

const SHAPES: &str = include_str!("../shapes/governance.ttl");

fn report(data: &str) -> quipu::ValidationFeedback {
    quipu::validate_shapes(SHAPES, data).expect("shape and fixture Turtle should parse")
}

#[test]
fn external_mapping_accepts_the_four_predicate_contract() {
    let data = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        aegis:camayoc aegis:source_uri "https://example.invalid/camayoc/README.md" ;
            aegis:access_via "file" ;
            aegis:freshness "snapshot(33d0300)" ;
            aegis:verified_by "sha256:abc" .
    "#;
    assert!(report(data).conforms);
}

#[test]
fn external_mapping_refuses_an_incomplete_or_ambiguous_pointer() {
    let data = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        aegis:camayoc aegis:source_uri "https://example.invalid/camayoc/README.md" ;
            aegis:access_via "browser maybe" ;
            aegis:freshness "recent" .
    "#;
    let result = report(data);
    assert!(!result.conforms);
    assert!(result.violations >= 3);
}

#[test]
fn certified_bundle_requires_both_distinct_signatures_and_a_passing_scrub() {
    let valid = r#"
        @prefix aegis: <http://aegis.gastown.local/ontology/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        aegis:bundle a aegis:CertifiedShareBundle ; rdfs:label "crew.qpack" ;
            aegis:canonicalGraphHash "sha256:graph" ;
            aegis:shapesBundleVersion "aegis-ontology@1" ;
            aegis:provenanceManifest aegis:manifest ;
            aegis:publisherAttestation aegis:publisher-claim ;
            aegis:certificationSeal aegis:quipu-seal .
        aegis:publisher-claim a aegis:PublisherAttestation ;
            rdfs:label "publisher attestation" ;
            aegis:attestsBundle aegis:bundle ; aegis:signingKey aegis:publisher-key ;
            aegis:attestationSignature "cosign:publisher" .
        aegis:quipu-seal a aegis:KnowledgeCertificationSeal ;
            rdfs:label "Quipu certification seal" ;
            aegis:certifiesBundle aegis:bundle ; aegis:canonicalGraphHash "sha256:graph" ;
            aegis:shapesBundleVersion "aegis-ontology@1" ; aegis:shaclReportHash "sha256:report" ;
            aegis:scrubCheckPass true ; aegis:provenanceManifest aegis:manifest ;
            aegis:signingKey aegis:certifier-key ; aegis:attestationSignature "cosign:certifier" ;
            aegis:frozenWindow aegis:window-42 .
        aegis:publisher-key a aegis:VerifierRegistration ;
            aegis:verifier "group-publisher" ; aegis:attests aegis:bundle .
        aegis:certifier-key a aegis:VerifierRegistration ;
            aegis:verifier "quipu-certifier" ; aegis:attests aegis:bundle .
    "#;
    let valid_report = report(valid);
    assert!(
        valid_report.conforms,
        "valid certification fixture failed: {:#?}",
        valid_report.results
    );

    let failed_scrub = valid.replace("aegis:scrubCheckPass true", "aegis:scrubCheckPass false");
    assert!(!report(&failed_scrub).conforms);

    let one_signature = valid.replace("aegis:publisherAttestation aegis:publisher-claim ;", "");
    assert!(!report(&one_signature).conforms);

    // The generic envelope also certifies static packs (for example
    // core.qpack.db), which have no shuttle source window.
    let static_pack = valid.replace(
        "            aegis:signingKey aegis:certifier-key ; aegis:attestationSignature \"cosign:certifier\" ;\n            aegis:frozenWindow aegis:window-42 .",
        "            aegis:signingKey aegis:certifier-key ; aegis:attestationSignature \"cosign:certifier\" .",
    );
    assert!(report(&static_pack).conforms);
}
