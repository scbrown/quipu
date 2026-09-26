//! Independent source ownership through the public knot/repair contract.
use quipu::{Store, tool_knot, tool_retract_source};
use serde_json::json;

#[cfg(feature = "reactive-reasoner")]
#[derive(Default)]
struct Capture(std::sync::Mutex<Vec<(usize, usize)>>);
#[cfg(feature = "reactive-reasoner")]
impl quipu::store::TransactObserver for Capture {
    fn after_commit(&self, _store: &mut Store, delta: &quipu::store::Delta) -> quipu::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((delta.asserts.len(), delta.retracts.len()));
        Ok(())
    }
}

const TRIPLE: &str =
    "<urn:test:s> <http://www.w3.org/2000/01/rdf-schema#label> \"overlap control\" .";

fn seed(store: &mut Store, source: &str, graph: &str) {
    tool_knot(
        store,
        &json!({"turtle":TRIPLE,"source":source,"graph": (graph != "urn:quipu:graph:root").then_some(graph)}),
    )
    .unwrap();
}
fn retract(store: &mut Store, source: &str, graph: &str) {
    let request = json!({"source":source,"graph": (graph != "urn:quipu:graph:root").then_some(graph),"repair":"test"});
    let plan = tool_retract_source(store, &request).unwrap();
    assert_eq!(plan["planned"], 1, "each source must own a claim");
    let result = tool_retract_source(
        store,
        &json!({"source":source,"graph": (graph != "urn:quipu:graph:root").then_some(graph),"repair":"test","apply":true,"expect":1}),
    )
    .unwrap();
    assert_eq!(result["remaining"], 0);
}
fn visible(store: &Store, graph: &str, expected: usize) {
    let body = if graph == "urn:quipu:graph:root" {
        "<urn:test:s> ?p ?o".to_owned()
    } else {
        format!("GRAPH <{graph}> {{ <urn:test:s> ?p ?o }}")
    };
    assert_eq!(
        quipu::sparql::query(store, &format!("SELECT ?p ?o WHERE {{ {body} }}"))
            .unwrap()
            .rows()
            .len(),
        expected
    );
}

#[test]
fn source_claims_both_orders_graph_isolation_and_restart() {
    for order in [["A", "B"], ["B", "A"]] {
        for graph in ["urn:quipu:graph:root", "urn:test:named"] {
            for model in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("claims.db");
                let mut store = Store::open(path.to_str().unwrap()).unwrap();
                store.set_read_model_enabled(model);
                let named = store.graph_create("urn:test:named").unwrap();
                let other = if graph == "urn:quipu:graph:root" {
                    "urn:test:named"
                } else {
                    "urn:quipu:graph:root"
                };
                let g = if graph == "urn:quipu:graph:root" {
                    0
                } else {
                    named
                };
                seed(&mut store, "control", other);
                seed(&mut store, "A", graph);
                let first_event = store.latest_event_offset().unwrap();
                #[cfg(feature = "reactive-reasoner")]
                let capture = std::sync::Arc::new(Capture::default());
                #[cfg(feature = "reactive-reasoner")]
                store.add_observer(capture.clone());
                seed(&mut store, "B", graph);
                seed(&mut store, "B", graph);
                assert_eq!(store.latest_event_offset().unwrap(), first_event);
                let conn = rusqlite::Connection::open(&path).unwrap();
                let claims: i64 = conn.query_row("SELECT COUNT(*) FROM facts f JOIN transactions t ON f.tx=t.id WHERE f.op=1 AND f.valid_to IS NULL AND f.g=?1 AND t.source IN ('A','B')", [g], |r| r.get(0)).unwrap();
                assert_eq!(
                    claims, 2,
                    "same-source reload is idempotent; distinct sources persist"
                );
                let reader = Store::open_read_only(path.to_str().unwrap()).unwrap();
                assert_eq!(reader.read_model_for(g).unwrap().len(), 1);
                assert_eq!(store.read_model_for(g).unwrap().len(), 1);
                visible(&store, graph, 1);
                retract(&mut store, order[0], graph);
                assert_eq!(store.latest_event_offset().unwrap(), first_event);
                visible(&store, graph, 1);
                visible(&store, other, 1);
                assert_eq!(reader.read_model_for(g).unwrap().len(), 1);
                assert_eq!(store.read_model_for(g).unwrap().len(), 1);
                let active: i64 = conn.query_row("SELECT COUNT(*) FROM facts f JOIN transactions t ON f.tx=t.id WHERE f.op=1 AND f.valid_to IS NULL AND f.g=?1 AND t.source=?2", rusqlite::params![g,order[1]], |r| r.get(0)).unwrap();
                assert_eq!(active, 1);
                #[cfg(feature = "reactive-reasoner")]
                assert!(
                    capture
                        .0
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|delta| *delta == (0, 0))
                );
                drop(store);
                let mut store = Store::open(path.to_str().unwrap()).unwrap();
                visible(&store, graph, 1);
                retract(&mut store, order[1], graph);
                visible(&store, graph, 0);
                visible(&store, other, 1);
                assert_eq!(reader.read_model_for(g).unwrap().len(), 0);
                if g == 0 {
                    assert!(store.latest_event_offset().unwrap() > first_event);
                }
                let history: i64 = conn.query_row("SELECT COUNT(*) FROM facts f JOIN transactions t ON f.tx=t.id WHERE f.op=1 AND f.g=?1 AND t.source IN ('A','B') AND f.retracted_tx IS NOT NULL", [g], |r| r.get(0)).unwrap();
                assert_eq!(history, 2, "both historical assertions remain auditable");
            }
        }
    }
}

