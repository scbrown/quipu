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

#[test]
fn malformed_anchor_is_refused_instead_of_answering_unanchored() {
    for anchor in [json!(null), json!(7), json!([]), json!("")] {
        let input = json!({"embedding":[1.,0.,0.,0.],"anchor":anchor});
        assert!(
            tool_search(&store(true), &input)
                .unwrap_err()
                .to_string()
                .contains("nonempty string")
        );
    }
}

#[test]
fn hub_edges_are_bounded_and_outer_ring_queries_only_aliases() {
    let s = store(true);
    let hub = s.lookup(&format!("{EX}hub")).unwrap().unwrap();
    let capped = edges(&s, hub, Direction::Out, None, HUB_DEGREE + 1, None).unwrap();
    assert_eq!(capped.len(), HUB_DEGREE + 1);
    let a = s.lookup(&format!("{EX}a")).unwrap().unwrap();
    let aliases = ids(&s, &[namespace::OWL_SAME_AS.to_owned()]).unwrap();
    let outer = edges(&s, a, Direction::Both, None, HUB_DEGREE + 1, Some(&aliases)).unwrap();
    assert_eq!(outer.len(), 1);
    assert!(
        outer
            .iter()
            .all(|(_, predicate, _)| aliases.contains(predicate))
    );
    assert!(
        edges(
            &s,
            a,
            Direction::Both,
            None,
            HUB_DEGREE + 1,
            Some(&HashSet::new())
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn anchor_neighbours_are_admitted_below_the_global_pool_for_each_text_mode() {
    let mut s = store(true);
    s.search_config_mut().keyword = true;
    s.search_config_mut().hybrid = true;
    s.initialize_lexical_index().unwrap();
    while !s.backfill_lexical_batch(1000).unwrap().complete {}
    let mut ttl = String::from(
        "@prefix ex: <http://example.org/> . @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . ex:r ex:p ex:near . ex:near rdfs:label \"needle weak\" .\n",
    );
    for n in 0..250 {
        ttl.push_str(&format!("ex:global{n} rdfs:label \"needle\" .\n"));
    }
    crate::rdf::ingest_rdf(
        &mut s,
        ttl.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-07T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let near = s.lookup(&format!("{EX}near")).unwrap().unwrap();
    s.embed_entity(
        near,
        "needle weak",
        &[0.01, 1., 0., 0.],
        "2026-10-07T00:00:00Z",
    )
    .unwrap();
    for n in 0..250 {
        let id = s.lookup(&format!("{EX}global{n}")).unwrap().unwrap();
        s.embed_entity(id, "needle", &[1., 0., 0., 0.], "2026-10-07T00:00:00Z")
            .unwrap();
    }
    for alpha in [0.0, 0.5, 1.0] {
        let mut input = json!({"query":"needle","limit":1,"mode":"hybrid","alpha":alpha,"anchor":format!("{EX}r"),"anchor_mode":"sort","explain":true,"verbose":true});
        if alpha != 0.0 {
            input["embedding"] = json!([1., 0., 0., 0.]);
        }
        let r = tool_search(&s, &input).unwrap();
        assert_eq!(r["count"], 1, "{r}");
        assert_eq!(r["results"][0]["entity"], format!("{EX}near"), "{r}");
        assert_eq!(r["results"][0]["hops"], 1);
        assert!(r["results"][0]["path"].as_str().unwrap().contains("near"));
        assert!(r["results"][0]["explain"].is_object());
        let mut plain = input.clone();
        plain.as_object_mut().unwrap().remove("anchor");
        let before = tool_search(&s, &plain).unwrap();
        s.search_config_mut().anchored = false;
        assert_eq!(tool_search(&s, &plain).unwrap(), before);
        assert!(
            tool_search(&s, &input)
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
        s.search_config_mut().anchored = true;
    }
}

#[test]
fn malformed_traversal_options_cannot_silently_widen_the_walk() {
    for extra in [
        json!({"max_hops":-1}),
        json!({"max_hops":"3"}),
        json!({"via":[7]}),
        json!({"via":"p"}),
        json!({"direction":7}),
        json!({"anchor_mode":7}),
        json!({"decay":"0.5"}),
    ] {
        let mut input = extra;
        input["anchor"] = json!(format!("{EX}a"));
        assert!(AnchorRequest::parse(&input).is_err(), "{input}");
    }
    assert!(AnchorRequest::parse(&json!({"anchor":format!("{EX}a"),"via":[]})).is_ok());
}

#[test]
fn neighbourhood_candidates_retain_type_and_historical_text_scope() {
    let mut s = Store::open_in_memory().unwrap();
    s.search_config_mut().anchored = true;
    s.search_config_mut().hybrid = true;
    s.search_config_mut().keyword = true;
    s.initialize_lexical_index().unwrap();
    let old = r#"@prefix ex: <http://example.org/> . @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
      ex:root ex:p ex:public, ex:secret . ex:public a ex:Public ; rdfs:label "old record" .
      ex:secret a ex:Secret ; rdfs:label "needle secret" ."#;
    crate::rdf::ingest_rdf(
        &mut s,
        old.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    for (name, text) in [("public", "old record"), ("secret", "needle secret")] {
        let id = s.lookup(&format!("{EX}{name}")).unwrap().unwrap();
        s.embed_entity(id, text, &[1., 0.], "2026-01-01T00:00:00Z")
            .unwrap();
    }
    let later = r#"<http://example.org/public> <http://www.w3.org/2000/01/rdf-schema#comment> "needle future" ."#;
    crate::rdf::ingest_rdf(
        &mut s,
        later.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let public = s.lookup(&format!("{EX}public")).unwrap().unwrap();
    s.embed_entity(public, "needle future", &[1., 0.], "2026-10-01T00:00:00Z")
        .unwrap();
    let base = json!({"mode":"hybrid","alpha":0.5,"query":"needle","embedding":[1.,0.],"anchor":format!("{EX}root"),"anchor_mode":"filter","entity_type":format!("{EX}Public"),"valid_at":"2026-02-01T00:00:00Z","verbose":true,"explain":true});
    let historical = tool_search(&s, &base).unwrap();
    assert_eq!(historical["count"], 1);
    assert_eq!(historical["results"][0]["entity"], format!("{EX}public"));
    assert!(
        !historical["results"][0]["text"]
            .as_str()
            .unwrap()
            .contains("future")
    );
    assert!(historical["results"][0]["explain"]["bm25"].is_null());
    let mut keyword = base.clone();
    keyword["alpha"] = json!(0.0);
    keyword.as_object_mut().unwrap().remove("embedding");
    assert_eq!(tool_search(&s, &keyword).unwrap()["count"], 0);
    keyword["query"] = json!("old");
    assert_eq!(tool_search(&s, &keyword).unwrap()["count"], 1);
    keyword["query"] = json!("needle");
    keyword["valid_at"] = json!("2026-10-02T00:00:00Z");
    assert_eq!(tool_search(&s, &keyword).unwrap()["count"], 1);
}
