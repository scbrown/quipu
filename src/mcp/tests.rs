//! Tests for MCP tool handlers.

use std::sync::Arc;

use super::graphiti::*;
use super::tools::*;
use super::*;
use crate::embedding::EmbeddingProvider;
use crate::error::Result as QResult;
use crate::vector::KnowledgeVectorStore;

fn test_store_with_data() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ; ex:name "Alice" ; ex:age "30"^^xsd:integer .
ex:bob a ex:Person ; ex:name "Bob" ; ex:age "25"^^xsd:integer .
"#;
    crate::rdf::ingest_rdf(
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
fn test_tool_query() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "SELECT ?name WHERE { ?s <http://example.org/name> ?name }"
    });
    let result = tool_query(&store, &input).unwrap();
    assert_eq!(result["count"], 2);
    assert_eq!(result["variables"], serde_json::json!(["name"]));
}

#[test]
fn test_tool_knot() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:carol a ex:Person ; ex:name \"Carol\" .",
        "timestamp": "2026-04-04T01:00:00Z",
        "actor": "test",
        "source": "unit-test"
    });
    let result = tool_knot(&mut store, &input).unwrap();
    assert_eq!(result["conforms"], true);
    assert_eq!(result["count"], 2);
    assert!(result["tx_id"].as_i64().unwrap() > 0);
}

#[test]
fn knot_snapshot_replacement_retracts_removed_turtle_facts() {
    let mut store = Store::open_in_memory().unwrap();
    let snapshot = |turtle: &str| {
        serde_json::json!({
            "turtle": turtle,
            "timestamp": "2026-08-07T10:00:00Z",
            "actor": "hank",
            "source": "hank promote demo@abc (cli)",
            "replace_snapshot": true,
            "snapshot": "code:demo"
        })
    };
    let query = serde_json::json!({
        "query": "PREFIX ex: <http://example.org/> SELECT ?s WHERE { ?s a ex:Module }"
    });

    tool_knot(
        &mut store,
        &snapshot("@prefix ex: <http://example.org/> . ex:a a ex:Module . ex:b a ex:Module ."),
    )
    .unwrap();
    assert_eq!(tool_query(&store, &query).unwrap()["count"], 2);

    tool_knot(&mut store, &snapshot("@prefix ex: <http://example.org/> .")).unwrap();
    assert_eq!(tool_query(&store, &query).unwrap()["count"], 0);

    tool_knot(
        &mut store,
        &snapshot("@prefix ex: <http://example.org/> . ex:c a ex:Module ."),
    )
    .unwrap();
    assert_eq!(tool_query(&store, &query).unwrap()["count"], 1);
}

#[test]
fn knot_snapshot_replacement_requires_stable_identity() {
    let mut store = Store::open_in_memory().unwrap();
    let err = tool_knot(
        &mut store,
        &serde_json::json!({"turtle": "", "replace_snapshot": true}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("stable 'snapshot' producer key"));
}

#[test]
fn omitted_timestamp_defaults_to_now_not_epoch() {
    // hq-tb4: a write with no explicit timestamp must be stamped with the real
    // clock, not 1970 — defaulting to epoch silently corrupts the bitemporal
    // log and breaks time-travel queries (quipu's flagship feature).
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:dave a ex:Person ; ex:name \"Dave\" .",
    });
    let res = tool_knot(&mut store, &input).unwrap();
    assert!(
        res["tx_id"].as_i64().unwrap() > 0,
        "knot should commit a tx"
    );

    let facts = tool_unravel(&store, &serde_json::json!({})).unwrap();
    let list = facts["facts"].as_array().unwrap();
    assert!(!list.is_empty(), "expected facts after knot");
    for f in list {
        let vf = f["valid_from"].as_str().unwrap_or("");
        assert!(
            !vf.starts_with("1970"),
            "valid_from defaulted to epoch: {vf}"
        );
        assert!(
            vf.starts_with("20"),
            "expected a current-era valid_from, got {vf}"
        );
    }
}

#[test]
#[cfg(feature = "shacl")]
fn test_tool_knot_with_validation_failure() {
    let mut store = Store::open_in_memory().unwrap();
    let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
"#;
    let input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:bad a ex:Person .",
        "shapes": shapes,
        "timestamp": "2026-04-04T01:00:00Z"
    });
    let result = tool_knot(&mut store, &input).unwrap();
    assert_eq!(result["conforms"], false);
    assert!(result["violations"].as_u64().unwrap() > 0);
}

#[test]
fn test_tool_cord() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "type": "http://example.org/Person"
    });
    let result = tool_cord(&store, &input).unwrap();
    assert_eq!(result["count"], 2);
}

#[test]
fn test_tool_cord_all() {
    let store = test_store_with_data();
    let input = serde_json::json!({ "limit": 10 });
    let result = tool_cord(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 2);
}

#[test]
fn test_tool_unravel() {
    let mut store = Store::open_in_memory().unwrap();

    crate::rdf::ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:a ex:v \"1\" .".as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    crate::rdf::ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:b ex:v \"2\" .".as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-02-01",
        None,
        None,
    )
    .unwrap();

    let input = serde_json::json!({ "tx": 1 });
    let result = tool_unravel(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
}

#[test]
#[cfg(feature = "shacl")]
fn test_tool_validate() {
    let input = serde_json::json!({
        "shapes": "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:property [ sh:path ex:name ; sh:minCount 1 ] .",
        "data": "@prefix ex: <http://example.org/> .\nex:x a ex:T ; ex:name \"ok\" ."
    });
    let result = tool_validate(&input).unwrap();
    assert_eq!(result["conforms"], true);
}

#[test]
fn tool_episode_mints_under_configured_base_ns() {
    // aegis-4h3x: the REST/MCP ingest path must mint IRIs under the store's
    // configured base_ns, not the hardcoded aegis default. Verified by INGESTING
    // and querying the entity back under the configured base — not by reading
    // config — because a wrong namespace fragments the graph silently.
    let custom = "http://example.org/kb/";
    let mut store = Store::open_in_memory().unwrap();
    store.set_base_ns(custom);

    let input = serde_json::json!({
        "name": "ns-check",
        "episode_body": "widget lives here",
        "source": "test",
        "group_id": "test",
        "timestamp": "2026-04-04T12:00:00Z",
        "nodes": [{"name": "widget", "type": "Thing"}],
        "edges": []
    });
    tool_episode(&mut store, &input).unwrap();

    // The entity exists under the CONFIGURED namespace ...
    assert!(
        ask(&store, &format!("<{custom}widget> ?p ?o")),
        "entity was not minted under the configured base_ns {custom}"
    );
    // ... and NOT under the aegis default it used to hardcode.
    assert!(
        !ask(&store, "<http://aegis.gastool.local/ontology/widget> ?p ?o"),
        "sanity: bogus-namespace probe should never match"
    );
    assert!(
        !ask(
            &store,
            &format!("<{}widget> ?p ?o", crate::namespace::DEFAULT_BASE_NS)
        ),
        "entity leaked into the hardcoded aegis namespace — base_ns was ignored (the aegis-4h3x bug)"
    );
}

#[test]
fn test_tool_episode() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "deploy-event",
        "episode_body": "Deployed new version of tapestry to ct-236",
        "source": "crew/mayor",
        "group_id": "aegis-ontology",
        "timestamp": "2026-04-04T12:00:00Z",
        "nodes": [
            {"name": "tapestry", "type": "WebApplication", "description": "Web UI"},
            {"name": "ct-236", "type": "LXCContainer"}
        ],
        "edges": [
            {"source": "tapestry", "target": "ct-236", "relation": "deployed_on"}
        ]
    });
    let result = tool_episode(&mut store, &input).unwrap();
    assert_eq!(result["episode"], "deploy-event");
    assert!(result["tx_id"].as_i64().unwrap() > 0);
    assert!(result["count"].as_i64().unwrap() >= 10);
}

// -- Episode-scoped logical retraction (aegis-hxb) --

const NS: &str = "http://aegis.gastown.local/ontology/";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

fn ask(store: &Store, pattern: &str) -> bool {
    let r = tool_query(
        store,
        &serde_json::json!({ "query": format!("ASK {{ {pattern} }}") }),
    )
    .unwrap();
    r["result"].as_bool().unwrap()
}

/// Ingest a "real graph" episode and a "goldblum test" episode that share a real
/// entity (quipu-server), then retract only the test episode. Its contributions
/// must vanish from current queries while the shared entity and the real
/// episode's facts survive — the core shared-IRI guarantee of aegis-hxb.
#[test]
fn test_retract_episode_scoped_isolation() {
    let mut store = Store::open_in_memory().unwrap();

    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "real-graph",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [
                {"name": "quipu-server", "type": "WebApplication"},
                {"name": "kota", "type": "LXCContainer"}
            ],
            "edges": [{"source": "quipu-server", "target": "kota", "relation": "runs_on"}]
        }),
    )
    .unwrap();

    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "goldblum-deploy-verify-032",
            "timestamp": "2026-04-02T00:00:00Z",
            "nodes": [
                {"name": "quipu-server", "type": "WebApplication"},
                {"name": "v032", "type": "Version"}
            ],
            "edges": [{"source": "quipu-server", "target": "v032", "relation": "running_version_on"}]
        }),
    )
    .unwrap();

    // Pre-condition: both episodes' facts are live.
    assert!(ask(
        &store,
        &format!("<{NS}quipu-server> <{NS}running_version_on> <{NS}v032>")
    ));
    assert!(ask(
        &store,
        &format!("<{NS}quipu-server> <{NS}runs_on> <{NS}kota>")
    ));

    let out = tool_retract_episode(
        &mut store,
        &serde_json::json!({ "episode": "goldblum-deploy-verify-032", "actor": "tester" }),
    )
    .unwrap();
    assert_eq!(out["episode"], "goldblum-deploy-verify-032");
    assert!(out["retracted"].as_i64().unwrap() > 0);
    assert!(!out["statements"].as_array().unwrap().is_empty());

    // The test episode's edge and generated entity are gone from current views…
    assert!(!ask(
        &store,
        &format!("<{NS}quipu-server> <{NS}running_version_on> <{NS}v032>")
    ));
    let v032 = tool_query(
        &store,
        &serde_json::json!({ "query": format!("SELECT ?p ?o WHERE {{ <{NS}v032> ?p ?o }}") }),
    )
    .unwrap();
    assert_eq!(v032["count"], 0, "generated entity v032 fully retracted");

    // …but the shared entity and the real episode's facts are intact.
    assert!(ask(
        &store,
        &format!("<{NS}quipu-server> <{NS}runs_on> <{NS}kota>")
    ));
    assert!(ask(
        &store,
        &format!("<{NS}quipu-server> <{RDFS_LABEL}> \"quipu-server\"")
    ));
}