#[test]
fn source_claims_snapshot_replacement_preserves_coowner() {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, "B", "urn:quipu:graph:root");
    tool_knot(
        &mut store,
        &json!({"turtle":TRIPLE,"snapshot":"A","replace_snapshot":true}),
    )
    .unwrap();
    tool_knot(
        &mut store,
        &json!({"turtle":"","snapshot":"A","replace_snapshot":true}),
    )
    .unwrap();
    visible(&store, "urn:quipu:graph:root", 1);
    retract(&mut store, "B", "urn:quipu:graph:root");
    visible(&store, "urn:quipu:graph:root", 0);
}

#[test]
fn source_claims_all_statement_readers_deduplicate() {
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "<urn:test:s> <urn:test:p> \"v\"; a <urn:test:Child> . <urn:test:Child> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <urn:test:Parent> .";
    for source in ["A", "B"] {
        tool_knot(&mut store, &json!({"source":source,"turtle":ttl})).unwrap();
    }
    let e = store.lookup("urn:test:s").unwrap().unwrap();
    let a = store.lookup("urn:test:p").unwrap().unwrap();
    assert_eq!(store.entity_facts(e).unwrap().len(), 2);
    assert_eq!(
        store.entity_history(e).unwrap().len(),
        4,
        "audit retains both claims"
    );
    assert_eq!(
        store
            .current_facts_for_attributes_in_graph(&[a], 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .current_facts_for_attributes_and_entities_in_graphs(&[a], &[e], &[0])
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .current_facts_for_entities_in_graphs(&[e], &[0])
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .current_facts_for_attributes_in_graphs_excluding_sources(&[a], &[0], &[])
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .current_facts_for_attributes_in_graphs_excluding_sources(&[a], &[0], &["A".into()])
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .current_facts_for_attributes_in_graphs_excluding_sources(
                &[a],
                &[0],
                &["A".into(), "B".into()]
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .facts_as_of(&quipu::store::AsOf {
                tx: Some(store.latest_tx_id().unwrap()),
                valid_at: None
            })
            .unwrap()
            .len(),
        3
    );
    for enabled in [false, true] {
        store.set_read_model_enabled(enabled);
        for query in [
            "SELECT (COUNT(*) AS ?n) WHERE { <urn:test:s> <urn:test:p> ?v }",
            "SELECT (COUNT(*) AS ?n) WHERE { ?s a <urn:test:Parent> }",
        ] {
            let result = quipu::sparql::query(&store, query).unwrap();
            assert_eq!(
                result.rows()[0].get("n"),
                Some(&quipu::Value::Int(1)),
                "{query}"
            );
        }
    }
    #[cfg(feature = "shacl")]
    {
        let shapes = "@prefix sh: <http://www.w3.org/ns/shacl#> . <urn:test:Shape> a sh:NodeShape; sh:targetNode <urn:test:s>; sh:property [ sh:path <urn:test:p>; sh:maxCount 1 ] .";
        let data =
            String::from_utf8(quipu::export_rdf(&store, oxrdfio::RdfFormat::Turtle).unwrap())
                .unwrap();
        assert!(quipu::validate_shapes(shapes, &data).unwrap().conforms);
        let invalid = format!("{data}\n<urn:test:s> <urn:test:p> \"second\" .");
        assert!(!quipu::validate_shapes(shapes, &invalid).unwrap().conforms);
    }
}

#[test]
fn source_claims_as_of_retract_reassert_transaction_boundary() {
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("urn:test:s").unwrap();
    let a = store.intern("urn:test:p").unwrap();
    let datum = quipu::Datum {
        entity: e,
        attribute: a,
        value: quipu::Value::Str("v".into()),
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: quipu::Op::Assert,
    };
    let first = store
        .transact(std::slice::from_ref(&datum), "2026-01-01", None, Some("A"))
        .unwrap();
    let mut retract = datum.clone();
    retract.op = quipu::Op::Retract;
    let replacement = store
        .transact(&[retract.clone(), datum], "2026-01-02", None, Some("A"))
        .unwrap();
    let final_tx = store
        .transact(&[retract], "2026-01-03", None, Some("A"))
        .unwrap();
    for (tx, count) in [(first, 1), (replacement, 1), (final_tx, 0)] {
        assert_eq!(
            store
                .facts_as_of(&quipu::store::AsOf {
                    tx: Some(tx),
                    valid_at: None
                })
                .unwrap()
                .len(),
            count
        );
        let context = quipu::sparql::TemporalContext {
            as_of_tx: Some(tx),
            ..Default::default()
        };
        assert_eq!(
            quipu::sparql::query_temporal(
                &store,
                "SELECT ?v WHERE { <urn:test:s> <urn:test:p> ?v }",
                &context
            )
            .unwrap()
            .rows()
            .len(),
            count
        );
    }
    assert_eq!(
        store
            .entity_history(e)
            .unwrap()
            .iter()
            .filter(|f| f.op == quipu::Op::Assert)
            .count(),
        2
    );
}

#[test]
fn source_claims_anonymous_and_named_are_independent_in_both_orders() {
    for sources in [[Some("A"), None], [None, Some("A")]] {
        let mut store = Store::open_in_memory().unwrap();
        let e = store.intern("urn:test:s").unwrap();
        let a = store
            .intern("http://www.w3.org/2000/01/rdf-schema#label")
            .unwrap();
        let datum = quipu::Datum {
            entity: e,
            attribute: a,
            value: quipu::Value::Str("overlap control".into()),
            valid_from: "2026-01-01".into(),
            valid_to: None,
            op: quipu::Op::Assert,
        };
        for source in sources {
            store
                .transact(std::slice::from_ref(&datum), "2026-01-01", None, source)
                .unwrap();
        }
        store
            .transact(std::slice::from_ref(&datum), "2026-01-01", None, None)
            .unwrap();
        assert_eq!(store.entity_history(e).unwrap().len(), 2);
        retract(&mut store, "A", "urn:quipu:graph:root");
        visible(&store, "urn:quipu:graph:root", 1);
        let mut retract = datum;
        retract.op = quipu::Op::Retract;
        store
            .transact(&[retract], "2026-01-02", None, None)
            .unwrap();
        visible(&store, "urn:quipu:graph:root", 0);
    }
}
