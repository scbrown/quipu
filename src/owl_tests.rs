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
fn loaded_range_applies_to_facts_written_after_ontology_load() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .load_ontology("range", TEST_ONTOLOGY, "2026-01-01T00:00:00Z")
        .unwrap();

    let alice = store.intern("http://example.org/alice").unwrap();
    let bob = store.intern("http://example.org/bob").unwrap();
    let knows = store.intern("http://example.org/knows").unwrap();
    store
        .transact(
            &[Datum {
                entity: alice,
                attribute: knows,
                value: Value::Ref(bob),
                valid_from: "2026-01-02T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-02T00:00:00Z",
            Some("test"),
            Some("post-load-range-probe"),
        )
        .unwrap();

    let result = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/alice> a <http://example.org/Person> . \
               <http://example.org/bob> a <http://example.org/Person> }",
    )
    .unwrap();
    assert!(matches!(result, crate::sparql::QueryResult::Ask(true)));
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
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/fido> a <http://example.org/Animal> }",
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
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/paper1> <http://example.org/authoredBy> <http://example.org/alice> }",
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

/// `rdfs:subPropertyOf` must RESTATE facts under the superproperty (aegis-qfncf).
///
/// Regression guard for an axiom class that was parsed and then dropped: `Axioms`
/// carried `subproperty_of`, `axiom_summary()` counted it, `/ontology` echoed it
/// back — and `materialize()` never read it, so loading one returned success and
/// changed nothing. Nothing failed, because nothing asked.
///
/// Uses its own ontology rather than `TEST_ONTOLOGY`, whose axiom counts are asserted
/// exactly by `ontology_axiom_summary`.
#[test]
fn materialize_subproperty_restates_facts_under_the_superproperty() {
    const SUBPROP_ONTOLOGY: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:touches a owl:ObjectProperty .
ex:references a owl:ObjectProperty ;
    rdfs:subPropertyOf ex:touches .
ex:calls a owl:ObjectProperty ;
    rdfs:subPropertyOf ex:references .
"#;
    let ont = Ontology::from_turtle(SUBPROP_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    let data = r#"
@prefix ex: <http://example.org/> .
ex:fnA ex:calls ex:fnB .
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
        report.sub_property_inferences > 0,
        "expected subproperty inferences, got {}",
        report.sub_property_inferences
    );

    // One hop: calls -> references.
    let direct = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/fnA> <http://example.org/references> <http://example.org/fnB> }",
    )
    .unwrap();
    assert!(
        matches!(direct, crate::sparql::QueryResult::Ask(true)),
        "fnA should reference fnB via calls ⊑ references"
    );

    // TRANSITIVE: calls ⊑ references ⊑ touches. This is the half that makes a
    // `touches` superproperty worth declaring at all — one query catching every
    // predicate beneath it, however deep.
    let transitive = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/fnA> <http://example.org/touches> <http://example.org/fnB> }",
    )
    .unwrap();
    assert!(
        matches!(transitive, crate::sparql::QueryResult::Ask(true)),
        "fnA should touch fnB via the calls ⊑ references ⊑ touches chain"
    );
}

/// `owl:TransitiveProperty` must materialize the FULL closure (quipu-923).
///
/// Regression guard for the same dropped-axiom shape as aegis-qfncf: `Axioms`
/// carried `transitive_properties`, `axiom_summary()` counted it, and
/// `materialize()` never read it — a loaded transitive property returned
/// success and derived nothing. A 3-link chain pins fixpoint (a→d), not one
/// join pass (a→c only); the second materialize pins idempotence.
#[test]
fn materialize_transitive_property_full_closure() {
    const TRANS_ONTOLOGY: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .

ex:dependsOn a owl:ObjectProperty, owl:TransitiveProperty .
"#;
    let ont = Ontology::from_turtle(TRANS_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    let data = r#"
@prefix ex: <http://example.org/> .
ex:a ex:dependsOn ex:b .
ex:b ex:dependsOn ex:c .
ex:c ex:dependsOn ex:d .
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
    // Closure of a 3-link chain adds a→c, a→d, b→d.
    assert_eq!(
        report.transitive_inferences, 3,
        "expected the full closure of a 3-link chain"
    );

    let full = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/a> <http://example.org/dependsOn> <http://example.org/d> }",
    )
    .unwrap();
    assert!(
        matches!(full, crate::sparql::QueryResult::Ask(true)),
        "a should depend on d via the full transitive closure, not one join pass"
    );

    // Idempotence: everything is already asserted, so a re-run derives nothing.
    let rerun = ont.materialize(&mut store, "2026-01-02T00:00:00Z").unwrap();
    assert_eq!(
        rerun.transitive_inferences, 0,
        "re-running materialization must be a no-op for the closure"
    );
}