#[test]
fn test_retract_episode_idempotent_and_unknown() {
    let mut store = Store::open_in_memory().unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-x",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [{"name": "thing", "type": "Widget"}],
            "edges": []
        }),
    )
    .unwrap();

    let first =
        tool_retract_episode(&mut store, &serde_json::json!({ "episode": "ep-x" })).unwrap();
    assert!(first["retracted"].as_i64().unwrap() > 0);

    // Re-retracting is a no-op.
    let second =
        tool_retract_episode(&mut store, &serde_json::json!({ "episode": "ep-x" })).unwrap();
    assert_eq!(second["retracted"], 0);
    assert_eq!(second["tx_id"], crate::episode::NOOP_TX);

    // Unknown episode is also a clean no-op.
    let unknown =
        tool_retract_episode(&mut store, &serde_json::json!({ "episode": "nope" })).unwrap();
    assert_eq!(unknown["retracted"], 0);

    // `episode_id` alias is accepted.
    let aliased = tool_retract_episode(&mut store, &serde_json::json!({ "episode_id": "nope" }));
    assert!(aliased.is_ok());

    // Missing identifier is an error.
    assert!(tool_retract_episode(&mut store, &serde_json::json!({})).is_err());
}

#[test]
fn test_retract_triple_level_removes_exactly_one_statement() {
    // aegis-arup ask 1. maldoon needed 2 stray edges gone; the only handle was
    // "retract the episode" — 33 statements for a 2-statement target. entity +
    // predicate + value pins one triple, so the blunt tool is no longer the
    // only tool.
    let mut store = Store::open_in_memory().unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-strays",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [
                {"name": "jw3k", "type": "Bead"},
                {"name": "stray", "type": "Bead"},
                {"name": "keeper", "type": "Bead"}
            ],
            "edges": [
                {"source": "jw3k", "target": "stray", "relation": "instance_of"},
                {"source": "jw3k", "target": "keeper", "relation": "instance_of"}
            ]
        }),
    )
    .unwrap();
    let before = tool_query(
        &store,
        &serde_json::json!({ "query": format!("SELECT ?p ?o WHERE {{ <{NS}jw3k> ?p ?o }}") }),
    )
    .unwrap()["count"]
        .as_i64()
        .unwrap();

    let out = tool_retract(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}jw3k"),
            "predicate": format!("{NS}instance_of"),
            "value": {"iri": format!("{NS}stray")}
        }),
    )
    .unwrap();

    assert_eq!(
        out["retracted"], 1,
        "exactly one statement, not the episode"
    );
    assert!(!ask(
        &store,
        &format!("<{NS}jw3k> <{NS}instance_of> <{NS}stray>")
    ));
    // The sibling edge, the node's identity, and everything else survive.
    assert!(ask(
        &store,
        &format!("<{NS}jw3k> <{NS}instance_of> <{NS}keeper>")
    ));
    assert!(ask(&store, &format!("<{NS}jw3k> <{RDFS_LABEL}> \"jw3k\"")));
    let after = tool_query(
        &store,
        &serde_json::json!({ "query": format!("SELECT ?p ?o WHERE {{ <{NS}jw3k> ?p ?o }}") }),
    )
    .unwrap()["count"]
        .as_i64()
        .unwrap();
    assert_eq!(after, before - 1, "blast radius is exactly 1");
}

#[test]
fn test_retract_without_value_still_takes_the_whole_predicate() {
    // POSITIVE CONTROL for the test above: drop the `value` narrowing and BOTH
    // instance_of edges go. If `value` were silently ignored, the test above
    // would pass for the wrong reason — this pins the difference.
    let mut store = Store::open_in_memory().unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-strays",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [
                {"name": "jw3k", "type": "Bead"},
                {"name": "stray", "type": "Bead"},
                {"name": "keeper", "type": "Bead"}
            ],
            "edges": [
                {"source": "jw3k", "target": "stray", "relation": "instance_of"},
                {"source": "jw3k", "target": "keeper", "relation": "instance_of"}
            ]
        }),
    )
    .unwrap();

    let out = tool_retract(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}jw3k"),
            "predicate": format!("{NS}instance_of")
        }),
    )
    .unwrap();

    assert_eq!(out["retracted"], 2);
    assert!(!ask(
        &store,
        &format!("<{NS}jw3k> <{NS}instance_of> <{NS}keeper>")
    ));
}

// -- Ghost nodes at the API boundary (aegis-arup) --

/// maldoon's specimen, through the real ingest path: episode A declares a node's
/// identity, episode B only adds an inbound edge (the re-asserted identical
/// label is skipped as already-active, so episode A still OWNS the identity).
fn ghost_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-a",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [{"name": "ty4h", "type": "Bead"}],
            "edges": []
        }),
    )
    .unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-b",
            "timestamp": "2026-04-02T00:00:00Z",
            "nodes": [{"name": "lnmc", "type": "Bead"}],
            "edges": [{"source": "lnmc", "target": "ty4h", "relation": "applies_to"}]
        }),
    )
    .unwrap();
    store
}

#[test]
fn test_retract_episode_keeps_identity_of_edge_reachable_node() {
    let mut store = ghost_store();
    assert!(ask(&store, &format!("<{NS}ty4h> <{RDFS_LABEL}> \"ty4h\"")));

    let out = tool_retract_episode(&mut store, &serde_json::json!({ "episode": "ep-a" })).unwrap();

    assert_eq!(out["on_orphan"], "preserve");
    assert_eq!(out["identity_orphans"], 1);
    assert_eq!(
        out["identity_orphan_entities"][0]["entity"],
        format!("{NS}ty4h")
    );
    assert!(out["identity_preserved"].as_i64().unwrap() >= 2);

    // Still findable by the two discovery paths every agent-facing read uses.
    assert!(
        ask(&store, &format!("<{NS}ty4h> <{RDFS_LABEL}> \"ty4h\"")),
        "label scan must still find it"
    );
    assert!(
        ask(&store, &format!("<{NS}ty4h> a <{NS}Bead>")),
        "rdf:type query must still find it"
    );
    // …and episode B's edge is untouched.
    assert!(ask(
        &store,
        &format!("<{NS}lnmc> <{NS}applies_to> <{NS}ty4h>")
    ));
}

#[test]
fn test_retract_episode_refuse_rejects_and_writes_nothing() {
    // The reference docs (rest-api.md / mcp-tools.md) promise on_orphan="refuse"
    // rejects an orphaning retraction and changes nothing. Assert it at the TOOL
    // level — the handler the docs describe — so the doc and the handler cannot
    // drift apart. ghost_store() is exactly the orphaning case: retracting ep-a
    // would strip ty4h's identity while ep-b's edge keeps it alive.
    let mut store = ghost_store();

    let err = tool_retract_episode(
        &mut store,
        &serde_json::json!({ "episode": "ep-a", "on_orphan": "refuse" }),
    );
    assert!(
        err.is_err(),
        "on_orphan=refuse must reject an orphaning retraction"
    );

    // Refuse means refuse: identity and the label are still there, nothing written.
    assert!(
        ask(&store, &format!("<{NS}ty4h> <{RDFS_LABEL}> \"ty4h\"")),
        "a refused retraction must leave the label intact"
    );
    assert!(
        ask(&store, &format!("<{NS}ty4h> a <{NS}Bead>")),
        "a refused retraction must leave rdf:type intact"
    );

    // And an unknown/bad policy value is a clean error, not a silent default.
    assert!(
        tool_retract_episode(
            &mut store,
            &serde_json::json!({ "episode": "ep-a", "on_orphan": "destroy" }),
        )
        .is_err(),
        "an unrecognized on_orphan value must error, not fall back to a default"
    );
}

#[test]
fn test_retract_episode_allow_ghosts_and_says_so() {
    // POSITIVE CONTROL: the assertions above must fail against the pre-fix
    // behaviour, which `allow` still is. What changed even here is silence —
    // the response now names the node it just made invisible.
    let mut store = ghost_store();

    let out = tool_retract_episode(
        &mut store,
        &serde_json::json!({ "episode": "ep-a", "on_orphan": "allow" }),
    )
    .unwrap();

    assert_eq!(out["on_orphan"], "allow");
    assert_eq!(out["identity_preserved"], 0);
    assert_eq!(out["identity_orphans"], 1);
    assert!(
        !ask(&store, &format!("<{NS}ty4h> <{RDFS_LABEL}> \"ty4h\"")),
        "the ghost has no name"
    );
    assert!(
        !ask(&store, &format!("<{NS}ty4h> a <{NS}Bead>")),
        "and no type"
    );
    assert!(
        ask(&store, &format!("<{NS}lnmc> <{NS}applies_to> <{NS}ty4h>")),
        "yet it is still there, holding an edge"
    );
}

#[test]
fn test_retract_episode_refuse_names_the_node_and_writes_nothing() {
    let mut store = ghost_store();

    let err = tool_retract_episode(
        &mut store,
        &serde_json::json!({ "episode": "ep-a", "on_orphan": "refuse" }),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("ty4h"), "must name the node: {err}");

    // Refused means nothing moved.
    assert!(ask(&store, &format!("<{NS}ty4h> <{RDFS_LABEL}> \"ty4h\"")));
    assert!(ask(&store, &format!("<{NS}ty4h> a <{NS}Bead>")));

    // An unknown policy is rejected rather than silently treated as the default.
    assert!(
        tool_retract_episode(
            &mut store,
            &serde_json::json!({ "episode": "ep-a", "on_orphan": "ignore" })
        )
        .is_err()
    );
}

#[test]
fn test_tool_query_enforces_max_sparql_rows() {
    // hq-gkd: a LIMIT-less SELECT must not dump the whole fact log — the server
    // ceiling truncates the result and flags it.
    let mut store = test_store_with_data(); // 2 Person entities, each w/ name + age
    store.search_config_mut().max_sparql_rows = 1;

    let result = tool_query(
        &store,
        &serde_json::json!({ "query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o }" }),
    )
    .unwrap();

    assert_eq!(
        result["count"], 1,
        "rows should be capped at max_sparql_rows"
    );
    assert_eq!(result["truncated"], true, "truncation must be surfaced");
}

