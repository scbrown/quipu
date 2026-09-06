#[cfg(test)]
mod tests {
    use super::*;

    const TURTLE_DATA: &str = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "30"^^xsd:integer ;
    ex:height "1.65"^^xsd:double ;
    ex:active "true"^^xsd:boolean ;
    ex:knows ex:bob .

ex:bob a ex:Person ;
    ex:name "Bob"@en .
"#;

    #[test]
    fn ingest_turtle_round_trip() {
        let mut store = Store::open_in_memory().unwrap();

        let (tx_id, count) = ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            Some("turtle-test"),
        )
        .unwrap();

        assert!(tx_id > 0);
        assert_eq!(count, 8); // 6 for alice + 2 for bob

        let facts = store.current_facts().unwrap();
        assert_eq!(facts.len(), 8);

        // Verify typed values came through correctly.
        let alice_id = store
            .lookup("http://example.org/alice")
            .unwrap()
            .expect("alice should be interned");
        let age_id = store
            .lookup("http://example.org/age")
            .unwrap()
            .expect("age should be interned");
        let alice_facts = store.entity_facts(alice_id).unwrap();

        let age_fact = alice_facts.iter().find(|f| f.attribute == age_id).unwrap();
        assert_eq!(age_fact.value, Value::Int(30));

        let height_id = store.lookup("http://example.org/height").unwrap().unwrap();
        let height_fact = alice_facts
            .iter()
            .find(|f| f.attribute == height_id)
            .unwrap();
        assert_eq!(
            height_fact.value,
            Value::Typed {
                lexical: "1.65E0".into(),
                datatype: namespace::XSD_DOUBLE.into(),
            }
        );

        let active_id = store.lookup("http://example.org/active").unwrap().unwrap();
        let active_fact = alice_facts
            .iter()
            .find(|f| f.attribute == active_id)
            .unwrap();
        assert_eq!(active_fact.value, Value::Bool(true));

        // Verify object reference (ex:knows ex:bob).
        let knows_id = store.lookup("http://example.org/knows").unwrap().unwrap();
        let knows_fact = alice_facts
            .iter()
            .find(|f| f.attribute == knows_id)
            .unwrap();
        let bob_id = store.lookup("http://example.org/bob").unwrap().unwrap();
        assert_eq!(knows_fact.value, Value::Ref(bob_id));