/// `owl:equivalentProperty` must restate facts in BOTH directions (quipu-923).
///
/// Same recovered dead-end shape: parsed, counted, never materialized.
#[test]
fn materialize_equivalent_property_restates_both_directions() {
    const EQUIV_ONTOLOGY: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .

ex:wrote a owl:ObjectProperty ;
    owl:equivalentProperty ex:authored .
"#;
    let ont = Ontology::from_turtle(EQUIV_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice ex:wrote ex:paper1 .
ex:bob ex:authored ex:paper2 .
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
    assert_eq!(
        report.equivalent_property_inferences, 2,
        "each direction restates its one fact"
    );

    for ask in [
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/alice> <http://example.org/authored> <http://example.org/paper1> }",
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/bob> <http://example.org/wrote> <http://example.org/paper2> }",
    ] {
        let result = crate::sparql::query(&store, ask).unwrap();
        assert!(
            matches!(result, crate::sparql::QueryResult::Ask(true)),
            "equivalentProperty must restate in both directions: {ask}"
        );
    }

    // Idempotence: both restatements now exist under both predicates.
    let rerun = ont.materialize(&mut store, "2026-01-02T00:00:00Z").unwrap();
    assert_eq!(
        rerun.equivalent_property_inferences, 0,
        "re-running materialization must be a no-op for equivalent properties"
    );
}

/// Materialization must reach FIXPOINT across axiom families (quipu-923, gap G3).
///
/// The one-shot version collected type facts once, so a type introduced by
/// `rdfs:range` in the same run never fed the subclass closure — the recorded
/// staleness that forced re-encoding OWL axioms as Datalog rules
/// (shapes/aegis-class-subsumption.rules.ttl). One `materialize()` call must
/// compose: range infers `bob a Person`, the next pass lifts it to `Agent`.
#[test]
fn materialize_reaches_fixpoint_across_axiom_families() {
    const FIXPOINT_ONTOLOGY: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:Agent a owl:Class .
ex:Person a owl:Class ;
    rdfs:subClassOf ex:Agent .
ex:knows a owl:ObjectProperty ;
    rdfs:range ex:Person .
"#;
    let ont = Ontology::from_turtle(FIXPOINT_ONTOLOGY).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    // bob carries NO asserted type: his Person-ness exists only via range
    // inference, so his Agent-ness requires the second pass.
    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice ex:knows ex:bob .
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
    assert!(report.domain_range_inferences > 0, "range must fire");
    assert!(
        report.subclass_inferences > 0,
        "subclass closure must see the range-inferred type in the same call"
    );

    let result = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/bob> a <http://example.org/Agent> }",
    )
    .unwrap();
    assert!(
        matches!(result, crate::sparql::QueryResult::Ask(true)),
        "bob must be an Agent: range → Person, then Person ⊑ Agent, in one materialize"
    );

    // Fixpoint reached: a re-run derives nothing at all.
    let rerun = ont.materialize(&mut store, "2026-01-02T00:00:00Z").unwrap();
    assert_eq!(rerun.total, 0, "a re-run at fixpoint must be a no-op");
}

