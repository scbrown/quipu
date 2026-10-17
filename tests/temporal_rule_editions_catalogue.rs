//! Synthetic snapshot-selection controls. These do not attest source history
//! or ingest a new vocabulary into any running service.

use quipu::reasoner::editions::{SelectionError, select_from_catalogue_snapshot};
use sha2::{Digest, Sha256};

const PACKAGE: &str = "https://example.org/editions/package";
const TRANSITION: &str = "2008-06-19T00:00:00Z";
const PREFIXES: &str = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <https://example.org/editions/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix time: <http://www.w3.org/2006/time#> .
@prefix spdx: <http://spdx.org/rdf/terms#> .
ex:package a rule:RuleSet ; rdfs:label "Synthetic catalogue" .
"#;

fn content(name: &str, premise: &str) -> String {
    format!(
        r#"@prefix rule: <http://quipu.local/rule#> .
        <https://example.org/rules/{name}> a rule:Rule ; rule:id "{name}" ;
        rule:prefix "https://example.org/" ; rule:head "expiry(?x, ?d)" ;
        rule:body "{premise}(?x, ?d)" ."#
    )
}

fn edition(name: &str, begins: &str, ends: Option<&str>, rules: &str) -> String {
    let serialized = oxrdf::Literal::new_simple_literal(rules).to_string();
    let hash = format!("{:x}", Sha256::digest(rules.as_bytes()));
    let end = ends.map_or_else(String::new, |date| {
        format!(
            "ex:{name}-end a time:Instant ; time:inXSDDateTimeStamp \"{date}\"^^xsd:dateTimeStamp ."
        )
    });
    let link = ends.map_or_else(String::new, |_| format!("; time:hasEnd ex:{name}-end"));
    format!(
        r#"
ex:{name} a rule:RuleEdition ; dct:isVersionOf ex:package ;
  prov:value {serialized} ; dct:source <https://example.org/synthetic-source> ;
  time:hasTime ex:{name}-interval ; spdx:checksum ex:{name}-checksum .
ex:{name}-interval a time:Interval ; time:hasBeginning ex:{name}-begin {link} .
ex:{name}-begin a time:Instant ; time:inXSDDateTimeStamp "{begins}"^^xsd:dateTimeStamp .
{end}
ex:{name}-checksum a spdx:Checksum ; spdx:algorithm spdx:checksumAlgorithm_sha256 ;
  spdx:checksumValue "{hash}" .
"#
    )
}