#[test]
fn test_tool_query_not_truncated_when_under_ceiling() {
    let store = test_store_with_data();
    let result = tool_query(
        &store,
        &serde_json::json!({ "query": "SELECT ?name WHERE { ?s <http://example.org/name> ?name }" }),
    )
    .unwrap();
    assert_eq!(result["count"], 2);
    assert_eq!(result["truncated"], false);
}

#[test]
fn test_tool_episode_surfaces_resolution_hints() {
    // hq-uye: the episode handler must read the store's resolution policy and
    // surface dedup hints in its response (the engine was previously inert).
    let mut store = Store::open_in_memory().unwrap();
    *store.resolution_config_mut() = crate::config::ResolutionConfig {
        enabled: true,
        threshold: 0.85,
        top_k: 3,
        strict_mode: false,
    };

    let ep = serde_json::json!({
        "name": "ep-1",
        "nodes": [{"name": "Grafana", "type": "WebApplication"}],
        "edges": [],
        "timestamp": "2026-04-04T12:00:00Z"
    });
    let first = tool_episode(&mut store, &ep).unwrap();
    // First sighting: nothing to dedup against yet.
    assert_eq!(first["resolution_hints"].as_array().unwrap().len(), 0);

    let ep2 = serde_json::json!({
        "name": "ep-2",
        "nodes": [{"name": "Grafana", "type": "WebApplication"}],
        "edges": [],
        "timestamp": "2026-04-05T12:00:00Z"
    });
    let second = tool_episode(&mut store, &ep2).unwrap();
    let hints = second["resolution_hints"].as_array().unwrap();
    assert_eq!(hints.len(), 1, "duplicate node should produce a hint");
    assert_eq!(hints[0]["node"], "Grafana");
    assert!(!hints[0]["candidates"].as_array().unwrap().is_empty());
}

#[test]
fn test_tool_retract_entity() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .";
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    assert_eq!(store.current_facts().unwrap().len(), 2);

    let input = serde_json::json!({
        "entity": "http://example.org/alice",
        "timestamp": "2026-02-01"
    });
    let result = tool_retract(&mut store, &input).unwrap();
    assert_eq!(result["retracted"], 2);
    assert!(result["tx_id"].as_i64().unwrap() > 0);

    assert_eq!(store.current_facts().unwrap().len(), 0);
}

#[test]
fn test_tool_retract_predicate() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = "@prefix ex: <http://example.org/> .\nex:bob a ex:Person ; ex:name \"Bob\" ; ex:age \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    assert_eq!(store.current_facts().unwrap().len(), 3);

    let input = serde_json::json!({
        "entity": "http://example.org/bob",
        "predicate": "http://example.org/name",
        "timestamp": "2026-02-01"
    });
    let result = tool_retract(&mut store, &input).unwrap();
    assert_eq!(result["retracted"], 1);

    assert_eq!(store.current_facts().unwrap().len(), 2);
}

/// Through the JSON boundary, where `json_to_value` is what turns a bare string
/// into `Value::Str`: a bare-string object for an IRI edge must be
/// REFUSED loudly, not reported as `{"retracted": 0}`. Then the `{"iri": ...}`
/// shape the error teaches must actually retract the edge — so re-parenting,
/// which is retract-old-edge then assert-new, is unblocked end to end.
#[test]
fn test_tool_retract_bare_string_for_an_iri_edge_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-crew",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [
                {"name": "kprobe-a", "type": "CrewMember"},
                {"name": "kprobe-b", "type": "CrewMember"}
            ],
            "edges": [{"source": "kprobe-a", "target": "kprobe-b", "relation": "reports_to"}]
        }),
    )
    .unwrap();
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));

    // The footgun: value as a BARE STRING. json_to_value -> Value::Str, which can
    // never equal the stored Value::Ref, so nothing matches. Must error, not 0.
    let footgun = tool_retract(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}reports_to"),
            "value": format!("{NS}kprobe-b")
        }),
    );
    let err = footgun.expect_err("a bare-string object for an IRI edge must be refused");
    assert!(
        err.to_string().contains("string literal"),
        "error must name the mismatch: {err}"
    );
    // The edge SURVIVED the refusal (no silent partial write).
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));

    // The shape the error teaches works: {"iri": ...} retracts the one edge.
    let ok = tool_retract(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}reports_to"),
            "value": {"iri": format!("{NS}kprobe-b")}
        }),
    )
    .unwrap();
    assert_eq!(
        ok["retracted"], 1,
        "the correctly shaped object retracts the edge"
    );
    assert!(!ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));
}

/// Seed a small org: a `reports_to` b, with c (and optionally more) available as
/// re-parent targets. Shared by the /set acceptance tests.
fn seed_reports_to(store: &mut Store, extra_edges: &[(&str, &str)]) {
    let mut edges = vec![serde_json::json!(
        {"source": "kprobe-a", "target": "kprobe-b", "relation": "reports_to"}
    )];
    for (s, t) in extra_edges {
        edges.push(serde_json::json!({"source": s, "target": t, "relation": "reports_to"}));
    }
    tool_episode(
        store,
        &serde_json::json!({
            "name": "ep-set-crew",
            "timestamp": "2026-04-01T00:00:00Z",
            "nodes": [
                {"name": "kprobe-a", "type": "CrewMember"},
                {"name": "kprobe-b", "type": "CrewMember"},
                {"name": "kprobe-c", "type": "CrewMember"},
                {"name": "kprobe-d", "type": "CrewMember"}
            ],
            "edges": edges
        }),
    )
    .unwrap();
}

/// The bead's headline acceptance: /set re-parents `reports_to` from B to C in
/// ONE call and ONE transaction; afterwards exactly one edge exists.
#[test]
fn test_tool_set_reparents_in_one_call_one_tx() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[]);
    let tx_before = store.list_transactions().unwrap().len();

    let result = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}reports_to"),
            "value": {"iri": format!("{NS}kprobe-c")},
            "timestamp": "2026-04-02T00:00:00Z"
        }),
    )
    .unwrap();
    assert_eq!(result["retracted"], 1);
    assert_eq!(result["asserted"], 1);
    assert!(result["tx_id"].as_i64().unwrap() > 0);

    // Exactly one edge, and it is the new one.
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-c>")
    ));
    assert!(!ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));

    // ONE transaction carried both halves — no window with zero edges exists
    // in the log, and a crash between "calls" is unrepresentable.
    let tx_after = store.list_transactions().unwrap().len();
    assert_eq!(
        tx_after,
        tx_before + 1,
        "retract + assert must ride a single transaction"
    );
}

/// SINGLE-VALUE semantics: an accidentally multi-valued predicate (two
/// supervisors — the exact state forgetting the retract half creates) is
/// repaired by one /set: ALL current objects replaced.
#[test]
fn test_tool_set_replaces_all_current_values() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[("kprobe-a", "kprobe-c")]);
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-c>")
    ));

    let result = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}reports_to"),
            "value": {"iri": format!("{NS}kprobe-d")}
        }),
    )
    .unwrap();
    assert_eq!(result["retracted"], 2, "both stale supervisors retracted");
    assert_eq!(result["asserted"], 1);

    for gone in ["kprobe-b", "kprobe-c"] {
        assert!(!ask(
            &store,
            &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}{gone}>")
        ));
    }
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-d>")
    ));
}

/// Setting the already-sole-current value is an idempotent no-op: no
/// transaction is written (a retract+reassert of the same value would churn
/// the bitemporal history for nothing).
#[test]
fn test_tool_set_idempotent_noop() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[]);
    let tx_before = store.list_transactions().unwrap().len();

    let result = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}reports_to"),
            "value": {"iri": format!("{NS}kprobe-b")}
        }),
    )
    .unwrap();
    assert_eq!(result["tx_id"], 0);
    assert_eq!(result["retracted"], 0);
    assert_eq!(result["asserted"], 0);
    assert_eq!(store.list_transactions().unwrap().len(), tx_before);
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));
}

/// The vqy9 footgun, write-side: a bare string aimed at an IRI edge must be a
/// LOUD refusal — otherwise /set would assert a Str literal no traversal can
/// follow, replacing a real edge with a mis-shaped one.
#[test]
fn test_tool_set_bare_string_for_an_iri_edge_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[]);

    let footgun = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}reports_to"),
            "value": format!("{NS}kprobe-c")
        }),
    );
    let err = footgun.expect_err("a bare-string object for an IRI edge must be refused");
    assert!(
        err.to_string().contains("string literal"),
        "error must name the mismatch and teach the {{\"iri\"}} shape: {err}"
    );
    // Nothing was replaced or half-written: the original edge survives intact.
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}reports_to> <{NS}kprobe-b>")
    ));
}

/// URL-valued STRING literals are legitimate: a predicate that already holds
/// Strs accepts an IRI-shaped bare string (the guard scopes to ref-only and
/// empty predicates, same as `retract_triples`). Caught in production: the
/// first real supersede batch — 60 traefik backend URLs — was refused by an
/// over-broad guard that treated every IRI-shaped string as a mistake.
#[test]
fn test_tool_set_url_literal_on_str_predicate_allowed() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[]);

    // Seed a Str-holding predicate (a template-ish placeholder, like the
    // real defect this batch was fixing).
    let first = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}backend"),
            "value": {"str": "http://PLACEHOLDER:3000"}
        }),
    )
    .unwrap();
    assert_eq!(first["asserted"], 1);

    // Superseding with a bare IRI-shaped string must now be ALLOWED — the
    // predicate holds Strs, so a string is the right shape here.
    let fixed = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}backend"),
            "value": "http://192.0.2.7:3000"
        }),
    )
    .unwrap();
    assert_eq!(fixed["retracted"], 1);
    assert_eq!(fixed["asserted"], 1);
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}backend> \"http://192.0.2.7:3000\"")
    ));
}

/// {"str": ...} is a STATED literal intent — it must disarm the IRI-shape
/// heuristic even on an EMPTY predicate, where a bare IRI-shaped string is
/// still refused (first-write of a URL-valued literal must be expressible).
#[test]
fn test_tool_set_explicit_str_tag_is_an_escape_hatch() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[]);

    // Bare IRI-shaped string on an EMPTY predicate: still the loud refusal.
    let bare = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}externalUrl"),
            "value": "http://192.0.2.9:8080"
        }),
    );
    assert!(
        bare.expect_err("bare IRI-shaped first-write must refuse")
            .to_string()
            .contains("string literal")
    );

    // The tagged spelling states the intent and goes through.
    let tagged = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}externalUrl"),
            "value": {"str": "http://192.0.2.9:8080"}
        }),
    )
    .unwrap();
    assert_eq!(tagged["asserted"], 1);
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}externalUrl> \"http://192.0.2.9:8080\"")
    ));
}

