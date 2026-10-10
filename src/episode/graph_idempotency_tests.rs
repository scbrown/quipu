//! Idempotency for GRAPH-SCOPED episodes (aegis-z1i5on).
//!
//! The content-hash lookup read only the default graph, so an episode written
//! into a named graph never found its own hash: every byte-identical re-post
//! was a full write reported `created`, and each one left another
//! `prov:generatedAtTime` on the activity (measured in production: 160,247
//! values on 25,213 activities in one plane).

use crate::{tool_episode, tool_query};
use serde_json::{Value, json};

use super::*;

const GRAPH: &str = "https://example.org/plane/records";
const EP: &str = "http://aegis.gastown.local/ontology/episode_graph-scoped-ep";

fn body(comment: &str, graph: Option<&str>) -> Value {
    let mut b = json!({
        "name": "graph-scoped-ep",
        "episode_body": comment,
        "source": "unit-test",
        "nodes": [{"name": "gs-item", "type": "Thing"}]
    });
    if let Some(g) = graph {
        b["graph"] = json!(g);
    }
    b
}

fn post(store: &mut Store, b: &Value) -> Value {
    tool_episode(store, b).unwrap()
}

fn count(store: &Store, pattern: &str, graph: Option<&str>) -> u64 {
    let inner = match graph {
        Some(g) => format!("GRAPH <{g}> {{ {pattern} }}"),
        None => pattern.to_string(),
    };
    let q = format!("SELECT (COUNT(*) AS ?n) WHERE {{ {inner} }}");
    let r = tool_query(store, &json!({"query": q})).unwrap();
    r["rows"][0]["n"].as_u64().unwrap()
}

fn generated_at(store: &Store, graph: Option<&str>) -> u64 {
    count(
        store,
        &format!("<{EP}> <{}generatedAtTime> ?t", namespace::PROV),
        graph,
    )
}

#[test]
fn an_identical_graph_scoped_repost_is_unchanged() {
    let mut store = Store::open_in_memory().unwrap();
    let first = post(&mut store, &body("v1", Some(GRAPH)));
    assert_eq!(first["outcome"], "created", "{first}");
    let again = post(&mut store, &body("v1", Some(GRAPH)));
    assert_eq!(again["outcome"], "unchanged", "{again}");
    assert_eq!(again["tx_id"], 0, "{again}");
    assert_eq!(generated_at(&store, Some(GRAPH)), 1);
}

#[test]
fn a_changed_graph_scoped_episode_replaces_its_activity_facts() {
    let mut store = Store::open_in_memory().unwrap();
    post(&mut store, &body("v1", Some(GRAPH)));
    // A different second, so a stale timestamp would be a distinct value.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let changed = post(&mut store, &body("v2", Some(GRAPH)));
    assert_eq!(changed["outcome"], "updated", "{changed}");
    assert_eq!(
        generated_at(&store, Some(GRAPH)),
        1,
        "stale generatedAtTime kept"
    );
    let hash = format!("<{EP}> <{}contentHash> ?h", namespace::DEFAULT_BASE_NS);
    assert_eq!(
        count(&store, &hash, Some(GRAPH)),
        1,
        "stale contentHash kept"
    );
    let comment = format!("<{EP}> <{}comment> ?c", namespace::RDFS);
    assert_eq!(
        count(&store, &comment, Some(GRAPH)),
        1,
        "stale comment kept"
    );
}

#[test]
fn the_graph_scoped_episode_never_writes_into_root() {
    let mut store = Store::open_in_memory().unwrap();
    post(&mut store, &body("v1", Some(GRAPH)));
    post(&mut store, &body("v2", Some(GRAPH)));
    assert_eq!(generated_at(&store, None), 0);
}

#[test]
fn root_episodes_keep_their_behaviour() {
    let mut store = Store::open_in_memory().unwrap();
    assert_eq!(post(&mut store, &body("v1", None))["outcome"], "created");
    assert_eq!(post(&mut store, &body("v1", None))["outcome"], "unchanged");
    assert_eq!(post(&mut store, &body("v2", None))["outcome"], "updated");
    assert_eq!(generated_at(&store, None), 1);
}

#[test]
fn the_same_episode_in_root_does_not_satisfy_the_graph_lookup() {
    // Hash present in ROOT only: a graph-scoped post of the same body must
    // still write into its graph, not short-circuit on ROOT's hash.
    let mut store = Store::open_in_memory().unwrap();
    post(&mut store, &body("v1", None));
    let scoped = post(&mut store, &body("v1", Some(GRAPH)));
    assert_ne!(scoped["outcome"], "unchanged", "{scoped}");
    assert_eq!(generated_at(&store, Some(GRAPH)), 1);
}
