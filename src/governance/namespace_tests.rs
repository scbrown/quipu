//! Namespace-drift tests. Size-exempt (`*_tests.rs`).

use super::*;
use crate::episode::{Episode, ingest_episode};
use crate::namespace::DEFAULT_BASE_NS;

const TS: &str = "2026-01-01T00:00:00Z";

fn episode(json: &str) -> Episode {
    serde_json::from_str(json).unwrap()
}

/// Ingest `json` as an episode at `ts`, through the real mint path.
fn ingest(store: &mut Store, ts: &str, json: &str) {
    ingest_episode(store, &episode(json), ts, DEFAULT_BASE_NS).unwrap();
}

fn shape(store: &Store, name: &str, turtle: &str) {
    store.load_shapes(name, turtle, TS).unwrap();
}

fn iri(local: &str) -> String {
    format!("{DEFAULT_BASE_NS}{local}")
}

fn found<'a>(report: &'a Report, local: &str) -> Option<&'a MintedPredicate> {
    report.ungoverned.iter().find(|p| p.local == local)
}

fn locals(report: &Report) -> Vec<&str> {
    report.ungoverned.iter().map(|p| p.local.as_str()).collect()
}

/// One node with two free-form properties and no shapes at all.
const DRIFT: &str = r#"{
    "name": "drift-episode",
    "nodes": [
        {"name": "koror", "type": "ProxmoxNode",
         "properties": {"hostname": "koror.local", "rackUnit": 7}}
    ]
}"#;

#[test]
fn an_empty_store_reports_nothing() {
    // Nothing minted, nothing to report — and the summary says it is unmeasured
    // rather than clean, because those are different states.
    let store = Store::open_in_memory().unwrap();
    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    assert!(report.ungoverned.is_empty(), "{report:#?}");
    assert_eq!(report.governed, 0);
    assert_eq!(report.subjects_scanned, 0);
    assert_eq!(report.minted(), 0);
    assert!(
        report.summary().contains("nothing has been minted"),
        "{}",
        report.summary()
    );
}

#[test]
fn episode_minted_predicates_absent_from_every_shape_are_reported() {
    let mut store = Store::open_in_memory().unwrap();
    ingest(&mut store, TS, DRIFT);

    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    assert_eq!(locals(&report), ["hostname", "rackUnit"], "{report:#?}");
    assert_eq!(report.governed, 0);
    assert_eq!(report.subjects_scanned, 1);
    assert_eq!(found(&report, "hostname").unwrap().iri, iri("hostname"));
    assert!(report.summary().contains("2 ungoverned predicate(s)"));
}

#[test]
fn a_predicate_a_loaded_shape_mentions_is_not_reported() {
    let mut store = Store::open_in_memory().unwrap();
    shape(
        &store,
        "hosts",
        &format!(
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
             @prefix aegis: <{DEFAULT_BASE_NS}> .\n\
             aegis:NodeShape a sh:NodeShape ;\n\
             \x20 sh:targetClass aegis:ProxmoxNode ;\n\
             \x20 sh:property [ sh:path aegis:hostname ; sh:minCount 0 ] .\n"
        ),
    );
    ingest(&mut store, TS, DRIFT);

    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    assert_eq!(locals(&report), ["rackUnit"], "{report:#?}");
    assert_eq!(report.governed, 1, "hostname is governed, not ungoverned");
    assert_eq!(report.minted(), 2);
    assert_eq!(report.shapes_loaded, 1);
    assert!(report.shape_terms > 0);
}

#[test]
fn counts_and_seen_window_span_every_use() {
    let mut store = Store::open_in_memory().unwrap();
    ingest(&mut store, TS, DRIFT);
    ingest(
        &mut store,
        "2026-03-04T00:00:00Z",
        r#"{
            "name": "second-episode",
            "nodes": [
                {"name": "ct-205", "type": "LXCContainer",
                 "properties": {"hostname": "ct-205.local"}},
                {"name": "ct-206", "type": "LXCContainer",
                 "properties": {"hostname": "ct-206.local"}}
            ]
        }"#,
    );

    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    let hostname = found(&report, "hostname").expect("hostname reported");
    assert_eq!(hostname.facts, 3, "one per node across both episodes");
    assert_eq!(hostname.subjects, 3);
    assert_eq!(hostname.first_seen, TS);
    assert_eq!(hostname.last_seen, "2026-03-04T00:00:00Z");

    let rack = found(&report, "rackUnit").expect("rackUnit reported");
    assert_eq!(rack.facts, 1);
    assert_eq!(rack.subjects, 1);
    assert_eq!(rack.first_seen, rack.last_seen);

    // Most-used first, so the drift worth acting on leads.
    assert_eq!(locals(&report), ["hostname", "rackUnit"]);
    assert_eq!(report.subjects_scanned, 3);
}