/// /set on an unknown entity is refused (same rule as /retract): a typo'd IRI
/// must not mint an unlabelled orphan node. A NEW predicate on an existing
/// entity, by contrast, is a legitimate first write.
#[test]
fn test_tool_set_unknown_entity_refused_new_predicate_allowed() {
    let mut store = Store::open_in_memory().unwrap();
    seed_reports_to(&mut store, &[]);

    let missing = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-nonexistent"),
            "predicate": format!("{NS}reports_to"),
            "value": {"iri": format!("{NS}kprobe-b")}
        }),
    );
    assert!(
        missing
            .expect_err("unknown entity must refuse")
            .to_string()
            .contains("entity not found"),
    );

    // First-time set of a brand-new predicate: asserted:1, retracted:0.
    let first = tool_set(
        &mut store,
        &serde_json::json!({
            "entity": format!("{NS}kprobe-a"),
            "predicate": format!("{NS}holds_badge"),
            "value": "vault-7"
        }),
    )
    .unwrap();
    assert_eq!(first["retracted"], 0);
    assert_eq!(first["asserted"], 1);
    assert!(ask(
        &store,
        &format!("<{NS}kprobe-a> <{NS}holds_badge> \"vault-7\"")
    ));
}

/// `get` returns the stored turtle byte-for-byte, and an unrecognized action is
/// an ERROR rather than a silent fallback to `list`.
///
/// Both halves matter and both are asserted, because the bug was that the
/// SUCCESS path was indistinguishable from the failure path: a typo'd action
/// returned 200 and a plausible shape list, so a caller could never tell a
/// no-op from a load. A missing action still means `list` — a bare `{}` probe
/// is a documented caller and must keep working (aegis-rtht / aegis-1y3q).
#[test]
fn test_tool_shapes_get_and_unknown_action() {
    let store = Store::open_in_memory().unwrap();
    let shapes = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:S a sh:NodeShape ; sh:targetClass ex:T .\n";

    tool_shapes(
        &store,
        &serde_json::json!({
            "action": "load", "name": "roundtrip", "turtle": shapes, "timestamp": "2026-07-20"
        }),
    )
    .unwrap();

    // get -> exact content back, so a caller can verify WHICH shapes are loaded.
    let got = tool_shapes(
        &store,
        &serde_json::json!({"action": "get", "name": "roundtrip"}),
    )
    .expect("get should succeed for a loaded set");
    assert_eq!(
        got["turtle"], shapes,
        "get must round-trip the turtle exactly"
    );
    assert_eq!(got["name"], "roundtrip");

    // get on an unknown name is an error, not an empty success.
    assert!(
        tool_shapes(
            &store,
            &serde_json::json!({"action": "get", "name": "absent"})
        )
        .is_err(),
        "get on a missing shape set must error"
    );

    // A typo must NOT silently behave like `list`.
    let typo = tool_shapes(&store, &serde_json::json!({"action": "laod"}));
    assert!(
        typo.is_err(),
        "an unknown action must error, not fall through to list"
    );

    // Explicit and implicit list both still work.
    assert_eq!(
        tool_shapes(&store, &serde_json::json!({"action": "list"})).unwrap()["count"],
        1
    );
    assert_eq!(
        tool_shapes(&store, &serde_json::json!({})).unwrap()["count"],
        1,
        "a bare {{}} probe must keep defaulting to list"
    );
}

#[test]
#[cfg(feature = "shacl")]
fn test_tool_shapes_load_and_enforce() {
    let mut store = Store::open_in_memory().unwrap();

    let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
"#;
    let load_input = serde_json::json!({
        "action": "load",
        "name": "person-rules",
        "turtle": shapes,
        "timestamp": "2026-04-04"
    });
    tool_shapes(&store, &load_input).unwrap();

    let list_input = serde_json::json!({ "action": "list" });
    let list_result = tool_shapes(&store, &list_input).unwrap();
    assert_eq!(list_result["count"], 1);

    let good_input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
        "timestamp": "2026-04-04T01:00:00Z"
    });
    let good_result = tool_knot(&mut store, &good_input).unwrap();
    assert_eq!(good_result["conforms"], true);

    let bad_input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:bob a ex:Person .",
        "timestamp": "2026-04-04T02:00:00Z"
    });
    let bad_result = tool_knot(&mut store, &bad_input).unwrap();
    assert_eq!(bad_result["conforms"], false);
}

#[test]
fn test_tool_definitions() {
    let defs = tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"quipu_query"));
    assert!(names.contains(&"quipu_knot"));
    assert!(names.contains(&"quipu_cord"));
    assert!(names.contains(&"quipu_unravel"));
    assert!(names.contains(&"quipu_impact"));
    assert!(names.contains(&"quipu_validate"));
    assert!(names.contains(&"quipu_search"));
    assert!(names.contains(&"quipu_hybrid_search"));
    assert!(names.contains(&"quipu_unified_search"));
    assert!(names.contains(&"quipu_resolve_entity"));
    assert!(names.contains(&"quipu_episode"));
    assert!(names.contains(&"quipu_retract"));
    assert!(names.contains(&"quipu_set"));
    assert!(names.contains(&"quipu_retract_episode"));
    assert!(names.contains(&"quipu_shapes"));
    assert!(names.contains(&"quipu_search_nodes"));
    assert!(names.contains(&"quipu_search_facts"));
    assert!(names.contains(&"quipu_episodes_complete"));
    assert!(names.contains(&"quipu_propose_schema_change"));
    assert!(names.contains(&"quipu_list_proposals"));
    assert!(names.contains(&"quipu_accept_proposal"));
    assert!(names.contains(&"quipu_reject_proposal"));
    // Previously documented but missing from the manifest (hq-6v4).
    assert!(names.contains(&"quipu_project"));
    assert!(names.contains(&"quipu_context"));
    // Named-query catalog tool (hq-h75).
    assert!(names.contains(&"quipu_ask"));
    // Graph report tool (hq-ct27).
    assert!(names.contains(&"quipu_report"));

    // quipu_load_ontology is only advertised when the `owl` feature compiles in
    // its handler (hq-8wd) — otherwise the call would always fail.
    // quipu_export (#36) then quipu_graph (render-ready node-link projection)
    // bring the base to 28.
    assert!(names.contains(&"quipu_export"));
    assert!(names.contains(&"quipu_graph"));
    #[cfg(feature = "owl")]
    {
        assert_eq!(defs.len(), 31);
        assert!(names.contains(&"quipu_load_ontology"));
    }
    #[cfg(not(feature = "owl"))]
    {
        assert_eq!(defs.len(), 30);
        assert!(!names.contains(&"quipu_load_ontology"));
    }

    // The only scoping parameters vector search has must be DISCOVERABLE from the
    // manifest (aegis-il4g): `tool_search` reads `group_ids` and `entity_type` and
    // changes results by them, but they were absent from the advertised schema, so
    // a schema-driven MCP client could not find the one control the search exposes.
    let search = defs
        .iter()
        .find(|d| d["name"] == "quipu_search")
        .expect("quipu_search must be advertised");
    let props = &search["inputSchema"]["properties"];
    assert!(
        props.get("group_ids").is_some(),
        "quipu_search must advertise group_ids"
    );
    assert!(
        props.get("entity_type").is_some(),
        "quipu_search must advertise entity_type"
    );
}

#[test]
#[cfg(not(feature = "owl"))]
fn readme_mcp_tool_counts_match_the_manifest() {
    // The README stated the MCP tool count four times and two of them had drifted
    // (22 vs the real 25) — the number an integrator uses to check their client
    // loaded the full manifest. Pin EVERY count mention to tool_definitions() so
    // they cannot disagree again. Runs on the default (no-owl) build, where the
    // primary count is 25 and the parenthetical is "(N with owl)" = 26.
    let base = tool_definitions().len();
    let with_owl = base + 1; // quipu_load_ontology is the only owl-gated tool.
    let readme =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
            .unwrap();

    // "(<N> tools)" — the architecture diagram.
    for cap in counts_before(&readme, " tools)") {
        assert_eq!(
            cap, base,
            "README '({cap} tools)' disagrees with the {base}-tool manifest"
        );
    }
    // "<N> MCP tools" — prose mentions.
    for cap in counts_before(&readme, " MCP tools") {
        assert_eq!(
            cap, base,
            "README '{cap} MCP tools' disagrees with the {base}-tool manifest"
        );
    }
    // "MCP tools (<N>; <M> with `owl`)" — the feature matrix row. Identified by
    // the `(N;` form, which the prose "MCP tools (M with owl)" mentions do not have.
    let mut matrix_primary = None;
    for (idx, _) in readme.match_indices("MCP tools (") {
        let after = &readme[idx + "MCP tools (".len()..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if after[digits.len()..].starts_with(';') {
            matrix_primary = digits.parse::<usize>().ok();
            assert!(
                after.contains(&format!("{with_owl} with")),
                "feature-matrix owl count must be {with_owl}"
            );
        }
    }
    assert_eq!(
        matrix_primary,
        Some(base),
        "feature-matrix 'MCP tools (N; ...)' primary count must be {base}"
    );
}

/// Every integer that appears immediately before each occurrence of `marker`.
/// Avoids a regex dep — the tool-count mentions are always `<digits><marker>`.
#[cfg(not(feature = "owl"))]
fn counts_before(hay: &str, marker: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (idx, _) in hay.match_indices(marker) {
        let digits: String = hay[..idx]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(n) = digits.parse() {
            out.push(n);
        }
    }
    out
}

// ── Schema evolution proposal tool tests ────────────────────────────

#[test]
fn test_propose_and_list_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "kind": "shape",
        "target": "PersonShape",
        "diff": "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:PersonShape a sh:NodeShape .",
        "rationale": "Need to add email property",
        "proposer": "agent/test",
        "trigger_ref": "val-fail-1",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &input).unwrap();
    assert_eq!(result["status"], "pending");
    let id = result["proposal_id"].as_i64().unwrap();
    assert!(id > 0);

    // List all proposals — should find exactly one.
    let list_result = super::proposal::tool_list_proposals(&store, &serde_json::json!({})).unwrap();
    assert_eq!(list_result["count"], 1);
    assert_eq!(list_result["proposals"][0]["id"], id);
    assert_eq!(list_result["proposals"][0]["kind"], "shape");
    assert_eq!(list_result["proposals"][0]["target"], "PersonShape");
    assert_eq!(list_result["proposals"][0]["proposer"], "agent/test");
    assert_eq!(list_result["proposals"][0]["status"], "pending");

    // Filter by status.
    let pending =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "pending" }))
            .unwrap();
    assert_eq!(pending["count"], 1);

    let accepted =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "accepted" }))
            .unwrap();
    assert_eq!(accepted["count"], 0);
}

