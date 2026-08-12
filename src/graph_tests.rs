use super::*;
use crate::rdf::ingest_rdf;

fn test_graph_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:knows ex:bob ; ex:knows ex:carol .
ex:bob a ex:Person ; ex:knows ex:carol .
ex:carol a ex:Person ; ex:knows ex:dave .
ex:dave a ex:Person .
ex:server1 a ex:Server ; ex:hosts ex:app1 .
ex:app1 a ex:App ; ex:uses ex:server1 .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

#[test]
fn test_project_all() {
    let store = test_graph_store();
    let pg = project(&store, None, None).unwrap();
    assert!(pg.node_count() >= 6);
    assert!(pg.edge_count() >= 10); // includes rdf:type edges
}

#[test]
fn test_project_scoped_to_named_graph() {
    let mut store = test_graph_store();
    let overlay = "urn:test:graph:derived";
    let g = store.graph_create(overlay).unwrap();
    let turtle = r"
@prefix ex: <http://example.org/> .
ex:x a ex:Person ; ex:knows ex:y .
ex:y a ex:Person .
";
    crate::rdf::ingest_rdf_to_graph(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
        g,
    )
    .unwrap();

    // The scoped projection sees ONLY the overlay's facts…
    let pg = project_in_graph(&store, None, None, Some(overlay)).unwrap();
    assert_eq!(pg.node_count(), 3); // x, y, ex:Person (type edges)
    // …and ROOT's projection is untouched by the overlay write.
    let root = project(&store, None, None).unwrap();
    let x = store.lookup("http://example.org/x").unwrap().unwrap();
    assert!(!root.entity_to_node.contains_key(&x));
}

#[test]
fn test_project_unknown_graph_is_an_error() {
    let store = test_graph_store();
    let err = project_in_graph(&store, None, None, Some("urn:no:such:graph"));
    assert!(err.is_err(), "a typo'd graph must refuse, not rank nothing");
}

#[test]
fn test_project_cached_reuses_until_a_write() {
    let mut store = test_graph_store();
    let first = project_cached(&store, None, None, None).unwrap();
    let again = project_cached(&store, None, None, None).unwrap();
    assert!(
        Arc::ptr_eq(&first, &again),
        "unchanged store must hit the resident projection"
    );

    // A different shape is a miss (single-entry cache) — and does not
    // corrupt the earlier Arc, which the caller still holds.
    let filtered = project_cached(&store, Some("http://example.org/Person"), None, None).unwrap();
    assert!(!Arc::ptr_eq(&first, &filtered));
    assert_eq!(first.node_count(), again.node_count());

    // Any committed transaction invalidates: the rebuild sees the new fact.
    let turtle = r"
@prefix ex: <http://example.org/> .
ex:new a ex:Person ; ex:knows ex:alice .
";
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-04-05T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let rebuilt = project_cached(&store, None, None, None).unwrap();
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    let new = store.lookup("http://example.org/new").unwrap().unwrap();
    assert!(rebuilt.entity_to_node.contains_key(&new));
    assert!(!first.entity_to_node.contains_key(&new));
}

#[test]
fn test_project_type_filter() {
    let store = test_graph_store();
    let pg = project(&store, Some("http://example.org/Person"), None).unwrap();
    // Only Person entities as sources
    assert!(pg.node_count() >= 4);
}

#[test]
fn test_project_predicate_filter() {
    let store = test_graph_store();
    let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
    assert_eq!(pg.edge_count(), 4); // alice->bob, alice->carol, bob->carol, carol->dave
}

#[test]
fn test_in_degree() {
    let store = test_graph_store();
    let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
    let degrees = in_degree(&pg);
    // carol should have highest in-degree (alice + bob know carol)
    let carol_id = store.lookup("http://example.org/carol").unwrap().unwrap();
    let carol_deg = degrees.iter().find(|(id, _)| *id == carol_id).unwrap().1;
    assert_eq!(carol_deg, 2);
}

#[test]
fn test_shortest_path() {
    let store = test_graph_store();
    let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
    let path = shortest_path(
        &store,
        &pg,
        "http://example.org/alice",
        "http://example.org/dave",
    )
    .unwrap();
    assert!(path.is_some());
    let path = path.unwrap();
    // alice -> carol -> dave (length 2)
    assert!(path.len() <= 4); // at most alice->bob->carol->dave
    assert_eq!(path.first().unwrap(), "http://example.org/alice");
    assert_eq!(path.last().unwrap(), "http://example.org/dave");
}

