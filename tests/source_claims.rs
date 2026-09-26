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