#[test]
#[cfg(feature = "shacl")]
fn test_accept_proposal_roundtrip() {
    let store = Store::open_in_memory().unwrap();

    // Create a valid shape proposal.
    let shape_turtle = r#"@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .
"#;
    let propose = serde_json::json!({
        "kind": "shape",
        "target": "PersonShape",
        "diff": shape_turtle,
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Accept it.
    let accept = serde_json::json!({
        "id": id,
        "decided_by": "aegis/crew/braino",
        "note": "Approved — looks correct",
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let accept_result = super::proposal::tool_accept_proposal(&store, &accept).unwrap();
    assert_eq!(accept_result["status"], "accepted");
    assert_eq!(accept_result["proposal_id"], id);

    // Verify the shape was loaded.
    let shapes = store.list_shapes().unwrap();
    assert!(shapes.iter().any(|(name, _, _)| name == "PersonShape"));
}

#[test]
fn test_reject_proposal_roundtrip() {
    let store = Store::open_in_memory().unwrap();

    let propose = serde_json::json!({
        "kind": "property",
        "target": "ex:dangerousField",
        "diff": "ex:dangerousField rdfs:range xsd:string .",
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Reject it.
    let reject = serde_json::json!({
        "id": id,
        "note": "Too permissive — needs cardinality constraints",
        "decided_by": "agent/reviewer",
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let reject_result = super::proposal::tool_reject_proposal(&store, &reject).unwrap();
    assert_eq!(reject_result["status"], "rejected");
    assert_eq!(reject_result["proposal_id"], id);

    // Verify it shows as rejected in listing.
    let list =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "rejected" }))
            .unwrap();
    assert_eq!(list["count"], 1);
}

#[test]
#[cfg(feature = "shacl")]
fn test_accept_invalid_turtle_stays_pending() {
    let store = Store::open_in_memory().unwrap();

    // Create a shape proposal with invalid Turtle.
    let propose = serde_json::json!({
        "kind": "shape",
        "target": "BadShape",
        "diff": "this is not valid turtle {{{{",
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Accepting should fail — invalid Turtle.
    let accept = serde_json::json!({
        "id": id,
        "decided_by": "agent/reviewer",
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let err = super::proposal::tool_accept_proposal(&store, &accept).unwrap_err();
    assert!(err.to_string().contains("invalid"));

    // Proposal should still be pending.
    let list =
        super::proposal::tool_list_proposals(&store, &serde_json::json!({ "status": "pending" }))
            .unwrap();
    assert_eq!(list["count"], 1);
}

#[test]
fn test_reject_missing_note_errors() {
    let store = Store::open_in_memory().unwrap();

    let propose = serde_json::json!({
        "kind": "class",
        "target": "ex:NewClass",
        "diff": "ex:NewClass a rdfs:Class .",
        "proposer": "agent/test",
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = super::proposal::tool_propose_schema_change(&store, &propose).unwrap();
    let id = result["proposal_id"].as_i64().unwrap();

    // Reject without note should error.
    let reject = serde_json::json!({
        "id": id,
        "decided_by": "agent/reviewer",
        "timestamp": "2026-04-13T01:00:00Z"
    });
    let err = super::proposal::tool_reject_proposal(&store, &reject).unwrap_err();
    assert!(err.to_string().contains("note"));
}

#[test]
fn test_propose_missing_required_fields_errors() {
    let store = Store::open_in_memory().unwrap();

    // Missing 'kind'.
    let input = serde_json::json!({
        "target": "X", "diff": "x", "proposer": "test"
    });
    assert!(super::proposal::tool_propose_schema_change(&store, &input).is_err());

    // Missing 'proposer'.
    let input = serde_json::json!({
        "kind": "shape", "target": "X", "diff": "x"
    });
    assert!(super::proposal::tool_propose_schema_change(&store, &input).is_err());
}

#[test]
#[cfg(feature = "shacl")]
fn test_knot_validation_failure_includes_proposal_hint() {
    let mut store = Store::open_in_memory().unwrap();
    let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
"#;
    let input = serde_json::json!({
        "turtle": "@prefix ex: <http://example.org/> .\nex:bad a ex:Person .",
        "shapes": shapes,
        "timestamp": "2026-04-13T00:00:00Z"
    });
    let result = tool_knot(&mut store, &input).unwrap();
    assert_eq!(result["conforms"], false);
    assert_eq!(
        result["hint"],
        "propose a schema change via quipu_propose_schema_change"
    );
}

#[test]
fn test_extract_type_filter_simple() {
    let sparql = "SELECT ?s WHERE { ?s a <http://example.org/Person> }";
    let filter = super::tools::search::extract_type_filter(sparql);
    assert_eq!(
        filter,
        Some("entity_type = 'http://example.org/Person'".into())
    );
}

#[test]
fn test_extract_type_filter_rdf_type() {
    let sparql = "SELECT ?s WHERE { ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Bot> }";
    let filter = super::tools::search::extract_type_filter(sparql);
    assert_eq!(
        filter,
        Some("entity_type = 'http://example.org/Bot'".into())
    );
}

#[test]
fn test_extract_type_filter_complex_returns_none() {
    // FILTER makes this too complex for pushdown
    let sparql = "SELECT ?s WHERE { ?s a <http://example.org/Person> . FILTER(?s != <http://example.org/bob>) }";
    let filter = super::tools::search::extract_type_filter(sparql);
    assert!(filter.is_none());
}

#[test]
fn test_extract_type_filter_no_type_returns_none() {
    let sparql = "SELECT ?s WHERE { ?s <http://example.org/name> \"Alice\" }";
    let filter = super::tools::search::extract_type_filter(sparql);
    assert!(filter.is_none());
}

#[test]
fn test_hybrid_search_includes_pushdown_filter() {
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let alice_id = store.intern("http://example.org/alice").unwrap();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(alice_id, "Alice", &emb, "2026-01-01")
        .unwrap();

    let input = serde_json::json!({
        "embedding": emb,
        "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        "limit": 5
    });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();

    // Result should include the pushdown_filter field.
    assert_eq!(
        result["pushdown_filter"],
        "entity_type = 'http://example.org/Person'"
    );
    assert_eq!(result["count"], 1);
}

#[test]
fn test_hybrid_search_vector_only() {
    let store = test_store_with_data();

    // Embed an entity for vector search.
    let eid = store.intern("http://example.org/alice").unwrap();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(eid, "Alice the person", &emb, "2026-01-01")
        .unwrap();

    // Hybrid search with no SPARQL filter — behaves like plain vector search.
    let input = serde_json::json!({
        "embedding": emb,
        "limit": 5
    });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
    assert!(result["sparql_candidates"].is_null());
}

#[test]
fn test_hybrid_search_with_sparql_filter() {
    let mut store = Store::open_in_memory().unwrap();

    // Ingest two entities.
    let ttl = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .\nex:bob a ex:Bot ; ex:name \"Bob\" .";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // Embed both.
    let alice_id = store.intern("http://example.org/alice").unwrap();
    let bob_id = store.intern("http://example.org/bob").unwrap();
    let emb_a: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    let emb_b: Vec<f32> = (0..8).map(|i| (1.1 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(alice_id, "Alice", &emb_a, "2026-01-01")
        .unwrap();
    store
        .embed_entity(bob_id, "Bob", &emb_b, "2026-01-01")
        .unwrap();

    // Hybrid search: SPARQL filters to only Person, vector ranks.
    let input = serde_json::json!({
        "embedding": emb_a,
        "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        "limit": 5
    });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();
    assert_eq!(result["count"], 1); // Only Alice (Person), not Bob (Bot).
    assert_eq!(result["sparql_candidates"], 1);
    let entity = result["results"][0]["entity"].as_str().unwrap();
    assert!(entity.contains("alice"));
}

#[test]
fn test_search_results_include_source_field() {
    let store = test_store_with_data();
    let eid = store.intern("http://example.org/alice").unwrap();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    store
        .embed_entity(eid, "Alice the person", &emb, "2026-01-01")
        .unwrap();

    // tool_search results should have source: "knowledge"
    let input = serde_json::json!({ "embedding": emb, "limit": 5 });
    let result = super::tools::tool_search(&store, &input).unwrap();
    assert_eq!(result["results"][0]["source"], "knowledge");

    // tool_hybrid_search results should also have source: "knowledge"
    let input = serde_json::json!({ "embedding": emb, "limit": 5 });
    let result = super::tools::tool_hybrid_search(&store, &input).unwrap();
    assert_eq!(result["results"][0]["source"], "knowledge");
}

#[test]
fn test_tool_search_dedupes_by_entity() {
    // aegis-a1s5: an entity has one embedding row per fact/text (PK is
    // (entity_id, valid_from)), so the raw top-N was returning the same entity
    // 2-3x and wasting the result slots. Each entity must appear at most once,
    // keeping its highest-scoring row, and the limit must be filled with
    // *distinct* entities.
    let store = test_store_with_data();
    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    let other: Vec<f32> = (0..8).map(|i| (2.0 + i as f32 * 0.1).sin()).collect();

    let alice = store.intern("http://example.org/alice").unwrap();
    for (n, day) in ["2026-01-01", "2026-01-02", "2026-01-03"]
        .iter()
        .enumerate()
    {
        store
            .embed_entity(alice, &format!("Alice fact {n}"), &emb, day)
            .unwrap();
    }
    let bob = store.intern("http://example.org/bob").unwrap();
    store
        .embed_entity(bob, "Bob", &other, "2026-01-01")
        .unwrap();

    let result = tool_search(
        &store,
        &serde_json::json!({ "embedding": emb, "limit": 10 }),
    )
    .unwrap();
    let results = result["results"].as_array().unwrap();

    let mut entities: Vec<&str> = results
        .iter()
        .map(|r| r["entity"].as_str().unwrap())
        .collect();
    let before = entities.len();
    entities.sort_unstable();
    entities.dedup();
    assert_eq!(
        before,
        entities.len(),
        "each entity must appear at most once"
    );
    assert_eq!(result["count"], 2, "alice (deduped) + bob");

    // A limit smaller than the duplicate count must still return distinct
    // entities, not one entity's rows filling every slot.
    let limited =
        tool_search(&store, &serde_json::json!({ "embedding": emb, "limit": 2 })).unwrap();
    let limited = limited["results"].as_array().unwrap();
    assert_eq!(limited.len(), 2);
    assert_ne!(limited[0]["entity"], limited[1]["entity"]);
}

/// Set up THREE entities that share ONE embedding, so an unfiltered vector search
/// returns all three:
///   - `AlphaSvc`, `BetaSvc`: created by episodes, in provenance groups rig-a / rig-b.
///   - `GammaSvc`: written via `tool_knot` with NO episode, so it is UNGROUPED
///     (it has no `prov:wasGeneratedBy` activity to trace a group through).
///
/// `GammaSvc` is the case the old two-episode fixture could not express, and the
/// reason `group_ids` behaviour was untestable: a group scope must DROP it (it
/// traces back to no activity), not return it (aegis-il4g). Returns the shared
/// embedding. (hq-93d / aegis-il4g test fixture)
///
/// "group", not "tenant": this is best-effort PROVENANCE scoping, not an isolation
/// boundary (docs/design/group-isolation.md).
fn two_group_store() -> (Store, Vec<f32>) {
    let mut store = Store::open_in_memory().unwrap();
    let ns = crate::namespace::DEFAULT_BASE_NS;
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-a", "group_id": "rig-a",
            "nodes": [{"name": "AlphaSvc", "type": "WebApplication"}], "edges": [],
            "timestamp": "2026-04-04T00:00:00Z"
        }),
    )
    .unwrap();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-b", "group_id": "rig-b",
            "nodes": [{"name": "BetaSvc", "type": "Database"}], "edges": [],
            "timestamp": "2026-04-04T00:00:00Z"
        }),
    )
    .unwrap();
    // An UNGROUPED fact: written via /knot, no episode, hence no
    // prov:wasGeneratedBy — the exact shape a group filter must drop.
    tool_knot(
        &mut store,
        &serde_json::json!({
            "turtle": format!("<{ns}GammaSvc> a <{ns}WebApplication> ."),
            "timestamp": "2026-04-04T00:00:00Z",
            "actor": "test", "source": "unit-test"
        }),
    )
    .unwrap();

    let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();
    for name in ["AlphaSvc", "BetaSvc", "GammaSvc"] {
        let id = store.intern(&format!("{ns}{name}")).unwrap();
        store
            .embed_entity(id, name, &emb, "2026-04-04T00:00:00Z")
            .unwrap();
    }
    (store, emb)
}

#[test]
fn test_tool_search_honors_group_ids() {
    // aegis-il4g / hq-93d: `group_ids` is a best-effort PROVENANCE filter, not
    // isolation. Two behaviours it must have, and the second was previously
    // untestable because the fixture had no ungrouped entity:
    //   1. an unscoped search returns everything — grouped AND ungrouped;
    //   2. a group scope narrows to that group's episode-provenanced entities and
    //      DROPS ungrouped /knot facts (they trace back to no activity). It does
    //      NOT return them — the design doc used to state this backwards.
    let (store, emb) = two_group_store();

    // Unscoped → all three visible, including the ungrouped GammaSvc.
    let all = tool_search(
        &store,
        &serde_json::json!({ "embedding": emb, "limit": 10 }),
    )
    .unwrap();
    assert_eq!(all["scoped"], false);
    assert_eq!(
        all["count"], 3,
        "unscoped search sees every group AND ungrouped facts"
    );

    // Scoped to rig-a → ONLY AlphaSvc. Not BetaSvc (other group), and not GammaSvc
    // (ungrouped — the case the old fixture could not reach). This drops-not-returns
    // assertion goes RED if the required prov join is ever loosened to OPTIONAL.
    let scoped = tool_search(
        &store,
        &serde_json::json!({ "embedding": emb, "limit": 10, "group_ids": ["rig-a"] }),
    )
    .unwrap();
    assert_eq!(scoped["scoped"], true);
    let results = scoped["results"].as_array().unwrap();
    let entities: Vec<&str> = results
        .iter()
        .map(|r| r["entity"].as_str().unwrap())
        .collect();
    assert_eq!(
        results.len(),
        1,
        "group scope must drop the other group AND ungrouped facts"
    );
    assert!(entities[0].contains("AlphaSvc"));
    assert!(
        !entities.iter().any(|e| e.contains("GammaSvc")),
        "an ungrouped /knot fact must be DROPPED from a group scope, not returned"
    );
}

#[test]
fn test_tool_search_scope_is_not_truncated_by_limit() {
    // The scope set is a provenance-scope ALLOW-LIST, so it must cover the whole
    // group regardless of the caller's `limit`. It used to be built with
    // `LIMIT oversample(limit)`, which capped it at a handful of arbitrary
    // entities — on the live graph that made 357 of one group's 457 entities
    // permanently unsearchable, and the smaller the caller's limit the more of
    // the group vanished.
    let mut store = Store::open_in_memory().unwrap();
    let n = 30;
    let nodes: Vec<_> = (0..n)
        .map(|i| serde_json::json!({"name": format!("Svc{i:02}"), "type": "WebApplication"}))
        .collect();
    tool_episode(
        &mut store,
        &serde_json::json!({
            "name": "ep-big", "group_id": "big-rig",
            "nodes": nodes, "edges": [],
            "timestamp": "2026-04-04T00:00:00Z"
        }),
    )
    .unwrap();

    // Each entity gets its own embedding, so querying with entity i's vector
    // makes entity i the unambiguous nearest neighbour.
    let ns = crate::namespace::DEFAULT_BASE_NS;
    let embedding_of = |i: usize| -> Vec<f32> {
        (0..n)
            .map(|d| if d == i { 1.0 } else { 0.0 })
            .collect::<Vec<f32>>()
    };
    for i in 0..n {
        let id = store.intern(&format!("{ns}Svc{i:02}")).unwrap();
        store
            .embed_entity(
                id,
                &format!("Svc{i:02}"),
                &embedding_of(i),
                "2026-04-04T00:00:00Z",
            )
            .unwrap();
    }

    // Every in-group entity must be findable, even at limit=1 — the limit bounds
    // the RESULTS, never the set of entities allowed to produce them.
    for i in 0..n {
        let result = tool_search(
            &store,
            &serde_json::json!({
                "embedding": embedding_of(i), "limit": 1, "group_ids": ["big-rig"]
            }),
        )
        .unwrap();
        let results = result["results"].as_array().unwrap();
        assert_eq!(
            results.len(),
            1,
            "Svc{i:02} is in group big-rig but a scoped search could not reach it"
        );
        assert!(
            results[0]["entity"]
                .as_str()
                .unwrap()
                .ends_with(&format!("Svc{i:02}")),
            "expected Svc{i:02}, got {}",
            results[0]["entity"]
        );
    }
}

#[test]
fn test_tool_search_honors_entity_type() {
    // hq-93d: scoping by entity_type restricts to that class.
    let (store, emb) = two_group_store();
    let ns = crate::namespace::DEFAULT_BASE_NS;

    let scoped = tool_search(
        &store,
        &serde_json::json!({
            "embedding": emb, "limit": 10,
            "entity_type": format!("{ns}Database")
        }),
    )
    .unwrap();
    assert_eq!(scoped["scoped"], true);
    let results = scoped["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "only the Database-typed entity");
    assert!(results[0]["entity"].as_str().unwrap().contains("BetaSvc"));
}

/// Deterministic embedding provider for testing query-text auto-embedding.
struct TestProvider;

impl EmbeddingProvider for TestProvider {
    fn embed_text(&self, text: &str) -> QResult<Vec<f32>> {
        let seed = text.len() as f32;
        Ok((0..8).map(|i| (seed + i as f32 * 0.1).sin()).collect())
    }

    fn dimension(&self) -> usize {
        8
    }
}

#[test]
fn test_search_with_query_text() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" ; rdfs:comment "A software engineer" .
ex:bob rdfs:label "Bob" ; rdfs:comment "A data scientist" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Search using query text (auto-embedded by provider).
    let input = serde_json::json!({ "query": "software engineer", "limit": 5 });
    let result = tool_search(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    assert_eq!(result["results"][0]["source"], "knowledge");
}

#[test]
fn test_search_query_text_without_provider_errors() {
    let store = Store::open_in_memory().unwrap();

    // No embedding provider → query-text search should fail with a clear message.
    let input = serde_json::json!({ "query": "software engineer" });
    let err = tool_search(&store, &input).unwrap_err();
    assert!(err.to_string().contains("no embedding provider"));
}

#[test]
fn test_search_missing_both_params_errors() {
    let store = Store::open_in_memory().unwrap();

    // Neither query nor embedding → error.
    let input = serde_json::json!({ "limit": 5 });
    let err = tool_search(&store, &input).unwrap_err();
    assert!(err.to_string().contains("missing"));
}

#[test]
fn test_search_explicit_embedding_preferred_over_query() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
ex:alice rdfs:label "Alice" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // When both embedding and query are provided, embedding wins.
    let emb: Vec<f32> = (0..8).map(|i| (5.0 + i as f32 * 0.1).sin()).collect();
    let input = serde_json::json!({
        "embedding": emb,
        "query": "ignored because embedding takes precedence",
        "limit": 5
    });
    let result = tool_search(&store, &input).unwrap();
    // Should succeed (uses explicit embedding).
    assert!(result["count"].as_u64().is_some());
}

#[test]
fn test_hybrid_search_with_query_text() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; rdfs:label "Alice" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Hybrid search with query text + SPARQL filter.
    let input = serde_json::json!({
        "query": "Alice",
        "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        "limit": 5
    });
    let result = tool_hybrid_search(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_hybrid_search_query_text_without_provider_errors() {
    let store = Store::open_in_memory().unwrap();

    let input = serde_json::json!({ "query": "test" });
    let err = tool_hybrid_search(&store, &input).unwrap_err();
    assert!(err.to_string().contains("no embedding provider"));
}

// ── search module tests (text-matching search_nodes / search_facts) ──

#[test]
fn test_tool_search_nodes_basic() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let nodes = result["nodes"].as_array().unwrap();
    // At least one node should have "alice" in its IRI.
    assert!(
        nodes
            .iter()
            .any(|n| n["iri"].as_str().unwrap().contains("alice"))
    );
}

#[test]
fn test_tool_search_nodes_with_type_filter() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "entity_type_filter": "http://example.org/Person",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_tool_search_nodes_no_match() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "zzz_nonexistent_entity",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn test_tool_search_nodes_returns_label_and_types() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &input).unwrap();
    let nodes = result["nodes"].as_array().unwrap();
    let alice = nodes
        .iter()
        .find(|n| n["iri"].as_str().unwrap().contains("alice"))
        .unwrap();
    // Should have types populated.
    assert!(!alice["types"].as_array().unwrap().is_empty());
}

#[test]
fn test_tool_search_nodes_with_group_ids() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "test-ep",
        "source": "test",
        "group_id": "my-group",
        "timestamp": "2026-04-04T12:00:00Z",
        "nodes": [
            {"name": "ServerAlpha", "type": "Server", "description": "Production server"}
        ],
        "edges": []
    });
    super::tools::tool_episode(&mut store, &input).unwrap();

    // Search with matching group_id.
    let search_input = serde_json::json!({
        "query": "ServerAlpha",
        "group_ids": ["my-group"],
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &search_input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);

    // Search with non-matching group_id.
    let search_input = serde_json::json!({
        "query": "ServerAlpha",
        "group_ids": ["wrong-group"],
        "max_results": 10
    });
    let result = super::search::tool_search_nodes(&store, &search_input).unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn test_tool_search_facts_basic() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "name",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let facts = result["facts"].as_array().unwrap();
    // Should find name predicates.
    assert!(
        facts
            .iter()
            .any(|f| f["predicate"].as_str().unwrap().contains("name"))
    );
}

#[test]
fn test_tool_search_facts_by_value() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "Alice",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let facts = result["facts"].as_array().unwrap();
    assert!(
        facts
            .iter()
            .any(|f| f["target"].as_str().unwrap() == "Alice")
    );
}

#[test]
fn test_tool_search_facts_no_match() {
    let store = test_store_with_data();
    let input = serde_json::json!({
        "query": "zzz_nonexistent_predicate",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &input).unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn test_tool_search_facts_with_provenance() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "deploy-ep",
        "source": "test",
        "group_id": "ops-group",
        "timestamp": "2026-04-04T12:00:00Z",
        "nodes": [
            {"name": "AppBeta", "type": "Application"},
            {"name": "HostGamma", "type": "Host"}
        ],
        "edges": [
            {"source": "AppBeta", "target": "HostGamma", "relation": "deployed_on"}
        ]
    });
    super::tools::tool_episode(&mut store, &input).unwrap();

    let search_input = serde_json::json!({
        "query": "deployed_on",
        "max_results": 10
    });
    let result = super::search::tool_search_facts(&store, &search_input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
    let facts = result["facts"].as_array().unwrap();
    let deploy_fact = facts
        .iter()
        .find(|f| f["predicate"].as_str().unwrap().contains("deployed_on"))
        .unwrap();
    // Should have provenance from the episode.
    assert!(!deploy_fact["provenance"].is_null());
}

// ── Graphiti-compatible endpoint tests ────────────────────────────

#[test]
fn test_search_nodes_sparql_fallback() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
@prefix aegis: <https://aegis.dev/ns/> .

ex:tapestry a aegis:WebApplication ;
    rdfs:label "tapestry" ;
    rdfs:comment "Web UI for Gas Town" .
ex:quipu a aegis:KnowledgeGraph ;
    rdfs:label "quipu" ;
    rdfs:comment "AI-native knowledge graph" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Search by label text — no embedding provider, uses SPARQL fallback.
    let input = serde_json::json!({ "query": "tapestry", "max_results": 5 });
    let result = tool_search_nodes(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["nodes"][0]["name"], "tapestry");
}

#[test]
fn test_search_nodes_with_type_filter() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix aegis: <https://aegis.dev/ns/> .

aegis:tapestry a aegis:WebApplication ;
    rdfs:label "tapestry" ;
    rdfs:comment "Web UI" .
aegis:quipu a aegis:KnowledgeGraph ;
    rdfs:label "quipu" ;
    rdfs:comment "Knowledge graph" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Filter to only WebApplication entities.
    let input = serde_json::json!({
        "query": "tapestry",
        "entity_type_filter": "WebApplication",
        "max_results": 5
    });
    let result = tool_search_nodes(&store, &input).unwrap();
    assert_eq!(result["count"], 1);
    assert!(
        result["nodes"][0]["type"]
            .as_str()
            .unwrap()
            .contains("WebApplication")
    );
}