#[test]
fn test_pagerank_converges_and_sums_to_one() {
    let store = test_graph_store();
    let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
    let ranks = page_rank(&pg, &PageRankConfig::default()).unwrap();
    assert!(!ranks.is_empty());
    let sum: f32 = ranks.iter().map(|(_, s)| s).sum();
    assert!(
        (sum - 1.0).abs() < 1e-3,
        "ranks should sum to ~1, got {sum}"
    );
}

#[test]
fn test_pagerank_ranks_hub_highest() {
    let store = test_graph_store();
    let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
    let ranks = page_rank(&pg, &PageRankConfig::default()).unwrap();
    // dave is a sink reached via carol (alice/bob/carol all flow toward it);
    // carol is referenced by both alice and bob. Top-ranked should be carol
    // or dave, never alice (which has no incoming knows edges).
    let alice = store.lookup("http://example.org/alice").unwrap().unwrap();
    let top = ranks[0].0;
    assert_ne!(top, alice, "alice has no in-edges and must not rank first");
    let carol = store.lookup("http://example.org/carol").unwrap().unwrap();
    let dave = store.lookup("http://example.org/dave").unwrap().unwrap();
    assert!(top == carol || top == dave, "expected carol or dave on top");
}

#[test]
fn test_personalized_pagerank_favors_seed_neighborhood() {
    let store = test_graph_store();
    let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
    let alice = store.lookup("http://example.org/alice").unwrap().unwrap();
    let cfg = PageRankConfig {
        seeds: vec![alice],
        ..Default::default()
    };
    let ranks = page_rank(&pg, &cfg).unwrap();
    // Personalized at alice: alice itself should carry significant rank
    // (restart mass) — far more than under global PageRank where it has 0
    // in-edges.
    let alice_score = ranks.iter().find(|(id, _)| *id == alice).unwrap().1;
    assert!(
        alice_score > 0.1,
        "seed should retain restart mass, got {alice_score}"
    );
}

#[test]
fn test_pagerank_empty_graph() {
    let store = Store::open_in_memory().unwrap();
    let pg = project(&store, None, None).unwrap();
    let ranks = page_rank(&pg, &PageRankConfig::default()).unwrap();
    assert!(ranks.is_empty());
}

