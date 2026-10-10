//! aegis-rcz5ib.8: anchored search on a graph built to exercise each rule.

use oxrdfio::RdfFormat;
use serde_json::json;

use super::*;
use crate::mcp::tools::search::tool_search;
use crate::vector::KnowledgeVectorStore as _;

const EX: &str = "http://example.org/";

/// a -p-> b -p-> c ; a -mentions-> d (excluded) ; a -p-> hub -p-> x0..x199
/// (hub not expanded) ; a sameAs a2 -p-> e (zero-cost alias) ; u unreachable.
/// Two entities share the label "twin".
fn store(anchored: bool) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.search_config_mut().anchored = anchored;
    let mut ttl = String::from(
        "@prefix ex: <http://example.org/> .
         @prefix aegis: <http://aegis.gastown.local/ontology/> .
         @prefix owl: <http://www.w3.org/2002/07/owl#> .
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
         ex:a ex:p ex:b ; aegis:mentions ex:d ; ex:p ex:hub ; owl:sameAs ex:a2 ; rdfs:label \"Anchor A\" .
         ex:b ex:p ex:c .
         ex:a2 ex:p ex:e .
         ex:t1 rdfs:label \"twin\" . ex:t2 rdfs:label \"twin\" .\n",
    );
    for i in 0..200 {
        ttl.push_str(&format!("ex:hub ex:p ex:x{i} .\n"));
    }
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-07T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    // Text similarity to the query [1,0,0,0], highest first: u, d, x0, c, e, b.
    for (name, v) in [
        ("u", [0.99_f32, 0.01, 0.0, 0.0]),
        ("d", [0.95, 0.05, 0.0, 0.0]),
        ("x0", [0.9, 0.1, 0.0, 0.0]),
        ("c", [0.8, 0.2, 0.0, 0.0]),
        ("e", [0.7, 0.3, 0.0, 0.0]),
        ("b", [0.6, 0.4, 0.0, 0.0]),
    ] {
        let id = store.intern(&format!("{EX}{name}")).unwrap();
        store.embed_entity(id, name, &v, "2026-10-07").unwrap();
    }
    store
}

fn search(store: &Store, extra: &serde_json::Value) -> serde_json::Value {
    let mut input = json!({"embedding": [1.0, 0.0, 0.0, 0.0], "limit": 10, "verbose": true});
    for (k, v) in extra.as_object().unwrap() {
        input[k] = v.clone();
    }
    tool_search(store, &input).unwrap()
}

fn hops_of(result: &serde_json::Value) -> Vec<(String, Option<u64>)> {
    result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["entity"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches(EX)
                    .to_owned(),
                r["hops"].as_u64(),
            )
        })
        .collect()
}

#[test]
fn without_an_anchor_the_response_is_unchanged_by_the_flag() {
    assert_eq!(
        search(&store(false), &json!({})),
        search(&store(true), &json!({}))
    );
}

#[test]
fn an_anchor_is_refused_while_the_flag_is_off() {
    let input = json!({"embedding": [1.0, 0.0, 0.0, 0.0], "anchor": format!("{EX}a")});
    let err = tool_search(&store(false), &input).unwrap_err().to_string();
    assert!(err.contains("disabled"), "{err}");
}

#[test]
fn hops_follow_the_rules() {
    let r = search(
        &store(true),
        &json!({"anchor": format!("{EX}a"), "anchor_mode": "filter"}),
    );
    let hops: std::collections::HashMap<_, _> = hops_of(&r).into_iter().collect();
    assert_eq!(hops.get("b"), Some(&Some(1)));
    assert_eq!(hops.get("c"), Some(&Some(2)));
    assert_eq!(
        hops.get("e"),
        Some(&Some(1)),
        "sameAs costs nothing: a2 is a"
    );
    assert!(
        !hops.contains_key("d"),
        "mentions is not traversed: {hops:?}"
    );
    assert!(
        !hops.contains_key("x0"),
        "a hub is reached but not expanded: {hops:?}"
    );
    assert!(
        !hops.contains_key("u"),
        "filter keeps only reachable results"
    );
    assert!(r["anchor"]["hubs_not_expanded"].as_u64().unwrap() >= 1);
    assert_eq!(r["anchor"]["truncated"], false);
}

#[test]
fn sort_orders_by_hops_and_decay_lets_a_strong_far_match_win() {
    let sort = hops_of(&search(
        &store(true),
        &json!({"anchor": format!("{EX}a"), "anchor_mode": "sort"}),
    ));
    let firsts: Vec<_> = sort.iter().take(2).map(|(n, h)| (n.as_str(), *h)).collect();
    assert_eq!(
        firsts,
        [("e", Some(1)), ("b", Some(1))],
        "hop 1 first, by text score"
    );
    assert_eq!(sort.last().unwrap().1, None, "unreachable ranks last");
    // decay 0.5: c (0.8 x 0.25 = 0.2) loses to e (0.7 x 0.5); u (0.99 x 0.5^4) ranks low.
    let decay = hops_of(&search(&store(true), &json!({"anchor": format!("{EX}a")})));
    assert_eq!(decay[0].0, "e");
    let u = decay.iter().position(|(n, _)| n == "u").unwrap();
    let c = decay.iter().position(|(n, _)| n == "c").unwrap();
    assert!(c < u, "{decay:?}");
    // decay 1.0 is the plain text order.
    let flat = hops_of(&search(
        &store(true),
        &json!({"anchor": format!("{EX}a"), "decay": 1.0}),
    ));
    assert_eq!(flat[0].0, "u");
}

#[test]
fn the_anchor_resolves_by_iri_curie_or_label_and_never_guesses() {
    let s = store(true);
    assert_eq!(
        search(&s, &json!({"anchor": "Anchor A"}))["anchor"]["iri"],
        format!("{EX}a")
    );
    let input = |a: &str| json!({"embedding": [1.0, 0.0, 0.0, 0.0], "anchor": a});
    let err = tool_search(&s, &input("twin")).unwrap_err().to_string();
    assert!(
        err.contains("ambiguous") && err.contains("t1") && err.contains("t2"),
        "{err}"
    );
    let err = tool_search(&s, &input("nobody")).unwrap_err().to_string();
    assert!(err.contains("not an IRI"), "{err}");
}

#[test]
fn a_budget_hit_reports_truncation_and_explain_gives_a_path() {
    let s = store(true);
    let mut req = AnchorRequest::parse(&json!({"anchor": format!("{EX}a")}))
        .unwrap()
        .unwrap();
    req.budget = 3;
    let nb = walk(&s, &req, None).unwrap();
    assert!(
        nb.truncated_at.is_some(),
        "a ring cut by the budget must say so"
    );
    let r = search(
        &s,
        &json!({"anchor": format!("{EX}a"), "explain": true, "anchor_mode": "filter"}),
    );
    let c = r["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entity"] == format!("{EX}c"))
        .unwrap();
    let path = c["path"].as_str().unwrap();
    assert!(
        path.starts_with("http://example.org/a ")
            && path.contains("example.org/b")
            && path.ends_with("example.org/c"),
        "{path}"
    );
}