#[test]
fn test_search_nodes_missing_query_errors() {
    let store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({ "max_results": 5 });
    let err = tool_search_nodes(&store, &input).unwrap_err();
    assert!(err.to_string().contains("missing 'query'"));
}

#[test]
fn test_search_nodes_with_vector_search() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(TestProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" ; rdfs:comment "A software engineer" .
ex:bob rdfs:label "Bob" ; rdfs:comment "A data scientist" .
"#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let input = serde_json::json!({ "query": "engineer", "max_results": 5 });
    let result = tool_search_nodes(&store, &input).unwrap();
    assert!(result["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_episodes_complete_basic() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "meeting-notes-2026-04",
        "episode_body": "Discussed the new auth middleware requirements",
        "group_id": "aegis-ontology",
        "source_description": "crew/ellie",
        "timestamp": "2026-04-04T14:00:00Z"
    });
    let result = tool_episodes_complete(&mut store, &input).unwrap();
    assert_eq!(result["episode"], "meeting-notes-2026-04");
    assert!(result["tx_id"].as_i64().unwrap() > 0);
    assert!(result["count"].as_i64().unwrap() >= 1);
}

#[test]
fn test_episodes_complete_minimal() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "quick-note"
    });
    let result = tool_episodes_complete(&mut store, &input).unwrap();
    assert_eq!(result["episode"], "quick-note");
    assert!(result["tx_id"].as_i64().unwrap() > 0);
}