#[test]
fn test_tool_project_pagerank() {
    let mut store = test_graph_store();
    let input = serde_json::json!({
        "algorithm": "pagerank",
        "predicate": "http://example.org/knows",
        "limit": 5
    });
    let result = tool_project(&mut store, &input).unwrap();
    assert_eq!(result["algorithm"], "pagerank");
    assert_eq!(result["personalized"], false);
    assert!(result["count"].as_u64().unwrap() > 0);
    assert!(result["results"][0]["score"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_tool_project_ppr_with_seeds() {
    let mut store = test_graph_store();
    let input = serde_json::json!({
        "algorithm": "ppr",
        "predicate": "http://example.org/knows",
        "seeds": ["http://example.org/alice"]
    });
    let result = tool_project(&mut store, &input).unwrap();
    assert_eq!(result["algorithm"], "pagerank");
    assert_eq!(result["personalized"], true);
}

#[test]
fn test_connected_components() {
    let store = test_graph_store();
    let pg = project(&store, None, None).unwrap();
    let comps = connected_components(&pg);
    assert!(!comps.is_empty());
}

#[test]
fn test_tool_project_stats() {
    let mut store = test_graph_store();
    let input = serde_json::json!({"algorithm": "stats"});
    let result = tool_project(&mut store, &input).unwrap();
    assert!(result["nodes"].as_u64().unwrap() >= 6);
    assert!(result["edges"].as_u64().unwrap() >= 4);
}

#[test]
fn test_tool_project_in_degree() {
    let mut store = test_graph_store();
    let input = serde_json::json!({
        "algorithm": "in_degree",
        "predicate": "http://example.org/knows",
        "limit": 5
    });
    let result = tool_project(&mut store, &input).unwrap();
    assert_eq!(result["algorithm"], "in_degree");
    assert!(result["count"].as_u64().unwrap() > 0);
}

#[test]
fn test_tool_project_shortest_path() {
    let mut store = test_graph_store();
    let input = serde_json::json!({
        "algorithm": "shortest_path",
        "predicate": "http://example.org/knows",
        "from": "http://example.org/alice",
        "to": "http://example.org/dave"
    });
    let result = tool_project(&mut store, &input).unwrap();
    assert!(result["path"].is_array());
    assert!(result["length"].as_u64().unwrap() >= 2);
}

// ── Louvain community detection (hq-zlph) ────────────────────────────────

/// Two 3-cliques joined by a single bridge edge — an unambiguous two-community
/// structure. Edges use ex:link so a predicate-filtered projection is clean.
fn two_cluster_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
ex:a1 ex:link ex:a2 . ex:a1 ex:link ex:a3 . ex:a2 ex:link ex:a3 .
ex:b1 ex:link ex:b2 . ex:b1 ex:link ex:b3 . ex:b2 ex:link ex:b3 .
ex:a1 ex:link ex:b1 .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

/// AC 1/3: louvain separates the two cliques and is deterministic (same graph
/// → identical partition, independent of hash iteration order).
#[test]
fn louvain_finds_two_clusters_deterministically() {
    let store = two_cluster_store();
    let pg = project(&store, None, Some("http://example.org/link")).unwrap();

    let c1 = louvain(&pg);
    let c2 = louvain(&pg);
    assert_eq!(
        c1.groups, c2.groups,
        "same graph must yield identical partition"
    );
    assert_eq!(c1.groups.len(), 2, "exactly two communities");
    for g in &c1.groups {
        assert_eq!(g.len(), 3, "each clique is one community of 3");
    }
    assert!(c1.modularity > 0.0, "clear structure → positive modularity");

    let id = |iri: &str| store.lookup(iri).unwrap().unwrap();
    let group_of = |e: i64| c1.groups.iter().position(|g| g.contains(&e)).unwrap();
    assert_eq!(
        group_of(id("http://example.org/a1")),
        group_of(id("http://example.org/a2")),
        "a-clique stays together"
    );
    assert_ne!(
        group_of(id("http://example.org/a1")),
        group_of(id("http://example.org/b1")),
        "the two cliques are distinct communities"
    );
}

/// AC 2: persist:true writes quipu:memberOfCommunity facts, queryable via SPARQL.
#[test]
fn louvain_persist_writes_queryable_membership() {
    let mut store = two_cluster_store();
    let input = serde_json::json!({
        "algorithm": "louvain",
        "predicate": "http://example.org/link",
        "persist": true
    });
    let result = tool_project(&mut store, &input).unwrap();
    assert_eq!(result["algorithm"], "louvain");
    assert_eq!(
        result["persisted"].as_u64().unwrap(),
        6,
        "6 entities got a community"
    );

    // SPARQL: a1 has exactly one community membership.
    let pred = format!("{}memberOfCommunity", namespace::QUIPU);
    let q = format!("SELECT ?c WHERE {{ <http://example.org/a1> <{pred}> ?c }}");
    let rows = crate::sparql::query(&store, &q).unwrap();
    assert_eq!(
        rows.rows().len(),
        1,
        "a1's community is queryable via SPARQL"
    );

    // Read-only default: no persist flag writes nothing new.
    let mut store2 = two_cluster_store();
    let ro = tool_project(
        &mut store2,
        &serde_json::json!({"algorithm":"louvain","predicate":"http://example.org/link"}),
    )
    .unwrap();
    assert!(ro["persisted"].is_null(), "default is read-only");
}

/// AC 5: a re-run bitemporally SUPERSEDES prior membership — no stale
/// accumulation. Driven through `persist_communities` directly with changed
/// partitions so the supersede is unambiguous.
#[test]
fn persist_supersedes_prior_membership() {
    let mut store = Store::open_in_memory().unwrap();
    let e1 = store.intern("http://example.org/e1").unwrap();
    let e2 = store.intern("http://example.org/e2").unwrap();

    // Run 1: two singleton communities.
    persist_communities(&mut store, &[vec![e1], vec![e2]], "2026-01-01T00:00:00Z").unwrap();
    // Run 2: both entities merge into one community.
    persist_communities(&mut store, &[vec![e1, e2]], "2026-01-02T00:00:00Z").unwrap();

    let pred_id = store
        .lookup(&format!("{}memberOfCommunity", namespace::QUIPU))
        .unwrap()
        .unwrap();
    let active: Vec<_> = store
        .current_facts()
        .unwrap()
        .into_iter()
        .filter(|f| f.attribute == pred_id)
        .collect();
    assert_eq!(
        active.len(),
        2,
        "exactly one active membership per entity (no accumulation)"
    );

    // Both now point at the latest derivation's community_0.
    let comm0 = store
        .lookup(&format!("{}community_0", namespace::QUIPU))
        .unwrap()
        .unwrap();
    for f in &active {
        assert_eq!(
            f.value,
            Value::Ref(comm0),
            "memberships reflect the latest run"
        );
    }
}
