//! Tests for the render-ready graph projection.

use super::*;
use crate::rdf::ingest_rdf;
use oxrdfio::RdfFormat;

fn store_with(turtle: &str) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

const FIXTURE: &str = r#"
@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

ex:kota   a ex:Host    ; rdfs:label "Kota"    ; ex:runs ex:traefik ; ex:runs ex:forgejo .
ex:traefik a ex:Service ; rdfs:label "Traefik" ; ex:routes_to ex:forgejo .
ex:forgejo a ex:Service ; rdfs:label "Forgejo" .
ex:ep1    a prov:Activity ; rdfs:label "episode" ; ex:touched ex:kota .
"#;

fn view(store: &Store, input: &serde_json::Value) -> JsonValue {
    tool_graph_view(store, input).unwrap()
}

#[test]
fn projects_nodes_and_index_based_edges() {
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({}));

    let nodes = g["nodes"].as_array().unwrap();
    let edges = g["edges"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "kota + traefik + forgejo, episode excluded");
    assert_eq!(edges.len(), 3, "2x runs + 1x routes_to");

    // Edges address nodes by index — the payload-size win — and resolve back.
    for e in edges {
        let si = e[0].as_u64().unwrap() as usize;
        let ti = e[1].as_u64().unwrap() as usize;
        assert!(si < nodes.len() && ti < nodes.len(), "index in range");
        assert!(e[2].is_string(), "predicate carried as a short name");
    }
}

#[test]
fn episodes_are_excluded_by_default_and_available_on_request() {
    let store = store_with(FIXTURE);

    let iris = |g: &JsonValue| -> Vec<String> {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["iri"].as_str().unwrap().to_string())
            .collect()
    };

    let default = view(&store, &json!({}));
    assert!(
        !iris(&default).iter().any(|i| i.ends_with("ep1")),
        "provenance activities must not bury the domain graph"
    );

    let with_eps = view(&store, &json!({"include_episodes": true}));
    assert!(
        iris(&with_eps).iter().any(|i| i.ends_with("ep1")),
        "…but they are reachable when asked for"
    );
}

#[test]
fn scaffolding_predicates_are_not_edges() {
    // rdf:type, rdfs:* and prov:* are node attributes or provenance, never
    // drawable relations — otherwise every node hangs off its class.
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({}));
    for e in g["edges"].as_array().unwrap() {
        let pred = e[2].as_str().unwrap();
        assert!(
            pred == "runs" || pred == "routes_to",
            "unexpected edge predicate: {pred}"
        );
    }
}

#[test]
fn labels_prefer_rdfs_label_and_fall_back_to_short_iri() {
    let store = store_with(
        r#"@prefix ex: <http://example.org/> .
           @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           ex:a a ex:T ; rdfs:label "Pretty" ; ex:rel ex:b .
           ex:b a ex:T ."#,
    );
    let g = view(&store, &json!({}));
    let by_iri: std::collections::HashMap<&str, &str> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["iri"].as_str().unwrap(), n["label"].as_str().unwrap()))
        .collect();
    assert_eq!(by_iri["http://example.org/a"], "Pretty");
    assert_eq!(
        by_iri["http://example.org/b"], "b",
        "no label -> short IRI, never an empty node"
    );
}

#[test]
fn degree_ranks_the_cap_and_truncation_is_reported() {
    // kota has degree 2, traefik 2, forgejo 2 — cap to 1 and the survivor must
    // be a highest-degree node, with the drop stated rather than silent.
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({"limit": 1}));

    assert_eq!(g["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(g["truncated"]["shown"], 1);
    assert_eq!(
        g["truncated"]["of"], 3,
        "the caller is told what was dropped (no silent caps)"
    );
}

#[test]
fn edges_to_dropped_nodes_are_omitted() {
    // An edge whose endpoint did not survive the cap would render as an arrow
    // into empty space.
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({"limit": 1}));
    assert!(
        g["edges"].as_array().unwrap().is_empty(),
        "a single surviving node has no drawable edge"
    );
}

#[test]
fn type_census_is_ordered_by_prevalence() {
    // The UI assigns its fixed eight-slot palette by this rank, so the order
    // has to be by count (descending), deterministic on ties.
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({}));
    let types = g["types"].as_array().unwrap();
    assert_eq!(types[0]["label"], "Service", "2 services outrank 1 host");
    assert_eq!(types[0]["count"], 2);
    assert_eq!(types[1]["label"], "Host");
}

#[test]
fn type_filter_scopes_both_nodes_and_edges() {
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({"type": "http://example.org/Service"}));
    let nodes = g["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2, "traefik + forgejo");
    assert_eq!(
        g["edges"].as_array().unwrap().len(),
        1,
        "only routes_to survives; kota's runs edges leave the filtered set"
    );
}

#[test]
fn ordering_is_stable_across_runs() {
    // Same store must render the same graph — otherwise the layout reshuffles
    // on every reload for no reason.
    let store = store_with(FIXTURE);
    let a = view(&store, &json!({}));
    let b = view(&store, &json!({}));
    assert_eq!(a["nodes"], b["nodes"]);
    assert_eq!(a["edges"], b["edges"]);
}

#[test]
fn empty_store_is_an_empty_payload_not_an_error() {
    let store = Store::open_in_memory().unwrap();
    let g = view(&store, &json!({}));
    assert!(g["nodes"].as_array().unwrap().is_empty());
    assert!(g["edges"].as_array().unwrap().is_empty());
    assert_eq!(g["truncated"]["of"], 0);
}

#[test]
fn limit_is_capped_at_the_hard_ceiling() {
    let store = store_with(FIXTURE);
    let g = view(&store, &json!({"limit": 999_999}));
    // Cannot be asked to render an unbounded graph; the fixture is small so the
    // observable effect is simply that it succeeds and stays bounded.
    assert!(g["nodes"].as_array().unwrap().len() <= MAX_LIMIT);
}