#[test]
fn test_episodes_complete_missing_name_errors() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "episode_body": "some text"
    });
    let err = tool_episodes_complete(&mut store, &input).unwrap_err();
    assert!(err.to_string().contains("missing 'name'"));
}

#[test]
fn test_episodes_complete_provenance_queryable() {
    let mut store = Store::open_in_memory().unwrap();
    let input = serde_json::json!({
        "name": "deploy-v2",
        "episode_body": "Deployed version 2 to production",
        "source_description": "ci/pipeline",
        "timestamp": "2026-04-04T15:00:00Z"
    });
    tool_episodes_complete(&mut store, &input).unwrap();

    // The episode provenance entity should be queryable via SPARQL.
    let q = serde_json::json!({
        "query": "SELECT ?label WHERE { ?s a <http://www.w3.org/ns/prov#Activity> ; <http://www.w3.org/2000/01/rdf-schema#label> ?label }"
    });
    let result = tool_query(&store, &q).unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["rows"][0]["label"], "deploy-v2");
}

#[test]
fn test_cooccurrence_deterministic_set_overlap() {
    // quipu#37: two work-items co-occur iff they share a touched entity, via
    // Bead <-implements- GitCommit -modifies-> entity. Deterministic, not mined.
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "@prefix a: <http://aegis.gastown.local/ontology/> .\n\
        a:c1 a a:GitCommit ; a:implements a:beadT ; a:modifies a:E1, a:E2 .\n\
        a:c2 a a:GitCommit ; a:implements a:beadX ; a:modifies a:E1 .\n\
        a:c3 a a:GitCommit ; a:implements a:beadY ; a:modifies a:E9 .\n";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let input = serde_json::json!({ "work_item": "http://aegis.gastown.local/ontology/beadT" });
    let result = super::tool_cooccurrence(&store, &input).unwrap();

    // beadX shares E1 with beadT; beadY (E9) shares nothing.
    assert_eq!(
        result["count"], 1,
        "exactly one co-occurring work-item: {result:#?}"
    );
    let co = result["cooccurring"].as_array().unwrap();
    assert_eq!(
        co[0]["work_item"],
        "http://aegis.gastown.local/ontology/beadX"
    );
    assert_eq!(co[0]["shared_entities"], 1);

    // Injection guard: a work_item that could break out of <...> is rejected.
    let bad = serde_json::json!({ "work_item": "x> } INSERT {" });
    assert!(super::tool_cooccurrence(&store, &bad).is_err());
}