#[test]
fn structural_predicates_are_not_agent_drift() {
    // The episode ACTIVITY carries aegis:groupId and aegis:contentHash. Neither
    // was minted by a properties key, and reporting the writer's own vocabulary
    // would put a permanent floor under every report.
    let mut store = Store::open_in_memory().unwrap();
    ingest(
        &mut store,
        TS,
        r#"{
            "name": "grouped",
            "group_id": "aegis-ontology",
            "nodes": [{"name": "koror", "type": "ProxmoxNode"}]
        }"#,
    );

    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    assert!(locals(&report).is_empty(), "{report:#?}");
    assert_eq!(report.subjects_scanned, 1, "the node was still scanned");
}

#[test]
fn an_edge_relation_is_not_reported_as_a_minted_property() {
    // Edge relations already pass through `resolve_edge_predicate`, which is a
    // fence. They resolve to node REFERENCES, and this report is about the
    // unfenced literal path.
    let mut store = Store::open_in_memory().unwrap();
    ingest(
        &mut store,
        TS,
        r#"{
            "name": "edges",
            "nodes": [
                {"name": "koror", "type": "ProxmoxNode"},
                {"name": "ct-205", "type": "LXCContainer"}
            ],
            "edges": [{"source": "ct-205", "target": "koror", "relation": "runs_on"}]
        }"#,
    );

    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    assert!(
        !locals(&report).contains(&"runs_on"),
        "edge relation reported as a minted property: {report:#?}"
    );
}

#[test]
fn facts_no_episode_wrote_are_out_of_scope() {
    // A predicate written straight to the fact log is not episode drift. It may
    // be ungoverned, but this report answers a narrower question and must not
    // quietly widen it.
    use crate::store::Datum;
    use crate::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let entity = store.intern("http://example.org/hand-written").unwrap();
    let attribute = store.intern(&iri("handMinted")).unwrap();
    store
        .transact(
            &[Datum {
                entity,
                attribute,
                value: Value::Str("value".into()),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            None,
            None,
        )
        .unwrap();

    let report = check(&store, DEFAULT_BASE_NS, None).unwrap();
    assert!(locals(&report).is_empty(), "{report:#?}");
    assert_eq!(report.subjects_scanned, 0);
}

#[test]
fn a_predicate_outside_the_base_namespace_is_out_of_scope() {
    // Only the base namespace can be minted by `sanitize_iri_local`. Scanning
    // with a DIFFERENT base namespace configured must therefore find nothing,
    // which is the check that the base-namespace filter is real.
    let mut store = Store::open_in_memory().unwrap();
    ingest(&mut store, TS, DRIFT);

    let report = check(&store, "https://other.example/ontology/", None).unwrap();
    assert!(locals(&report).is_empty(), "{report:#?}");
    assert_eq!(
        report.subjects_scanned, 0,
        "episode activities are named in the base namespace too"
    );
}

#[test]
fn an_unknown_graph_is_an_error_not_an_empty_report() {
    let store = Store::open_in_memory().unwrap();
    let err = check(&store, DEFAULT_BASE_NS, Some("urn:quipu:graph:nope")).unwrap_err();
    assert!(format!("{err}").contains("no such graph"), "{err}");
}

#[test]
fn the_root_iri_names_the_root_graph() {
    let mut store = Store::open_in_memory().unwrap();
    ingest(&mut store, TS, DRIFT);

    let named = check(&store, DEFAULT_BASE_NS, Some(crate::schema::ROOT_GRAPH_IRI)).unwrap();
    assert_eq!(locals(&named), ["hostname", "rackUnit"]);
    assert_eq!(named.graph, crate::schema::ROOT_GRAPH_IRI);
}
