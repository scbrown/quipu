use super::*;

#[test]
fn empty_catalogue_cannot_verify_outward_payload() {
    let store = Store::open_in_memory().unwrap();
    let files = BTreeMap::from([("export.nt".into(), "clean".into())]);
    assert!(matches!(
        scrub_outward_payload(&store, &files, "test"),
        Err(Error::CannotVerify(_))
    ));
}

#[test]
fn named_catalogue_rejects_a_real_violation_and_accepts_clean_payload() {
    let mut store = Store::open_in_memory().unwrap();
    seed_test_catalogue(&mut store);
    let clean = BTreeMap::from([("export.nt".into(), "clean".into())]);
    scrub_outward_payload(&store, &clean, "test").unwrap();
    let bad = BTreeMap::from([("export.nt".into(), "FIXTURE_PRIVATE_TOKEN".into())]);
    assert!(matches!(
        scrub_outward_payload(&store, &bad, "test"),
        Err(Error::PolicyDenied(_))
    ));
}

#[test]
fn split_catalogue_statements_do_not_manufacture_a_rule() {
    let mut store = Store::open_in_memory().unwrap();
    for (name, ttl) in [
        (
            "urn:test:one",
            "<urn:rule> a <http://aegis.gastown.local/ontology/InternalIdentifierPattern> ; <http://www.w3.org/2000/01/rdf-schema#label> \"split\" .",
        ),
        (
            "urn:test:two",
            "<urn:rule> <http://aegis.gastown.local/ontology/regex> \"secret\" ; <http://aegis.gastown.local/ontology/enforcementTier> \"block\" .",
        ),
    ] {
        let graph = store.overlay_create(name, 0).unwrap();
        crate::rdf::ingest_rdf_to_graph(
            &mut store,
            ttl.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-08-01T00:00:00Z",
            None,
            Some("test"),
            graph,
        )
        .unwrap();
    }
    assert!(matches!(
        outward_scrub_patterns(&store),
        Err(Error::CannotVerify(_))
    ));
}