#[test]
fn test_entity_centric_provenance_named_queries() {
    // quipu#37 entity-centric side: entity_work (what work touched an entity)
    // and cochanged_with (entities sharing a touching work-item), via the same
    // Bead <-implements- GitCommit -modifies-> entity provenance chain, served
    // through the quipu_ask named-query catalog.
    let mut store = Store::open_in_memory().unwrap();
    // c1 implements beadT, touches E1+E2; c2 implements beadX, touches E1;
    // c3 implements beadY, touches E9. So via beadT/E1, E1 co-changes with E2
    // (same commit c1) and — through shared beadT? no — via shared work-items:
    // E1 and E2 share beadT (c1); E1 and E1 excluded; E9 shares nothing with E1.
    let ttl = "@prefix a: <http://aegis.gastown.local/ontology/> .\n\
        a:c1 a a:GitCommit ; a:implements a:beadT ; a:modifies a:E1, a:E2 .\n\
        a:c2 a a:GitCommit ; a:implements a:beadX ; a:modifies a:E1 .\n\
        a:c3 a a:GitCommit ; a:implements a:beadY ; a:modifies a:E9 .\n";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // entity_work(E1): commits c1 and c2 touched E1, implementing beadT and beadX.
    let ew = crate::tool_ask(
        &store,
        &serde_json::json!({
            "name": "entity_work",
            "params": { "entity": "http://aegis.gastown.local/ontology/E1" }
        }),
    )
    .unwrap();
    assert_eq!(
        ew["count"], 2,
        "E1 touched by two commit/bead pairs: {ew:#?}"
    );
    let beads: std::collections::HashSet<String> = ew["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["bead"].as_str().unwrap().to_string())
        .collect();
    assert!(beads.contains("http://aegis.gastown.local/ontology/beadT"));
    assert!(beads.contains("http://aegis.gastown.local/ontology/beadX"));

    // cochanged_with(E1): E2 shares beadT with E1 (both via c1). E9 shares nothing.
    let cc = crate::tool_ask(
        &store,
        &serde_json::json!({
            "name": "cochanged_with",
            "params": { "entity": "http://aegis.gastown.local/ontology/E1" }
        }),
    )
    .unwrap();
    assert_eq!(cc["count"], 1, "only E2 co-changes with E1: {cc:#?}");
    let row = &cc["rows"].as_array().unwrap()[0];
    assert_eq!(row["other"], "http://aegis.gastown.local/ontology/E2");
    assert_eq!(row["shared_workitems"], 1);

    // Injection guard inherited from the catalog's Iri param validation.
    let bad = crate::tool_ask(
        &store,
        &serde_json::json!({
            "name": "cochanged_with",
            "params": { "entity": "x> } INSERT {" }
        }),
    );
    assert!(bad.is_err(), "malformed IRI must be rejected");
}

#[test]
fn test_policy_check_committed_tier_evaluation() {
    // The loom's committed-tier eval: a Policy claim (ASK) over the graph of
    // record yields a Verdict {satisfied|unsatisfied|unknown}, reproducibly.
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "@prefix a: <http://aegis.gastown.local/ontology/> .\n\
        a:sym1 a a:CodeSymbol ; a:hasTest a:test1 .\n\
        a:sym2 a a:CodeSymbol .\n";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let claim = "PREFIX a: <http://aegis.gastown.local/ontology/> ASK { $target a:hasTest ?t }";
    // sym1 has a test -> satisfied
    let r1 = super::tool_policy_check(&store, &serde_json::json!({
        "claim": claim, "target": "http://aegis.gastown.local/ontology/sym1", "predicate_id": "has-test"
    })).unwrap();
    assert_eq!(r1["outcome"], "satisfied", "{r1:#?}");
    assert_eq!(r1["signed"], false);
    assert!(r1["evidence_hash"].as_str().unwrap().starts_with("fnv1a:"));

    // sym2 has no test -> unsatisfied
    let r2 = super::tool_policy_check(&store, &serde_json::json!({
        "claim": claim, "target": "http://aegis.gastown.local/ontology/sym2", "predicate_id": "has-test"
    })).unwrap();
    assert_eq!(r2["outcome"], "unsatisfied", "{r2:#?}");

    // unknown: an evidence probe that's false (target isn't even a CodeSymbol -> no evidence)
    let probe = "PREFIX a: <http://aegis.gastown.local/ontology/> ASK { $target a a:CodeSymbol }";
    let r3 = super::tool_policy_check(
        &store,
        &serde_json::json!({
            "claim": claim, "evidence_probe": probe,
            "target": "http://aegis.gastown.local/ontology/nothere", "predicate_id": "has-test"
        }),
    )
    .unwrap();
    assert_eq!(
        r3["outcome"], "unknown",
        "no-evidence must be unknown, not unsatisfied: {r3:#?}"
    );

    // reproducible: same inputs -> same evidence hash
    let r1b = super::tool_policy_check(&store, &serde_json::json!({
        "claim": claim, "target": "http://aegis.gastown.local/ontology/sym1", "predicate_id": "has-test"
    })).unwrap();
    assert_eq!(
        r1["evidence_hash"], r1b["evidence_hash"],
        "verdict must be reproducible"
    );

    // injection guard on target
    assert!(
        super::tool_policy_check(
            &store,
            &serde_json::json!({
                "claim": claim, "target": "x> } INSERT {"
            })
        )
        .is_err()
    );
}

#[test]
fn test_verifier_registry_authority() {
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "@prefix a: <http://aegis.gastown.local/ontology/> .\n\
        a:reg1 a a:VerifierRegistration ; a:verifier \"quipu\" ; a:attests \"has-test\" .\n\
        a:sym1 a a:CodeSymbol ; a:hasTest a:t1 .\n";
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // quipu IS registered for has-test; NOT for something-else.
    let a = super::tool_verifier_authorized(
        &store,
        &serde_json::json!({"verifier":"quipu","predicate":"has-test"}),
    )
    .unwrap();
    assert_eq!(a["authorized"], true, "{a:#?}");
    let b = super::tool_verifier_authorized(
        &store,
        &serde_json::json!({"verifier":"quipu","predicate":"other"}),
    )
    .unwrap();
    assert_eq!(b["authorized"], false);

    // policy_check surfaces the authority flag: predicate_id has-test -> authorized.
    let claim = "PREFIX a: <http://aegis.gastown.local/ontology/> ASK { $target a:hasTest ?t }";
    let v = super::tool_policy_check(&store, &serde_json::json!({
        "claim": claim, "target": "http://aegis.gastown.local/ontology/sym1", "predicate_id": "has-test"
    })).unwrap();
    assert_eq!(v["outcome"], "satisfied");
    assert_eq!(
        v["verifier_authorized"], true,
        "quipu is registered for has-test: {v:#?}"
    );
}

#[test]
fn test_signed_verdict_end_to_end_root_of_trust() {
    use std::sync::Arc;
    let mut store = Store::open_in_memory().unwrap();
    // v1 signing identity (host-file key in a temp dir).
    let dir = tempfile::tempdir().unwrap();
    let id = crate::signing::SigningIdentity::load(&dir.path().join("k.pk8"), "quipu").unwrap();
    let pubkey = id.public_key_hex();
    store.set_signing_identity(Arc::new(id));

    // Register quipu's pubkey for has-test (the human trust root), + evidence.
    let ttl = format!(
        "@prefix a: <http://aegis.gastown.local/ontology/> .\n\
         a:reg a a:VerifierRegistration ; a:verifier \"quipu\" ; a:attests \"has-test\" ; a:publicKey \"{pubkey}\" .\n\
         a:sym1 a a:CodeSymbol ; a:hasTest a:t1 .\n"
    );
    crate::rdf::ingest_rdf(
        &mut store,
        ttl.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // Evaluate -> SIGNED verdict.
    let claim = "PREFIX a: <http://aegis.gastown.local/ontology/> ASK { $target a:hasTest ?t }";
    let v = super::tool_policy_check(&store, &serde_json::json!({
        "claim": claim, "target": "http://aegis.gastown.local/ontology/sym1", "predicate_id": "has-test"
    })).unwrap();
    assert_eq!(v["outcome"], "satisfied");
    assert_eq!(
        v["signed"], true,
        "a signing identity is set -> verdict must be signed: {v:#?}"
    );
    assert!(v["signature"].as_str().unwrap().len() >= 64);

    // Verify against the registered key -> trusted.
    let ok = super::tool_verdict_verify(&store, &v).unwrap();
    assert_eq!(ok["signature_valid"], true, "{ok:#?}");
    assert_eq!(ok["verifier_authorized"], true);
    assert_eq!(
        ok["trusted"], true,
        "signed by the registered key + authorized: {ok:#?}"
    );

    // Tamper the outcome -> signature no longer valid, not trusted.
    let mut forged = v.clone();
    forged["outcome"] = serde_json::json!("unsatisfied");
    let bad = super::tool_verdict_verify(&store, &forged).unwrap();
    assert_eq!(
        bad["signature_valid"], false,
        "flipping outcome must break the sig"
    );
    assert_eq!(bad["trusted"], false);
}

// ── aegis-fmyi × aegis-arup: the two changes meet in json_to_value ──────
//
// Triple-level retraction matches on exact `Value` equality and parses its
// `value` param with `json_to_value`. Once a language-tagged literal stopped
// being `Str("hello@en")` and became `Value::Lang`, the bare-string form could
// no longer name it — so if `json_to_value` could not express a tagged literal,
// lang-tagged facts would have become UNRETRACTABLE by the precise API.

#[test]
fn triple_retraction_can_name_a_lang_tagged_literal() {
    let mut store = Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut store,
        r#"<http://example.org/s> <http://example.org/g> "hello"@en .
<http://example.org/s> <http://example.org/g> "hello@en" .
<http://example.org/s> <http://example.org/g> "hello"@fr ."#
            .as_bytes(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-07-15T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // Retract ONLY the @en literal, named by tag.
    let out = tool_retract(
        &mut store,
        &serde_json::json!({
            "entity": "http://example.org/s",
            "predicate": "http://example.org/g",
            "value": {"value": "hello", "lang": "en"}
        }),
    )
    .unwrap();
    assert_eq!(out["retracted"], 1, "exactly the @en literal");

    let rows = tool_query(
        &store,
        &serde_json::json!({"query": "SELECT ?o WHERE { <http://example.org/s> <http://example.org/g> ?o }"}),
    )
    .unwrap();
    let left: Vec<_> = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["o"])
        .collect();
    assert_eq!(left.len(), 2, "the other two survive: {left:?}");
    // The lookalike PLAIN STRING must be untouched — it is a different term.
    assert!(
        left.iter().any(|o| *o == &serde_json::json!("hello@en")),
        "the plain string must survive being confused with the tag: {left:?}"
    );
    // …as must the same lexical form under a different tag.
    assert!(
        left.iter()
            .any(|o| *o == &serde_json::json!({"value": "hello", "lang": "fr"})),
        "@fr must survive: {left:?}"
    );
}

#[test]
fn triple_retraction_can_name_a_typed_literal() {
    let mut store = Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut store,
        r#"<http://example.org/s> <http://example.org/d> "2026-07-15"^^<http://www.w3.org/2001/XMLSchema#date> .
<http://example.org/s> <http://example.org/d> "2026-07-15" ."#
            .as_bytes(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-07-15T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // The typed literal and the plain string share a lexical form; only the
    // datatype separates them, so only the datatype can name one for removal.
    let out = tool_retract(
        &mut store,
        &serde_json::json!({
            "entity": "http://example.org/s",
            "predicate": "http://example.org/d",
            "value": {"value": "2026-07-15", "datatype": "http://www.w3.org/2001/XMLSchema#date"}
        }),
    )
    .unwrap();
    assert_eq!(out["retracted"], 1, "the xsd:date, not the plain string");

    let rows = tool_query(
        &store,
        &serde_json::json!({"query": "SELECT ?o WHERE { <http://example.org/s> <http://example.org/d> ?o }"}),
    )
    .unwrap();
    let left = rows["rows"].as_array().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(
        left[0]["o"],
        serde_json::json!("2026-07-15"),
        "plain string survives"
    );
}

#[test]
fn test_tool_export_named_graph_subset() {
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";
    let g_iri = "http://example.org/g/t1";
    let g = store.overlay_create(g_iri, 0).unwrap();
    let a = store.intern("http://example.org/a").unwrap();
    let p = store.intern("http://example.org/p").unwrap();
    let y = store.intern("http://example.org/y").unwrap();
    store
        .overlay_write(g, Op::Assert, a, p, Value::Ref(y), ts)
        .unwrap();

    let out = tool_export(
        &store,
        &serde_json::json!({ "graph": g_iri, "format": "ntriples" }),
    )
    .unwrap();
    assert_eq!(out["triples"], 1);
    assert_eq!(out["graph"], g_iri);
    assert!(
        out["rdf"]
            .as_str()
            .unwrap()
            .contains("http://example.org/a"),
        "exported RDF carries the graph's triple"
    );

    // Unknown graph -> error (not an empty success).
    assert!(
        tool_export(
            &store,
            &serde_json::json!({ "graph": "http://example.org/g/nope" })
        )
        .is_err()
    );
}