        // Verify language-tagged literal. The LEXICAL value is "Bob" and the
        // tag lives beside it. This assertion used to read Str("Bob@en") — it
        // pinned the aegis-fmyi corruption in place as expected behavior.
        let bob_facts = store.entity_facts(bob_id).unwrap();
        let name_id = store.lookup("http://example.org/name").unwrap().unwrap();
        let bob_name = bob_facts.iter().find(|f| f.attribute == name_id).unwrap();
        assert_eq!(
            bob_name.value,
            Value::Lang {
                lexical: "Bob".into(),
                lang: "en".into()
            }
        );
    }

    #[test]
    fn export_ntriples() {
        let mut store = Store::open_in_memory().unwrap();

        ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        let ntriples = export_rdf(&store, RdfFormat::NTriples).unwrap();
        let output = String::from_utf8(ntriples).unwrap();

        assert!(output.contains("<http://example.org/alice>"));
        assert!(output.contains("<http://example.org/Person>"));
        assert!(output.contains("\"Alice\""));
        assert!(output.contains("\"Bob\"@en"));
        assert!(output.contains("\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>"));
    }

    #[test]
    fn round_trip_ntriples() {
        let mut store1 = Store::open_in_memory().unwrap();

        ingest_rdf(
            &mut store1,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        let ntriples = export_rdf(&store1, RdfFormat::NTriples).unwrap();

        let mut store2 = Store::open_in_memory().unwrap();
        let (_, count) = ingest_rdf(
            &mut store2,
            ntriples.as_slice(),
            RdfFormat::NTriples,
            None,
            "2026-04-04T01:00:00Z",
            None,
            None,
        )
        .unwrap();

        assert_eq!(count, 8);
        assert_eq!(store2.current_facts().unwrap().len(), 8);
    }

    #[test]
    fn literal_to_value_types() {
        let xsd = "http://www.w3.org/2001/XMLSchema#";

        let cases: Vec<(Literal, Value)> = vec![
            (
                Literal::new_typed_literal("42", NamedNode::new_unchecked(format!("{xsd}integer"))),
                Value::Int(42),
            ),
            (
                Literal::new_typed_literal(
                    "3.25",
                    NamedNode::new_unchecked(format!("{xsd}double")),
                ),
                Value::Typed {
                    lexical: "3.25E0".into(),
                    datatype: format!("{xsd}double"),
                },
            ),
            (
                Literal::new_typed_literal(
                    "true",
                    NamedNode::new_unchecked(format!("{xsd}boolean")),
                ),
                Value::Bool(true),
            ),
            (
                Literal::new_typed_literal(
                    "false",
                    NamedNode::new_unchecked(format!("{xsd}boolean")),
                ),
                Value::Bool(false),
            ),
            (
                Literal::new_simple_literal("hello"),
                Value::Str("hello".into()),
            ),
            (
                Literal::new_language_tagged_literal("hola", "es").unwrap(),
                Value::Lang {
                    lexical: "hola".into(),
                    lang: "es".into(),
                },
            ),
            // The plain string "hola@es" must NOT collapse onto the lang literal
            // above. These two lines are the whole of aegis-fmyi.
            (
                Literal::new_simple_literal("hola@es"),
                Value::Str("hola@es".into()),
            ),
            // Datatypes without a fast-path variant keep their IRI verbatim
            // instead of decaying into an untyped string.
            (
                Literal::new_typed_literal(
                    "2026-07-15",
                    NamedNode::new_unchecked(format!("{xsd}date")),
                ),
                Value::Typed {
                    lexical: "2026-07-15".into(),
                    datatype: format!("{xsd}date"),
                },
            ),
            // xsd:decimal is EXACT and xsd:double is not; collapsing both into
            // f64 was a silent change of numeric semantics.
            (
                Literal::new_typed_literal(
                    "3.25",
                    NamedNode::new_unchecked(format!("{xsd}decimal")),
                ),
                Value::Typed {
                    lexical: "3.25".into(),
                    datatype: format!("{xsd}decimal"),
                },
            ),
            (
                Literal::new_typed_literal("42", NamedNode::new_unchecked(format!("{xsd}long"))),
                Value::Typed {
                    lexical: "42".into(),
                    datatype: format!("{xsd}long"),
                },
            ),
        ];

        for (lit, expected) in cases {
            let result = super::literal_to_value(&lit).unwrap();
            assert_eq!(result, expected, "failed for literal: {lit}");
        }
    }

    #[test]
    fn ingest_same_rdf_twice_no_duplicates() {
        let mut store = Store::open_in_memory().unwrap();

        let (_, count1) = ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("first"),
            Some("test-1"),
        )
        .unwrap();
        assert_eq!(count1, 8);
        assert_eq!(store.current_facts().unwrap().len(), 8);

        // Second ingestion of the same data — assertions are idempotent,
        // so no new facts should be created.
        let (_, count2) = ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T01:00:00Z",
            Some("second"),
            Some("test-2"),
        )
        .unwrap();

        // count2 is the number of triples parsed from the input, not written.
        assert_eq!(count2, 8);

        // But the store should still have only 8 active facts.
        assert_eq!(
            store.current_facts().unwrap().len(),
            8,
            "re-ingesting the same RDF should not create duplicates"
        );
    }

    #[test]
    fn blank_node_round_trip() {
        let ntriples = r#"
_:node1 <http://example.org/label> "test" .
<http://example.org/thing> <http://example.org/ref> _:node1 .
"#;
        let mut store = Store::open_in_memory().unwrap();
        let (_, count) = ingest_rdf(
            &mut store,
            ntriples.as_bytes(),
            RdfFormat::NTriples,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        assert_eq!(count, 2);

        // The blank node should be in the term dictionary.
        let bnode_id = store
            .lookup("_:node1")
            .unwrap()
            .expect("blank node interned");
        assert!(bnode_id > 0);

        // The reference should point to the blank node.
        let thing_id = store.lookup("http://example.org/thing").unwrap().unwrap();
        let facts = store.entity_facts(thing_id).unwrap();
        assert_eq!(facts[0].value, Value::Ref(bnode_id));
    }
    // ── STREAMING CHUNKED INGEST (aegis-j0yaxj.2) ────────────────────────────
    //
    // The prerequisite for benchmarking at 1B: `ingest_rdf_to_graph` parses the
    // whole input into a Vec and commits ONE transaction, which cannot reach that
    // scale by construction. These arms pin the properties that make the chunked
    // form safe to substitute for it.

    fn live_fact_count(store: &Store) -> i64 {
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE op = 1 AND valid_to IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// THE LOAD-BEARING ONE: chunking must not change what lands.
    ///
    /// A chunked ingest that produced a different store from the one-shot form
    /// would make every benchmark number a measurement of the harness. Same input,
    /// three chunk sizes including one larger than the dataset, one datum at a
    /// time -- all must agree with each other AND with the unchunked path.
    #[test]
    fn chunking_does_not_change_what_lands() {
        let one_shot = {
            let mut store = Store::open_in_memory().unwrap();
            ingest_rdf(
                &mut store,
                TURTLE_DATA.as_bytes(),
                RdfFormat::Turtle,
                None,
                "2026-04-04T00:00:00Z",
                Some("test"),
                None,
            )
            .unwrap();
            live_fact_count(&store)
        };

        for chunk in [1usize, 3, 10_000] {
            let mut store = Store::open_in_memory().unwrap();
            ingest_rdf_chunked(
                &mut store,
                TURTLE_DATA.as_bytes(),
                RdfFormat::Turtle,
                None,
                "2026-04-04T00:00:00Z",
                Some("test"),
                None,
                0,
                chunk,
            )
            .unwrap();
            assert_eq!(
                live_fact_count(&store),
                one_shot,
                "chunk={chunk} produced a different store from the one-shot ingest"
            );
        }
    }

    /// One transaction per chunk, and the count is arithmetic rather than luck.
    #[test]
    fn one_transaction_per_chunk_and_a_tail() {
        let mut store = Store::open_in_memory().unwrap();
        let report = ingest_rdf_chunked(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            0,
            3,
        )
        .unwrap();

        let expected = report.parsed.div_ceil(3);
        assert_eq!(
            report.tx_ids.len(),
            expected,
            "{} triples at chunk 3 must be {expected} transactions",
            report.parsed
        );
    }

    /// EVERY CHUNK CARRIES THE SAME TIMESTAMP.
    ///
    /// One ingest is one logical event. If chunks stamped their own time, a long
    /// load would appear in the store as data that trickled in, and every temporal
    /// read would believe it. Asserting ONE distinct `valid_from` is the check;
    /// asserting a specific value would pass even if the code called `now()` and got
    /// lucky within a second.
    #[test]
    fn every_chunk_carries_the_same_timestamp() {
        let mut store = Store::open_in_memory().unwrap();
        ingest_rdf_chunked(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            0,
            1,
        )
        .unwrap();

        let distinct: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT valid_from) FROM facts WHERE op = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(distinct, 1, "chunks disagreed about when the ingest happened");
    }

    /// An empty input commits NOTHING, not an empty transaction.
    ///
    /// A zero-datum tx would appear in the log as an ingest that happened, which is
    /// the same class of lie as reporting a parse count as a write count.
    #[test]
    fn an_empty_input_commits_no_transaction() {
        let mut store = Store::open_in_memory().unwrap();
        let report = ingest_rdf_chunked(
            &mut store,
            "".as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            0,
            100,
        )
        .unwrap();
        assert_eq!(report.parsed, 0);
        assert!(
            report.tx_ids.is_empty(),
            "an empty ingest opened a transaction: {:?}",
            report.tx_ids
        );
    }

    /// `parsed` IS NOT A WRITE COUNT, and this pins the difference rather than
    /// documenting it.
    ///
    /// quipu #127: `ingest_rdf_to_graph` returns `datums.len()`, which malcolm
    /// measured reporting 4 writes for a re-apply that stored nothing. This bead
    /// publishes INGEST THROUGHPUT on a public trust page, so a harness taking the
    /// number from here would report the parse -- the cheap half -- as the write.
    /// Ingesting the same content twice parses it all again and writes nothing new.
    #[test]
    fn parsed_counts_triples_not_writes() {
        let mut store = Store::open_in_memory().unwrap();
        let first = ingest_rdf_chunked(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            0,
            10,
        )
        .unwrap();
        let after_first = live_fact_count(&store);

        let second = ingest_rdf_chunked(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            0,
            10,
        )
        .unwrap();
        let after_second = live_fact_count(&store);

        assert_eq!(
            second.parsed, first.parsed,
            "the second pass parsed a different number of triples"
        );
        assert_eq!(
            after_second, after_first,
            "the second pass wrote new facts; then this test cannot demonstrate the gap"
        );
        assert!(
            i64::try_from(second.parsed).unwrap() > 0,
            "parsed {} while writing 0 new facts is the whole point",
            second.parsed
        );
    }

    // ── DECLARED INGEST: refusing a partial load (aegis-j0yaxj.2) ────────────

    const DECLARED_GRAPH: &str = "http://example.org/graphs/watdiv";

    fn declared_store() -> (Store, i64) {
        let store = Store::open_in_memory().unwrap();
        let g = store.graph_create(DECLARED_GRAPH).unwrap();
        (store, g)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::new().chain_update(bytes).finalize())
    }

    /// Counts the completion markers a declared ingest writes about its own graph.
    fn completion_markers(store: &Store, g: i64) -> i64 {
        let subject = store.lookup(DECLARED_GRAPH).unwrap().unwrap();
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM facts f JOIN terms t ON t.id = f.a \
                 WHERE f.e = ?1 AND f.g = ?2 AND f.op = 1 AND f.valid_to IS NULL \
                   AND t.iri LIKE 'urn:quipu:ingest:%'",
                rusqlite::params![subject, g],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// The happy path, and the guarantee that makes the marker trustworthy: the
    /// completion assertions are in the SAME transaction as the last chunk.
    #[test]
    fn a_met_declaration_marks_the_graph_complete_in_the_final_chunks_tx() {
        let (mut store, g) = declared_store();
        let bytes = TURTLE_DATA.as_bytes();

        // Chunk 1 is the adversarial size: it maximises the number of commits and
        // so the number of chances to close the tx before the markers are ready.
        let report = ingest_rdf_declared(
            &mut store,
            bytes,
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            g,
            1,
            &LoadDeclaration {
                triples: turtle_triple_count(),
                sha256: sha256_hex(bytes),
            },
        )
        .unwrap();

        assert_eq!(completion_markers(&store, g), 3, "expected all three markers");

        let last_tx = *report.tx_ids.last().unwrap();
        let data_in_last_tx: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM facts f JOIN terms t ON t.id = f.a \
                 WHERE f.tx = ?1 AND t.iri NOT LIKE 'urn:quipu:ingest:%'",
                rusqlite::params![last_tx],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            data_in_last_tx > 0,
            "the completion markers landed in a transaction of their own -- there is \
             then a window in which the store says complete while data is still arriving"
        );
    }

    /// THE ONE THAT MATTERS. A short load is refused, the partial graph is LEFT
    /// VISIBLE, and it carries NO completion marker.
    ///
    /// malcolm, on this bead: the partial graph is the dangerous one, and note which
    /// way it fails -- 700M has better latency than 1B, so truncation makes the
    /// numbers look BETTER. A published good-looking result is not investigated.
    #[test]
    fn a_short_load_is_refused_and_leaves_an_unmarked_partial_graph() {
        // Both chunk regimes, because they exercise different code. At 2 the tail is
        // one of several commits; at 1000 the WHOLE load is the tail, so this arm is
        // the one that fails if a refusal discards the batch it is holding. Without
        // it the assertion below passes on the strength of the earlier chunks and
        // pins nothing -- measured: dropping the tail commit left every arm green.
        for chunk in [2usize, 1000] {
            let (mut store, g) = declared_store();
            let bytes = TURTLE_DATA.as_bytes();

            let err = ingest_rdf_declared(
                &mut store,
                bytes,
                RdfFormat::Turtle,
                None,
                "2026-04-04T00:00:00Z",
                Some("test"),
                None,
                g,
                chunk,
                // The declaration claims more than the source holds: the load is short.
                &LoadDeclaration {
                    triples: turtle_triple_count() + 1,
                    sha256: sha256_hex(bytes),
                },
            )
            .unwrap_err();

            let msg = err.to_string();
            assert!(msg.contains("declaration unmet"), "unhelpful refusal: {msg}");
            assert_eq!(
                completion_markers(&store, g),
                0,
                "chunk={chunk}: a refused load marked its graph complete"
            );

            // EVERYTHING PARSED IS STILL THERE. Not merely "something landed": a
            // vanished failed load and a load that never ran are the same
            // observation, and half a failed load is worse than either.
            let landed: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM facts WHERE g = ?1 AND op = 1",
                    rusqlite::params![g],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                landed,
                i64::try_from(turtle_triple_count()).unwrap(),
                "chunk={chunk}: the refused load did not leave its partial graph in place"
            );
        }
    }


    /// The same bytes under a different pin is also refused -- a re-derivable result
    /// needs the source identified, not merely counted.
    #[test]
    fn a_wrong_source_digest_is_refused_even_when_the_count_matches() {
        let (mut store, g) = declared_store();
        let err = ingest_rdf_declared(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            g,
            4,
            &LoadDeclaration {
                triples: turtle_triple_count(),
                sha256: "0".repeat(64),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("sha256"), "{err}");
        assert_eq!(completion_markers(&store, g), 0);
    }

    /// ROOT is refused: a declaration describes one load window, and ROOT is the
    /// store -- its triple count is not the dataset's, so the check would be
    /// meaningless in the direction that passes.
    #[test]
    fn a_declared_ingest_refuses_root() {
        let mut store = Store::open_in_memory().unwrap();
        let err = ingest_rdf_declared(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            crate::schema::ROOT_GRAPH,
            4,
            &LoadDeclaration {
                triples: turtle_triple_count(),
                sha256: sha256_hex(TURTLE_DATA.as_bytes()),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("ROOT"), "{err}");
    }

    /// ROOT BY ITS IRI IS STILL ROOT, and the id check above cannot see it.
    ///
    /// `graph_create("urn:quipu:graph:root")` interns a NEW named graph with a
    /// nonzero id, so `a_declared_ingest_refuses_root` passes while this exact
    /// call sails through. malcolm measured the consequence on 2026-09-05:
    /// 641,803 facts in a named graph and a ROOT query returning 0, which reads
    /// as a successful load of nothing.
    ///
    /// Asserts on the STORE as well as the error, because "refused" is a claim
    /// about what was written: a refusal that still staged the data would satisfy
    /// an error-only assertion.
    #[test]
    fn a_declared_ingest_refuses_root_by_its_iri_too() {
        let mut store = Store::open_in_memory().unwrap();
        let graph = store.graph_create(crate::schema::ROOT_GRAPH_IRI).unwrap();
        assert_ne!(
            graph,
            crate::schema::ROOT_GRAPH,
            "precondition: the root IRI must intern as a NON-root id, otherwise this \
             test is exercising the id check and not the IRI one"
        );
        let before = live_fact_count(&store);
        let err = ingest_rdf_declared(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            graph,
            4,
            &LoadDeclaration {
                triples: turtle_triple_count(),
                sha256: sha256_hex(TURTLE_DATA.as_bytes()),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("ROOT"), "{err}");
        assert_eq!(
            live_fact_count(&store),
            before,
            "a refusal must write nothing"
        );
    }

    /// The declared count is what the FIXTURE holds, measured rather than written
    /// down: a hardcoded number would make every arm above pass or fail for a
    /// reason that has nothing to do with the declaration logic.
    fn turtle_triple_count() -> usize {
        let mut store = Store::open_in_memory().unwrap();
        ingest_rdf_chunked(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            None,
            0,
            1000,
        )
        .unwrap()
        .parsed
    }

}

/// aegis-fmyi acceptance: RDF term fidelity across a full ingest → store →
/// export round trip.
///
/// These assert the OBSERVED EFFECT of a round trip through the real store, not
/// the behavior of `literal_to_value` in isolation — the defect was that a term
/// survived parsing looking plausible and came back out wrong.
#[cfg(test)]
mod term_fidelity_tests {
    use super::*;
    use crate::store::Store;

    const FIDELITY_TTL: &str = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:s ex:greeting "hello"@en ;
     ex:lookalike "hello@en" ;
     ex:when "2026-07-15"^^xsd:date ;
     ex:exact "0.1"^^xsd:decimal ;
     ex:approx "0.1"^^xsd:double ;
     ex:custom "abc"^^ex:MyType .
"#;

    fn ingested() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        ingest_rdf(
            &mut store,
            FIDELITY_TTL.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-07-15T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    fn value_of(store: &Store, pred: &str) -> Value {
        let s = store.lookup("http://example.org/s").unwrap().unwrap();
        let p = store
            .lookup(&format!("http://example.org/{pred}"))
            .unwrap()
            .unwrap();
        store
            .entity_facts(s)
            .unwrap()
            .into_iter()
            .find(|f| f.attribute == p)
            .unwrap_or_else(|| panic!("no fact for ex:{pred}"))
            .value
    }

    /// The headline criterion: the lexical value is "hello", NOT "hello@en".
    #[test]
    fn lang_tag_is_not_mangled_into_the_lexical_value() {
        let store = ingested();
        let v = value_of(&store, "greeting");
        assert_eq!(
            v,
            Value::Lang {
                lexical: "hello".into(),
                lang: "en".into()
            }
        );
        // Stated separately because THIS is the consumer-visible harm: anyone
        // asking for the value got the tag glued onto it.
        assert_eq!(v.as_lexical(), Some("hello"));
        assert_eq!(v.language(), Some("en"));
    }

    /// `"hello"@en` and the plain string `hello@en` must not collapse. This is
    /// the criterion that no serializer change could ever have met: once both
    /// became `Str("hello@en")` the distinction was gone from the database.
    #[test]
    fn lang_literal_and_lookalike_string_stay_distinguishable() {
        let store = ingested();
        let tagged = value_of(&store, "greeting");
        let plain = value_of(&store, "lookalike");

        assert_ne!(tagged, plain);
        assert_eq!(plain, Value::Str("hello@en".into()));
        assert_eq!(plain.language(), None);

        // …and they stay distinct on the way back out to RDF.
        let out = String::from_utf8(export_rdf(&store, RdfFormat::NTriples).unwrap()).unwrap();
        assert!(out.contains(r#""hello"@en"#), "lang literal lost: {out}");
        assert!(out.contains(r#""hello@en""#), "plain string lost: {out}");
    }

    #[test]
    fn xsd_date_round_trips_with_its_datatype_iri() {
        let store = ingested();
        assert_eq!(
            value_of(&store, "when"),
            Value::Typed {
                lexical: "2026-07-15".into(),
                datatype: "http://www.w3.org/2001/XMLSchema#date".into()
            }
        );
        let out = String::from_utf8(export_rdf(&store, RdfFormat::NTriples).unwrap()).unwrap();
        assert!(
            out.contains(r#""2026-07-15"^^<http://www.w3.org/2001/XMLSchema#date>"#),
            "xsd:date datatype lost on export: {out}"
        );
    }

    /// xsd:decimal is exact; xsd:double is not. Collapsing both into f64 was a
    /// silent semantic change, not a formatting choice.
    #[test]
    fn decimal_and_double_stay_distinguishable() {
        let store = ingested();
        let exact = value_of(&store, "exact");
        let approx = value_of(&store, "approx");

        assert_eq!(exact.datatype(), Some(namespace::XSD_DECIMAL));
        assert_eq!(approx.datatype(), Some(namespace::XSD_DOUBLE));
        assert_ne!(exact, approx);
        // Both still read as numbers, so ORDER BY / SUM / FILTER keep working.
        assert_eq!(exact.as_f64(), Some(0.1));
        assert_eq!(approx.as_f64(), Some(0.1));
    }

    #[test]
    fn custom_datatype_survives() {
        let store = ingested();
        assert_eq!(
            value_of(&store, "custom"),
            Value::Typed {
                lexical: "abc".into(),
                datatype: "http://example.org/MyType".into()
            }
        );
    }

    /// A plain string is NEVER sniffed for a trailing "@xx" on the way out.
    /// Recovering a tag that way is the trap the bead calls out: it would
    /// manufacture `"hello"@en` from a string nobody tagged.
    #[test]
    fn plain_string_is_not_sniffed_into_a_lang_literal() {
        let mut store = Store::open_in_memory().unwrap();
        let term = value_to_term(&store, &Value::Str("hello@en".into())).unwrap();
        assert_eq!(
            term,
            OxTerm::Literal(Literal::new_simple_literal("hello@en"))
        );
        // …and not into a datatype either.
        let term = value_to_term(&store, &Value::Str("2026-07-15".into())).unwrap();
        assert_eq!(
            term,
            OxTerm::Literal(Literal::new_simple_literal("2026-07-15"))
        );
        let _ = &mut store;
    }

    /// Storage encoding: the length-delimited framing must survive a lexical
    /// form that itself contains the delimiter-ish characters.
    #[test]
    fn tagged_literals_survive_the_blob_encoding() {
        for v in [
            Value::Lang {
                lexical: "a@b@c".into(),
                lang: "en-GB".into(),
            },
            Value::Typed {
                lexical: String::new(),
                datatype: "http://example.org/T".into(),
            },
            Value::Typed {
                lexical: "^^<not an iri>".into(),
                datatype: "http://example.org/T".into(),
            },
        ] {
            assert_eq!(
                Value::from_bytes(&v.to_bytes()).unwrap(),
                v,
                "round trip: {v:?}"
            );
        }
    }

    // --- Subset export (quipu #36) ------------------------------------------

    fn store_with_named_graph() -> (Store, &'static str) {
        use crate::types::{Op, Value};
        let mut store = Store::open_in_memory().unwrap();
        let ts = "2026-04-04T00:00:00Z";
        ingest_rdf(
            &mut store,
            "@prefix ex: <http://example.org/> .\nex:root ex:p \"ROOTVAL\" .\n".as_bytes(),
            RdfFormat::Turtle,
            None,
            ts,
            None,
            None,
        )
        .unwrap();
        let g_iri = "http://example.org/g/t1";
        let g = store.overlay_create(g_iri, 0).unwrap();
        let a = store.intern("http://example.org/a").unwrap();
        let p = store.intern("http://example.org/p").unwrap();
        let y = store.intern("http://example.org/y").unwrap();
        store
            .overlay_write(g, Op::Assert, a, p, Value::Ref(y), ts)
            .unwrap();
        (store, g_iri)
    }

    #[test]
    fn export_subset_scopes_to_named_graph() {
        let (store, g_iri) = store_with_named_graph();

        // The named-graph slice: only its own triple.
        let (bytes, count) = export_rdf_subset(&store, RdfFormat::NTriples, Some(g_iri)).unwrap();
        let doc = String::from_utf8(bytes).unwrap();
        assert_eq!(count, 1);
        assert!(doc.contains("http://example.org/a"));
        assert!(doc.contains("http://example.org/y"));
        assert!(
            !doc.contains("ROOTVAL"),
            "the ROOT graph must be excluded from a named-graph subset: {doc}"
        );

        // The ROOT default (graph omitted): only ROOT.
        let (rbytes, rcount) = export_rdf_subset(&store, RdfFormat::NTriples, None).unwrap();
        let rdoc = String::from_utf8(rbytes).unwrap();
        assert_eq!(rcount, 1);
        assert!(rdoc.contains("ROOTVAL"));
        assert!(
            !rdoc.contains("http://example.org/a"),
            "the named graph must be excluded from the ROOT export"
        );
    }

    #[test]
    fn export_subset_unknown_graph_errors() {
        let (store, _g) = store_with_named_graph();
        assert!(
            export_rdf_subset(
                &store,
                RdfFormat::NTriples,
                Some("http://example.org/g/nope")
            )
            .is_err(),
            "a targeted export of a non-existent graph is an error, not an empty success"
        );
    }

    #[test]
    fn export_subset_round_trips() {
        let (store, g_iri) = store_with_named_graph();
        // Export the slice, re-ingest it into a fresh store's ROOT, get it back.
        let (bytes, _) = export_rdf_subset(&store, RdfFormat::Turtle, Some(g_iri)).unwrap();
        let mut store2 = Store::open_in_memory().unwrap();
        ingest_rdf(
            &mut store2,
            bytes.as_slice(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        let facts = store2.current_facts().unwrap();
        assert_eq!(
            facts.len(),
            1,
            "the exported slice re-ingests to exactly its triples"
        );
    }

}