fn catalogue() -> String {
    format!(
        "{PREFIXES}{}{}",
        edition(
            "old",
            "1900-01-01T00:00:00Z",
            Some(TRANSITION),
            &content("old", "oldDeadline")
        ),
        edition(
            "transition",
            TRANSITION,
            None,
            &content("transition", "transitionDeadline")
        )
    )
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn governed_history_integrates_admission_catalogue_clocks_and_premise_scope() {
    use quipu::{
        config::QuipuConfig,
        reasoner::{governed::*, history_attestation::*, raw_history::*},
        signing::{public_key_hex, sign_hex},
        store::{Datum, Store},
        types::{Op, Value},
    };
    use ring::{rand::SystemRandom, signature::Ed25519KeyPair};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("raw.db");
    let mut store = Store::open(path.to_str().unwrap()).unwrap();
    quipu::rdf::ingest_rdf(
        &mut store,
        catalogue().as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2000-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let graph = store.graph_create("urn:test:premises").unwrap();
    let datums: Vec<_> = [("oldDeadline", "old"), ("transitionDeadline", "transition")]
        .into_iter()
        .map(|(predicate, object)| Datum {
            entity: store.intern("https://example.org/claim").unwrap(),
            attribute: store
                .intern(&format!("https://example.org/{predicate}"))
                .unwrap(),
            value: Value::Ref(
                store
                    .intern(&format!("https://example.org/{object}"))
                    .unwrap(),
            ),
            valid_from: "2000-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store
        .transact_to_graph(&datums, "2000-01-01T00:00:01Z", None, None, graph)
        .unwrap();
    let tx = store.latest_tx_id().unwrap();
    let lineage = store.store_id().unwrap();
    drop(store);
    let bytes = std::fs::read(&path).unwrap();
    let digest = |b: &[u8]| format!("{:x}", Sha256::digest(b));
    let policy_bytes = b"synthetic external reviewed grant v1";
    let evidence = b"synthetic complete retained fixture including pre-window supports";
    let key_bytes = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(key_bytes.as_ref()).unwrap();
    let public_key = public_key_hex(&key);
    let payload = HistoryAttestation {
        schema_version: 1,
        issuer_id: "synthetic-fixture-reviewer".into(),
        grant_policy_sha256: digest(policy_bytes),
        reviewed_evidence_ref: "urn:test:review:complete-fixture".into(),
        reviewed_evidence_sha256: digest(evidence),
        raw_artifact_sha256: digest(&bytes),
        artifact_kind: HistoryArtifactKind::RawSqliteSnapshot,
        provider_scope: HistoryProviderScope::NativeOnly,
        coverage: HistoryCoverage {
            source_store_id: lineage,
            catalogue_graph: quipu::schema::ROOT_GRAPH_IRI.into(),
            premise_graph: "urn:test:premises".into(),
            transaction_first: 1,
            transaction_last: tx,
            valid_time_from: -2208988800,
            valid_time_until: 4102444800,
        },
        issued_at: 0,
        expires_at: 100,
    };
    let signature = sign_hex(&key, &history_attestation_message(&payload).unwrap());
    let envelope = SignedHistoryAttestation { payload, signature };
    let grant = HistoryCoverageGrant {
        issuer_id: "synthetic-fixture-reviewer",
        public_key_hex: &public_key,
        policy_sha256: &envelope.payload.grant_policy_sha256,
        policy_bytes,
        reviewed_evidence_ref: &envelope.payload.reviewed_evidence_ref,
        reviewed_evidence_sha256: &envelope.payload.reviewed_evidence_sha256,
        raw_artifact_sha256: &envelope.payload.raw_artifact_sha256,
        coverage: &envelope.payload.coverage,
    };
    let verified = verify_history_attestation(&envelope, &grant, Some(evidence), 50).unwrap();
    let mut policy = Store::open_in_memory().unwrap();
    for (date, expected) in [
        ("2007-01-01T00:00:00Z", "old"),
        ("2014-01-01T00:00:00Z", "transition"),
    ] {
        let source = admit_raw_history(
            &bytes,
            &verified,
            RawHistoryRequest {
                valid_at: quipu::time::epoch_of_rfc3339(date).unwrap(),
                as_of_tx: tx,
            },
            &QuipuConfig::default(),
            &policy,
            50,
        )
        .unwrap();
        let answer = evaluate_governed_history(&source, PACKAGE).unwrap();
        if std::env::var_os("QUIPU_HISTORY_DEMO").is_some() {
            println!("DEMO {date}: {}", serde_json::to_string(&answer).unwrap());
        }
        assert_eq!(
            answer.selected_edition.edition,
            format!("https://example.org/editions/{expected}")
        );
        assert_eq!(answer.facts.len(), 1);
        assert_eq!(
            answer.facts[0].object,
            format!("https://example.org/{expected}")
        );
        assert_eq!(answer.as_of_tx, tx);
        assert_eq!(answer.source.raw_artifact_sha256, digest(&bytes));
        assert!(matches!(
            evaluate_governed_history(&source, "urn:test:missing-package"),
            Err(GovernedHistoricalError::Selection(
                SelectionError::MissingCatalogue(_)
            ))
        ));
    }
    policy.search_config_mut().max_sparql_rows = 1;
    let source = admit_raw_history(
        &bytes,
        &verified,
        RawHistoryRequest {
            valid_at: 1388534400,
            as_of_tx: tx,
        },
        &QuipuConfig::default(),
        &policy,
        50,
    )
    .unwrap();
    assert!(matches!(
        evaluate_governed_history(&source, PACKAGE),
        Err(GovernedHistoricalError::Capacity)
    ));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn temporal_rule_editions_catalogue_boundary_and_identity() {
    let catalogue = catalogue();
    for (at, expected) in [
        ("2008-06-18T23:59:59Z", "old"),
        (TRANSITION, "transition"),
        ("2008-06-19T00:00:01Z", "transition"),
        ("2008-06-18T20:00:00-04:00", "transition"),
    ] {
        let selected = select_from_catalogue_snapshot(&catalogue, PACKAGE, at).unwrap();
        assert_eq!(
            selected.edition,
            format!("https://example.org/editions/{expected}")
        );
        assert_eq!(selected.ruleset, PACKAGE);
        assert_eq!(
            selected.content_sha256,
            format!("{:x}", Sha256::digest(selected.turtle.as_bytes()))
        );
    }
    assert!(matches!(
        select_from_catalogue_snapshot(&catalogue, PACKAGE, "1899-12-31T23:59:59Z"),
        Err(SelectionError::NoMatchingEdition)
    ));
}

#[test]
fn temporal_rule_editions_catalogue_refusals_have_populated_control() {
    let catalogue = catalogue();
    assert!(select_from_catalogue_snapshot(&catalogue, PACKAGE, TRANSITION).is_ok());
    assert!(matches!(
        select_from_catalogue_snapshot(&catalogue, "https://example.org/absent", TRANSITION),
        Err(SelectionError::MissingCatalogue(_))
    ));
    assert!(matches!(
        select_from_catalogue_snapshot(PREFIXES, PACKAGE, TRANSITION),
        Err(SelectionError::EmptyCatalogue(_))
    ));
    for date in [
        "not-a-date",
        "2008-02-30T00:00:00Z",
        "2008-06-19T00:00:00.1Z",
        "9999-12-31T23:00:00-04:00",
    ] {
        assert!(matches!(
            select_from_catalogue_snapshot(&catalogue, PACKAGE, date),
            Err(SelectionError::InvalidInstant(_))
        ));
    }
    let overlapping = format!(
        "{catalogue}{}",
        edition(
            "overlap",
            "2007-01-01T00:00:00Z",
            Some("2010-01-01T00:00:00Z"),
            &content("overlap", "oldDeadline")
        )
    );
    assert!(
        matches!(select_from_catalogue_snapshot(&overlapping, PACKAGE, TRANSITION), Err(SelectionError::AmbiguousEditions(ids)) if ids == vec!["https://example.org/editions/overlap", "https://example.org/editions/transition"])
    );
    let corrupt = catalogue.replace("oldDeadline", "tamperedDeadline");
    assert!(matches!(
        select_from_catalogue_snapshot(&corrupt, PACKAGE, TRANSITION),
        Err(SelectionError::InvalidDigest(_))
    ));
    let empty = format!(
        "{PREFIXES}{}",
        edition(
            "empty",
            "1900-01-01T00:00:00Z",
            None,
            "@prefix ex: <https://example.org/> ."
        )
    );
    assert!(matches!(
        select_from_catalogue_snapshot(&empty, PACKAGE, TRANSITION),
        Err(SelectionError::InvalidRules(_))
    ));
}

#[test]
fn temporal_rule_editions_catalogue_same_facts_distinct_answers() {
    use quipu::reasoner::historical::evaluate_temporal_view;
    use quipu::sparql::TemporalContext;
    use quipu::store::{Datum, Store};
    use quipu::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let facts: Vec<_> = [
        ("oldDeadline", "deadline-old"),
        ("transitionDeadline", "deadline-transition"),
    ]
    .into_iter()
    .map(|(predicate, object)| Datum {
        entity: store.intern("https://example.org/claim").unwrap(),
        attribute: store
            .intern(&format!("https://example.org/{predicate}"))
            .unwrap(),
        value: Value::Ref(
            store
                .intern(&format!("https://example.org/{object}"))
                .unwrap(),
        ),
        valid_from: "1900-01-01T00:00:00Z".into(),
        valid_to: None,
        op: Op::Assert,
    })
    .collect();
    store
        .transact(&facts, "2000-01-01T00:00:00Z", None, None)
        .unwrap();
    let tx = store.latest_tx_id().unwrap();
    for (at, expected) in [
        ("2007-01-01T00:00:00Z", "deadline-old"),
        ("2014-01-01T00:00:00Z", "deadline-transition"),
    ] {
        let selected = select_from_catalogue_snapshot(&catalogue(), PACKAGE, at).unwrap();
        let rules = quipu::reasoner::parse_rules(&selected.turtle, None).unwrap();
        let ctx = TemporalContext {
            valid_at: Some(at.into()),
            as_of_tx: Some(tx),
            ..TemporalContext::default()
        };
        let answer = evaluate_temporal_view(&store, &rules, &ctx).unwrap();
        assert_eq!(answer.len(), 1);
        assert_eq!(answer[0].object, format!("https://example.org/{expected}"));
        assert_eq!(store.latest_tx_id().unwrap(), tx);
    }
}

#[test]
fn temporal_rule_editions_retrospective_catalogue_snapshot_is_stable() {
    use oxrdfio::{RdfFormat, RdfParser};
    use quipu::sparql::{QueryResult, TemporalContext, query_temporal};
    use quipu::store::{Datum, Store};
    use quipu::types::{Op, Value};

    fn snapshot(store: &Store, tx: i64) -> String {
        // Controlled private store, complete known fixture. Catalogue metadata
        // uses transaction visibility; applicability is selected separately.
        let context = TemporalContext {
            as_of_tx: Some(tx),
            ..TemporalContext::default()
        };
        let result =
            query_temporal(store, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", &context).unwrap();
        let QueryResult::Graph(triples) = result else {
            panic!("expected RDF graph")
        };
        assert!(triples.len() > 10); // Positive catalogue-coverage control.
        triples
            .into_iter()
            .map(|t| {
                format!(
                    "<{}> <{}> {} .\n",
                    t.subject,
                    t.predicate,
                    quipu::rdf::value_to_term(store, &t.object).unwrap()
                )
            })
            .collect()
    }

    let mut store = Store::open_in_memory().unwrap();
    quipu::rdf::ingest_rdf(
        &mut store,
        catalogue().as_bytes(),
        RdfFormat::Turtle,
        None,
        "2000-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let before = store.latest_tx_id().unwrap();
    let original_snapshot = snapshot(&store, before);
    let selected_before =
        select_from_catalogue_snapshot(&original_snapshot, PACKAGE, "2014-01-01T00:00:00Z")
            .unwrap();
    assert_eq!(
        selected_before.edition,
        "https://example.org/editions/transition"
    );
    let mut correction = vec![Datum {
        entity: store
            .intern("https://example.org/editions/transition")
            .unwrap(),
        attribute: store
            .intern("http://purl.org/dc/terms/isVersionOf")
            .unwrap(),
        value: Value::Ref(store.intern(PACKAGE).unwrap()),
        valid_from: "2000-01-01T00:00:00Z".into(),
        valid_to: None,
        op: Op::Retract,
    }];
    let updated = format!(
        "{PREFIXES}{}",
        edition(
            "corrected",
            TRANSITION,
            None,
            &content("corrected", "correctedDeadline")
        )
    );
    for quad in RdfParser::from_format(RdfFormat::Turtle).for_reader(updated.as_bytes()) {
        let triple = oxrdf::Triple::from(quad.unwrap());
        correction.push(Datum {
            entity: quipu::rdf::intern_subject(&store, &triple.subject).unwrap(),
            attribute: store.intern(triple.predicate.as_str()).unwrap(),
            value: quipu::rdf::term_to_value(&store, &triple.object).unwrap(),
            valid_from: "2020-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    store
        .transact(&correction, "2020-01-01T00:00:00Z", None, None)
        .unwrap();
    let after = store.latest_tx_id().unwrap();
    let selected_after =
        select_from_catalogue_snapshot(&snapshot(&store, after), PACKAGE, "2014-01-01T00:00:00Z")
            .unwrap();
    assert_eq!(
        selected_after.edition,
        "https://example.org/editions/corrected"
    );
    assert_ne!(
        selected_after.content_sha256,
        selected_before.content_sha256
    );
    let replay =
        select_from_catalogue_snapshot(&snapshot(&store, before), PACKAGE, "2014-01-01T00:00:00Z")
            .unwrap();
    assert_eq!(replay.edition, selected_before.edition);
    assert_eq!(replay.content_sha256, selected_before.content_sha256);
    assert_eq!(store.latest_tx_id().unwrap(), after);
}
