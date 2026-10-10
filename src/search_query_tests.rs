use super::*;
use crate::vector::KnowledgeVectorStore;

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.search_config_mut().keyword = true;
    s.initialize_lexical_index().unwrap();
    crate::rdf::ingest_rdf(
        &mut s,
        br#"@prefix ex:<https://example.org/>.
        @prefix rdfs:<http://www.w3.org/2000/01/rdf-schema#>.
        ex:identifier rdfs:label "ClosePatch"; ex:next ex:target .
        ex:meaning rdfs:label "accounting discrepancy"; ex:next ex:target .
        ex:noise rdfs:label "unrelated"; ex:next ex:elsewhere .
        ex:target rdfs:label "target" .
    "#
        .as_slice(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    for (iri, vec) in [("meaning", vec![1.0, 0.0]), ("noise", vec![-1.0, 0.0])] {
        let id = s
            .lookup(&format!("https://example.org/{iri}"))
            .unwrap()
            .unwrap();
        s.embed_entity(id, iri, &vec, "2026-01-01T00:00:00Z")
            .unwrap();
    }
    s
}
fn input() -> Value {
    json!({"query":"ClosePatch", "seed_limit":1,
        "sparql":"SELECT ?s ?next WHERE { ?s <https://example.org/next> ?next }",
        "query_options":{"verbose":true}})
}
#[test]
fn lexical_only_traverses_identifier_without_embeddings_and_empty_stays_empty() {
    let s = store();
    let r = tool_search_query(&s, &input()).unwrap();
    assert_eq!(r["seed_count"], 1);
    assert_eq!(r["seeds"][0]["entity"], "https://example.org/identifier");
    assert_eq!(r["seeds"][0]["semantic_rank"], Value::Null);
    assert_eq!(r["result"]["count"], 1);
    assert!(
        r["result"]
            .to_string()
            .contains("https://example.org/target")
    );
    let mut q = input();
    q["query"] = "absenttoken".into();
    let r = tool_search_query(&s, &q).unwrap();
    assert_eq!(r["seed_count"], 0);
    assert_eq!(r["result"]["count"], 0);
}
#[test]
fn hybrid_includes_semantic_only_paraphrase_and_lexical_only_identifier() {
    let s = store();
    let mut q = input();
    q["mode"] = "hybrid".into();
    q["embedding"] = json!([1.0, 0.0]);
    q["seed_limit"] = 2.into();
    let r = tool_search_query(&s, &q).unwrap();
    let seeds = r["seeds"].as_array().unwrap();
    assert_eq!(seeds.len(), 2);
    let identifier = seeds
        .iter()
        .find(|v| v["entity"] == "https://example.org/identifier")
        .unwrap();
    let meaning = seeds
        .iter()
        .find(|v| v["entity"] == "https://example.org/meaning")
        .unwrap();
    assert_eq!(identifier["keyword_rank"], 1);
    assert_eq!(identifier["semantic_rank"], Value::Null);
    assert_eq!(meaning["keyword_rank"], Value::Null);
    assert_eq!(meaning["semantic_rank"], 1);
    assert_eq!(r["result"]["count"], 2);
    assert_eq!(
        seeds
            .iter()
            .filter(|v| v["entity"] == "https://example.org/identifier")
            .count(),
        1
    );
}
#[test]
fn seeds_bind_before_aggregate_and_limit_even_with_comment_braces() {
    let s = store();
    let mut q = input();
    q["sparql"]="# brace { in comment\nSELECT (COUNT(?next) AS ?n) WHERE { ?s <https://example.org/next> ?next } LIMIT 1".into();
    let r = tool_search_query(&s, &q).unwrap();
    assert_eq!(r["result"]["rows"][0]["n"], 1);
}
#[test]
fn disabled_index_and_failed_semantic_never_fall_back() {
    let mut s = store();
    s.search_config_mut().keyword = false;
    assert!(
        tool_search_query(&s, &input())
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    s.search_config_mut().keyword = true;
    let mut q = input();
    q["mode"] = "hybrid".into();
    assert!(
        tool_search_query(&s, &q)
            .unwrap_err()
            .to_string()
            .contains("embedding provider")
    );
    assert_eq!(tool_search_query(&s, &input()).unwrap()["seed_count"], 1);
}
#[test]
fn rejects_unbounded_and_scope_overrides_before_search() {
    let s = store();
    for limit in [json!(0), json!(101), json!("20"), json!(-1)] {
        let mut q = input();
        q["seed_limit"] = limit;
        assert!(tool_search_query(&s, &q).is_err());
    }
    for key in ["query", "tx", "valid_at", "federated"] {
        let mut q = input();
        q["query_options"][key] = json!(true);
        assert!(tool_search_query(&s, &q).is_err(), "{key}");
    }
    let mut q = input();
    q["sparql"] = "ASK {?s ?p ?o}".into();
    assert!(tool_search_query(&s, &q).is_err());
    q = input();
    q["seed_variable"] = "s> { ?x ?p ?o".into();
    assert!(tool_search_query(&s, &q).is_err());
}

#[test]
fn rejects_unconnected_seed_and_patterns_without_safe_bind_join() {
    let s = store();
    for sparql in [
        "SELECT ?x WHERE {?x ?p ?o}",
        "SELECT ?s WHERE {?s ?p ?o OPTIONAL {?s ?p2 ?o2}}",
        "SELECT ?s WHERE {{?s ?p ?o} UNION {?x ?p ?o}}",
        "SELECT ?s WHERE {?s <https://example.org/next>+ ?next}",
    ] {
        let mut q = input();
        q["sparql"] = sparql.into();
        assert!(tool_search_query(&s, &q).is_err(), "{sparql}");
    }
}

#[test]
fn named_query_scope_and_result_caps_survive_seeding() {
    let mut s = store();
    let mut q = input();
    q["query_options"]["graph"] = "https://example.org/unknown-graph".into();
    assert_eq!(tool_search_query(&s, &q).unwrap()["result"]["count"], 0);
    q = input();
    q["mode"] = "hybrid".into();
    q["embedding"] = json!([1.0, 0.0]);
    q["seed_limit"] = 2.into();
    s.search_config_mut().max_sparql_rows = 1;
    let r = tool_search_query(&s, &q).unwrap();
    assert_eq!(r["seed_count"], 2);
    assert_eq!(r["result"]["count"], 1);
    assert_eq!(r["result"]["truncated"], true);
}
#[test]
fn temporal_retrieval_and_query_share_the_same_cut() {
    let s = store();
    let mut q = input();
    q["valid_at"] = "2025-01-01T00:00:00Z".into();
    assert_eq!(tool_search_query(&s, &q).unwrap()["seed_count"], 0);
    q["valid_at"] = "2026-01-01T00:00:00Z".into();
    assert_eq!(tool_search_query(&s, &q).unwrap()["result"]["count"], 1);
}
#[test]
fn malformed_options_and_embeddings_are_refused() {
    let s = store();
    for options in [
        json!({"graph":false}),
        json!({"unknown":true}),
        json!({"include_kinds":[42]}),
    ] {
        let mut q = input();
        q["query_options"] = options;
        assert!(tool_search_query(&s, &q).is_err());
    }
    let mut q = input();
    q["mode"] = "semantic".into();
    q["embedding"] = json!(["bad", 0]);
    assert!(tool_search_query(&s, &q).is_err());
    q = input();
    q["sparql"] = "SELECT ?s WHERE {SELECT ?s WHERE {?s ?p ?o}}".into();
    assert!(tool_search_query(&s, &q).is_err());
}

#[test]
fn fusion_exposes_both_contributions_and_cap_before_query() {
    let s = store();
    let mut q = input();
    q["mode"] = "hybrid".into();
    q["embedding"] = json!([1.0, 0.0]);
    let r = tool_search_query(&s, &q).unwrap();
    assert_eq!(r["candidate_count"], 2);
    assert_eq!(r["seed_count"], 1);
    assert_eq!(r["seeds_truncated"], true);
    assert_eq!(r["result"]["count"], 1);
    assert_eq!(r["search_complete"], false);
    q["query"] = "accounting".into();
    let r = tool_search_query(&s, &q).unwrap();
    assert_eq!(r["candidate_count"], 1);
    assert_eq!(r["seeds"][0]["keyword_rank"], 1);
    assert_eq!(r["seeds"][0]["semantic_rank"], 1);
    assert!((r["seeds"][0]["rrf_score"].as_f64().unwrap() - 2.0 / 61.0).abs() < 1e-10);
}

#[test]
fn seeded_bgp_stays_within_join_budget_that_unbound_scan_exceeds() {
    let mut s = store();
    let mut ttl = String::new();
    for i in 0..200 {
        ttl.push_str(&format!(
            "<https://example.org/filler{i}> <https://example.org/p> \"irrelevant\" .\n"
        ));
    }
    crate::rdf::ingest_rdf(
        &mut s,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    s.search_config_mut().max_join_rows = 2;
    let sparql = "SELECT ?s ?p ?o WHERE {?s ?p ?o}";
    assert!(
        crate::tool_query(&s, &json!({"query":sparql})).is_err(),
        "unbound positive control must exceed budget"
    );
    let mut q = input();
    q["sparql"] = sparql.into();
    let r = tool_search_query(&s, &q).unwrap();
    assert_eq!(r["seed_count"], 1);
    assert_eq!(r["result"]["count"], 2);
}
#[test]
fn configured_label_floor_is_not_bypassed() {
    let mut s = store();
    let graph = "urn:test:stale";
    let g = s.overlay_create(graph, 0).unwrap();
    let e = s.lookup("https://example.org/identifier").unwrap().unwrap();
    let p = s.intern("https://example.org/value").unwrap();
    s.overlay_write(
        g,
        crate::types::Op::Assert,
        e,
        p,
        crate::types::Value::Str("stale data".into()),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    s.set_graph_label(
        graph,
        &crate::store::labels::GraphLabel {
            freshness: Some(crate::lattice::Freshness::Stale),
            ..Default::default()
        },
        "2026-01-01T00:00:00Z",
        None,
    )
    .unwrap();
    let mut q = input();
    q["sparql"] = "SELECT ?s ?v WHERE {?s <https://example.org/value> ?v}".into();
    q["query_options"]["graph"] = graph.into();
    assert_eq!(tool_search_query(&s, &q).unwrap()["result"]["count"], 1);
    s.labels_config_mut().min_freshness = Some("fresh".into());
    let error = tool_search_query(&s, &q).unwrap_err().to_string();
    assert!(
        error.contains("stale") && error.contains("refused"),
        "{error}"
    );
}