/// The write path must REJECT an owl:disjointWith violation (aegis-bmqup).
///
/// `Ontology::validate()` implemented this and had no caller in the server, while
/// docs/book/src/concepts/owl.md claimed write-time enforcement. These two tests
/// are the difference between the doc being true and being aspiration.
#[cfg(feature = "owl")]
#[test]
fn write_path_rejects_disjoint_class_violation() {
    const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:Person a owl:Class .
ex:Robot a owl:Class ;
    owl:disjointWith ex:Person .
"#;
    let mut store = Store::open_in_memory().unwrap();
    store.owl_config_mut().validate_on_write = true;
    store
        .load_ontology("t", ONT, "2026-01-01T00:00:00Z")
        .unwrap();
    store.invalidate_owl_cache();

    // CONTROL: one type alone must be accepted, or the test proves nothing
    // beyond "writes fail".
    let ok = crate::rdf::ingest_rdf(
        &mut store,
        b"@prefix ex: <http://example.org/> .\nex:r2d2 a ex:Robot .\n".as_ref(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    );
    assert!(ok.is_ok(), "a single non-conflicting type must be accepted");

    // The violation: the same entity also typed as the disjoint class.
    let err = crate::rdf::ingest_rdf(
        &mut store,
        b"@prefix ex: <http://example.org/> .\nex:r2d2 a ex:Person .\n".as_ref(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    );
    let msg = format!("{:?}", err.unwrap_err());
    assert!(
        msg.contains("OWL constraint violation") && msg.contains("isjoint"),
        "expected a structured disjointWith rejection, got: {msg}"
    );

    // FAILED CLOSED: the offending type must not be in the store.
    let stuck = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/r2d2> a <http://example.org/Person> }",
    )
    .unwrap();
    assert!(
        matches!(stuck, crate::sparql::QueryResult::Ask(false)),
        "the rejected type must NOT have been written"
    );
}

/// A functional property must SUPERSEDE on update, not reject (aegis-7vn3b).
///
/// SEMANTICS CORRECTION, recorded rather than quietly rewritten. aegis-bmqup shipped
/// a test here asserting the OPPOSITE — that a second value in a LATER write is
/// rejected. That encoded a bug: in a bitemporal store `owl:FunctionalProperty`
/// means at most one value AT A TIME, so a new value closes the old. Enforcing
/// rejection instead turned every ordinary edit-and-re-ingest into an HTTP 400.
/// That test was removed and replaced by this one.
///
/// Asserted against the MECHANISM, not a count: re-run the producing path, then
/// re-measure. A test that only counted values would pass on a store where nothing
/// had been updated yet.
#[cfg(feature = "owl")]
#[test]
fn functional_property_supersedes_on_update_instead_of_rejecting() {
    const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:contentHash a owl:DatatypeProperty, owl:FunctionalProperty .
"#;
    let mut store = Store::open_in_memory().unwrap();
    store.owl_config_mut().validate_on_write = true;
    store
        .load_ontology("t", ONT, "2026-01-01T00:00:00Z")
        .unwrap();
    store.invalidate_owl_cache();

    fn ingest(store: &mut Store, hash: &str, ts: &str) -> crate::error::Result<(i64, usize)> {
        let ttl =
            format!("@prefix ex: <http://example.org/> .\nex:doc1 ex:contentHash \"{hash}\" .\n");
        crate::rdf::ingest_rdf(
            store,
            ttl.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            ts,
            None,
            None,
        )
    }
    fn asks(store: &Store, v: &str) -> bool {
        let q =
            format!("ASK {{ <http://example.org/doc1> <http://example.org/contentHash> \"{v}\" }}");
        matches!(
            crate::sparql::query(store, &q).unwrap(),
            crate::sparql::QueryResult::Ask(true)
        )
    }

    ingest(&mut store, "aaa", "2026-01-01T00:00:00Z").unwrap();
    assert!(
        asks(&store, "aaa"),
        "control: the first value must be current"
    );

    // RE-RUN THE PRODUCING PATH: the same document, edited.
    ingest(&mut store, "bbb", "2026-01-02T00:00:00Z")
        .expect("an ordinary update to a functional property must be ACCEPTED, not rejected");

    // RE-MEASURE.
    assert!(
        !asks(&store, "aaa"),
        "the superseded value must no longer be current"
    );
    assert!(asks(&store, "bbb"), "the new value must be current");

    // Not one-shot: a third update must work too.
    ingest(&mut store, "ccc", "2026-01-03T00:00:00Z")
        .expect("a second update must also be accepted");
    assert!(asks(&store, "ccc"));
    assert!(!asks(&store, "bbb"));

    // SUPERSEDE, NOT DELETE. A store that ERASED the prior value would satisfy
    // every assertion above and still be wrong — this is a bitemporal log.
    let doc_id = store
        .lookup("http://example.org/doc1")
        .unwrap()
        .expect("doc1 must exist");
    let hist = format!("{:?}", store.entity_history(doc_id).unwrap());
    assert!(
        hist.contains("aaa"),
        "the superseded value must remain in history, not be erased"
    );
}

/// Two distinct values for one functional property in a SINGLE batch must still be
/// REJECTED (aegis-7vn3b) — superseding there would silently pick a winner.
#[cfg(feature = "owl")]
#[test]
fn functional_property_still_rejects_two_values_in_one_batch() {
    const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:contentHash a owl:DatatypeProperty, owl:FunctionalProperty .
"#;
    let mut store = Store::open_in_memory().unwrap();
    store.owl_config_mut().validate_on_write = true;
    store
        .load_ontology("t", ONT, "2026-01-01T00:00:00Z")
        .unwrap();
    store.invalidate_owl_cache();

    let err = crate::rdf::ingest_rdf(
        &mut store,
        b"@prefix ex: <http://example.org/> .\nex:doc2 ex:contentHash \"aaa\", \"bbb\" .\n"
            .as_ref(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    );
    let msg = format!("{:?}", err.unwrap_err());
    assert!(
        msg.contains("OWL constraint violation"),
        "an ambiguous batch must still be refused, got: {msg}"
    );
}

// --- semi-naive derive: same fixpoint, bounded cost (aegis-2dp8e2) -----------
//
// The naive path re-read EVERY current fact on every pass. Measured on the live
// store: 641,803 facts, >= 2 scans per relevant write, ~67 s of work per 60 s of
// wall clock at a 29 writes/min offered load — it could not keep up, and it
// ratcheted, because its own output lands in the companion graph which is a
// premise for the next run.
//
// Semi-naive is an EVALUATION-STRATEGY change, NOT a semantics change. The
// fixpoint must be identical, and the test for that is a DIFF of the derived
// sets — not a spot-check of a few ASKs, which passes while missing whole rule
// families.

const SN_TS: &str = "2026-01-01T00:00:00Z";

/// An ontology exercising SEVEN single-premise families plus the one that is not.
const SN_ONT: &str = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:   <http://example.org/> .
ex:Dog     rdfs:subClassOf ex:Mammal .
ex:Mammal  rdfs:subClassOf ex:Animal .
ex:Pet     owl:equivalentClass ex:Companion .
ex:owns    owl:inverseOf ex:ownedBy .
ex:knows   a owl:SymmetricProperty .
ex:partOf  a owl:TransitiveProperty .
ex:likes   rdfs:subPropertyOf ex:regards .
ex:cares   owl:equivalentProperty ex:tends .
ex:feeds   rdfs:domain ex:Keeper ; rdfs:range ex:Animal .
"#;

fn sn_ingest(store: &mut Store, ttl: &str) {
    crate::rdf::ingest_rdf(
        store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        SN_TS,
        None,
        None,
    )
    .unwrap();
}

/// Every fact in the companion inferred graph, as a comparable set.
fn sn_derived(store: &Store) -> std::collections::BTreeSet<(i64, i64, Vec<u8>)> {
    let companion = store
        .lookup("urn:quipu:graph:root#inferred")
        .unwrap()
        .expect("companion graph");
    store
        .current_facts_in_graph(companion)
        .unwrap()
        .into_iter()
        .map(|f| (f.entity, f.attribute, f.value.to_bytes()))
        .collect()
}

#[test]
fn semi_naive_reaches_the_same_fixpoint_as_naive() {
    // THE correctness test. Two stores, identical input; one materialised the
    // naive way, one from the delta. The DERIVED SETS must be equal — every
    // rule family, not a sampled few.
    let data = r#"
@prefix ex: <http://example.org/> .
ex:fido   a ex:Dog .
ex:rex    a ex:Pet .
ex:ann    ex:owns    ex:fido .
ex:ann    ex:knows   ex:bob .
ex:paw    ex:partOf  ex:leg .
ex:leg    ex:partOf  ex:fido .
ex:ann    ex:likes   ex:rex .
ex:ann    ex:cares   ex:rex .
ex:ann    ex:feeds   ex:fido .
"#;

    let mut naive = Store::open_in_memory().unwrap();
    naive.load_ontology("t", SN_ONT, SN_TS).unwrap();
    sn_ingest(&mut naive, data);
    let ont = crate::owl::Ontology::from_turtle(SN_ONT).unwrap();
    let rn = ont.materialize(&mut naive, SN_TS).unwrap();

    let mut semi = Store::open_in_memory().unwrap();
    semi.load_ontology("t", SN_ONT, SN_TS).unwrap();
    sn_ingest(&mut semi, data);
    // The seed is every asserted fact — what a delta carrying this ingest holds.
    let seed = semi
        .current_facts_in_graph(crate::schema::ROOT_GRAPH)
        .unwrap();
    let rs = ont.materialize_delta(&mut semi, SN_TS, &seed).unwrap();

    // ANTI-VACUITY: if nothing was derived, set equality is trivially true and
    // this test would pass against a materialiser that does nothing at all.
    assert!(
        rn.total > 0,
        "fixture derives nothing — the comparison below would be vacuous"
    );

    assert_eq!(
        sn_derived(&naive),
        sn_derived(&semi),
        "semi-naive must reach the SAME fixpoint as naive — it is an evaluation \
         strategy, not a semantics change (naive total {}, semi total {})",
        rn.total,
        rs.total
    );
}

#[test]
fn transitive_closure_joins_the_delta_against_history() {
    // THE ARM THAT DELTA-ONLY WOULD FAIL, and the reason the (delta x existing)
    // path exists at all. `a→b` and `b→c` arrive in DIFFERENT transactions, so a
    // delta-only adjacency sees one edge and cannot derive `a→c`.
    //
    // This deployment has ZERO transitive properties loaded, so nothing here
    // exercises this path in production — which is exactly how it would rot
    // unnoticed. Hence a test that loads one.
    let mut store = Store::open_in_memory().unwrap();
    store.load_ontology("t", SN_ONT, SN_TS).unwrap();
    let ont = crate::owl::Ontology::from_turtle(SN_ONT).unwrap();

    // Transaction 1: a→b, materialised.
    sn_ingest(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:a ex:partOf ex:b .\n",
    );
    let first = store
        .current_facts_in_graph(crate::schema::ROOT_GRAPH)
        .unwrap();
    ont.materialize_delta(&mut store, SN_TS, &first).unwrap();

    // Transaction 2: b→c ONLY. The delta does not contain a→b.
    let before = store
        .current_facts_in_graph(crate::schema::ROOT_GRAPH)
        .unwrap();
    sn_ingest(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:b ex:partOf ex:c .\n",
    );
    let after = store
        .current_facts_in_graph(crate::schema::ROOT_GRAPH)
        .unwrap();
    let seed: Vec<_> = after
        .iter()
        .filter(|f| {
            !before
                .iter()
                .any(|b| b.entity == f.entity && b.attribute == f.attribute && b.value == f.value)
        })
        .cloned()
        .collect();
    assert_eq!(
        seed.len(),
        1,
        "the seed must be the ONE new edge, not the store"
    );

    ont.materialize_delta(&mut store, SN_TS, &seed).unwrap();

    let result = crate::sparql::query(
        &store,
        "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> \
         { <http://example.org/a> <http://example.org/partOf> <http://example.org/c> }",
    )
    .unwrap();
    assert!(
        matches!(result, crate::sparql::QueryResult::Ask(true)),
        "a→c must be derived from a delta containing ONLY b→c, by joining it \
         against the EXISTING a→b edge (aegis-2dp8e2)"
    );
}

#[test]
fn the_pass_budget_is_reported_not_silent() {
    // A fixpoint that stopped early is indistinguishable from one that finished
    // unless it says so. Normal runs must report the flag CLEAR — the assertion
    // that stops the field being decorative.
    let mut store = Store::open_in_memory().unwrap();
    store.load_ontology("t", SN_ONT, SN_TS).unwrap();
    let ont = crate::owl::Ontology::from_turtle(SN_ONT).unwrap();
    sn_ingest(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:fido a ex:Dog .\n",
    );
    let seed = store
        .current_facts_in_graph(crate::schema::ROOT_GRAPH)
        .unwrap();
    let r = ont.materialize_delta(&mut store, SN_TS, &seed).unwrap();
    assert!(
        !r.pass_budget_exhausted,
        "an ordinary run must not hit the budget"
    );
    assert!(r.passes > 0, "passes must be counted, not left at zero");
}
