//! Private acceptance for the opt-in static HTTP matching profile.
#![cfg(feature = "shacl")]

use quipu::{store::Store, tool_ask, tool_knot, tool_queries, tool_query};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SHAPES: &str = include_str!("../profiles/static-http/shapes.ttl");
const QUERY: &str = include_str!("../profiles/static-http/matches.json");
const GRAPH: &str = "urn:example:static-http:derived";
const NS: &str = "http://aegis.gastown.local/ontology/";

fn candidate_iri(id: &str, role: &str, method: &str, path: &str) -> String {
    // id selects a distinct mount byte range in this synthetic fixture.
    let mount = 200 + id.bytes().map(u64::from).sum::<u64>();
    let repo = if role == "consumer" {
        "client"
    } else {
        "server"
    };
    let tuple = json!([
        "yupana-http-candidates/v1",
        "python-fastapi-requests/v1",
        repo,
        "a".repeat(40),
        "src/api.rs",
        "b".repeat(64),
        "fixture-v1",
        role,
        [100, 140],
        [[mount, mount + 10]],
        method,
        path,
        null
    ]);
    format!(
        "{NS}http-candidate/{:x}",
        Sha256::digest(tuple.to_string().as_bytes())
    )
}

fn candidate(id: &str, role: &str, method: &str, path: &str) -> String {
    let repo = if role == "consumer" {
        "client"
    } else {
        "server"
    };
    let iri = candidate_iri(id, role, method, path);
    let mount = 200 + id.bytes().map(u64::from).sum::<u64>();
    let body = json!({"contract":"yupana-http-candidates/v1",
        "profile":"python-fastapi-requests/v1", "repository":repo,
        "observed_at":"2026-01-01T00:00:00Z",
        "revision":"a".repeat(40), "extractor":{"name":"yupana","version":"fixture-v1","profile_version":1},
        "files":[{"module":"src/api.rs","source_sha256":"b".repeat(64),
        "language":"python", "diagnostics":[],
        "snapshot_state":"verified_commit", "candidates":[{"id":iri.rsplit('/').next().unwrap(),
        "role":role,"tier":"treesitter","confidence":"candidate","method":method,
        "declared_path":path,"path":path,"authority":null,"query_present":false,
        "occurrence_span":{"start_byte":100,"end_byte":140,"start_line":10,"end_line":10,"start_column_byte":0,"end_column_byte":40},
        "prefix_spans":[{"start_byte":mount,"end_byte":mount+10,"start_line":1,"end_line":1,"start_column_byte":0,"end_column_byte":10}],"caller":null,"handler":null,"resolution":"literal","unresolved_reasons":[]}]}]}).to_string();
    let hash = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
    // JSON string escaping is also valid Turtle string escaping for this fixture.
    let literal = serde_json::to_string(&body).unwrap();
    format!(
        r#"@prefix c: <{NS}> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        <{iri}> a c:HttpContractCandidate ; rdfs:label "{id}" ;
          c:sourceManifest <urn:example:manifest-{id}> ;
          c:candidateTier "treesitter" ; c:candidateConfidence "candidate" ;
          c:candidateRole "{role}" ; c:service <urn:example:service> ;
          c:repo "{repo}" ; c:revision "{}" ;
          c:sourceHash "sha256:{}" ; c:extractorVersion "fixture-v1" ;
          c:filePath "src/api.rs" ; c:occurrenceSpan "100:140" ;
          c:httpMethod "{method}" ; c:httpPath "{path}" ;
          c:resolution "exact" ; c:generatedClient false .
        <urn:example:manifest-{id}> a c:Document ; rdfs:label "DTO manifest {id}" ;
          c:filePath "fixture.json" ; c:httpCandidateContract "yupana-http-candidates/v1" ;
          c:httpCandidateProfile "python-fastapi-requests/v1" ;
          c:manifestBody {literal} ; c:manifestHash "{hash}" .
        "#,
        "a".repeat(40),
        "b".repeat(64)
    )
}

fn store(data: &str) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store
        .load_shapes("static-http", SHAPES, "2026-01-01T00:00:00Z")
        .unwrap();
    store
        .load_shapes(
            "code-entities",
            include_str!("../shapes/code-entities.ttl"),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
    store.graph_create(GRAPH).unwrap();
    tool_knot(&mut store, &json!({"turtle": data, "graph": GRAPH})).unwrap();
    let mut query: Value = serde_json::from_str(QUERY).unwrap();
    query["action"] = json!("load");
    tool_queries(&store, &query).unwrap();
    store
}

fn matches(store: &Store) -> Value {
    tool_ask(
        store,
        &json!({"name": "static_http_matches", "params": {
            "input_graph": GRAPH, "service": "urn:example:service",
            "consumer_repository": "client", "producer_repository": "server",
            "consumer_revision": "a".repeat(40), "producer_revision": "a".repeat(40),
        "consumer_source_hash": format!("sha256:{}", "b".repeat(64)),
        "producer_source_hash": format!("sha256:{}", "b".repeat(64))
        }}),
    )
    .unwrap()
}

fn fixture(producer: &str) -> String {
    candidate("consumer", "consumer", "GET", "/api/items") + producer
}

fn assert_unresolved(producer: &str) {
    let store = store(&fixture(producer));
    let result = matches(&store);
    assert_eq!(result["count"], 1, "consumer must remain visible: {result}");
    assert!(
        result["rows"][0].get("producer").is_none_or(Value::is_null),
        "{result}"
    );
}

#[test]
fn unique_exact_match_preserves_both_provenances_and_ignores_root() {
    let data = fixture(&candidate("producer", "producer", "GET", "/api/items"));
    let mut before = Store::open_in_memory().unwrap();
    before
        .load_shapes(
            "code-entities",
            include_str!("../shapes/code-entities.ttl"),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
    before.graph_create(GRAPH).unwrap();
    let refused = tool_knot(&mut before, &json!({"turtle": data, "graph": GRAPH})).unwrap_err();
    assert!(
        refused.to_string().contains("unknown rdf:type"),
        "{refused}"
    );
    println!("BEFORE (base code-entities only): candidate input refused as unknown rdf:type");
    let mut store = store(&fixture(&candidate(
        "producer",
        "producer",
        "GET",
        "/api/items",
    )));
    // A ROOT collision must not influence this explicitly scoped derived read.
    tool_knot(
        &mut store,
        &json!({"turtle": candidate("root", "producer", "GET", "/api/items")}),
    )
    .unwrap();
    let result = matches(&store);
    assert_eq!(result["count"], 1, "{result}");
    println!("AFTER (opt-in profile loaded): {}", result["rows"][0]);
    let row = &result["rows"][0];
    assert_eq!(
        row["producer"],
        candidate_iri("producer", "producer", "GET", "/api/items")
    );
    assert_eq!(row["consumerManifest"], "urn:example:manifest-consumer");
    assert!(
        row["producerManifestHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(row["consumerRevision"], "a".repeat(40));
    assert_eq!(row["producerHash"], format!("sha256:{}", "b".repeat(64)));
    assert_eq!(row["consumerExtractor"], "fixture-v1");
    assert_eq!(row["producerExtractor"], "fixture-v1");
    let root = tool_query(
        &store,
        &json!({"query": format!("SELECT ?s WHERE {{ ?s a <{NS}HttpContractCandidate> }}")}),
    )
    .unwrap();
    assert_eq!(
        root["count"], 1,
        "only the deliberately seeded ROOT control"
    );
}

#[test]
fn collisions_wrong_method_and_wrong_prefix_do_not_resolve() {
    let producer = candidate("producer", "producer", "GET", "/api/items");
    assert_unresolved(&(producer + &candidate("collision", "producer", "GET", "/api/items")));
    assert_unresolved(&candidate("producer", "producer", "POST", "/api/items"));
    assert_unresolved(&candidate("producer", "producer", "GET", "/items"));
    assert_unresolved("");
}

#[test]
fn stale_generated_unknown_and_wrong_service_do_not_resolve() {
    let producer = candidate("producer", "producer", "GET", "/api/items");
    assert_unresolved(&producer.replace(&"a".repeat(40), &"c".repeat(40)));
    assert_unresolved(&producer.replace(&"b".repeat(64), &"d".repeat(64)));
    assert_unresolved(&producer.replace("generatedClient false", "generatedClient true"));
    assert_unresolved(&producer.replace("resolution \"exact\"", "resolution \"unresolved\""));
    assert_unresolved(&producer.replace("urn:example:service", "urn:example:other-service"));
    for path in ["/api/*", "/api/{id}"] {
        let data = candidate("consumer", "consumer", "GET", path)
            + &candidate("producer", "producer", "GET", path);
        let result = matches(&store(&data));
        assert!(
            result["rows"][0].get("producer").is_none_or(Value::is_null),
            "{result}"
        );
    }
    let data = candidate("consumer", "consumer", "UNKNOWN", "/api/items")
        + &candidate("producer", "producer", "UNKNOWN", "/api/items");
    let result = matches(&store(&data));
    assert!(
        result["rows"][0].get("producer").is_none_or(Value::is_null),
        "{result}"
    );
}

#[test]
fn shape_requires_single_service_and_complete_committed_source_evidence() {
    let good = candidate("producer", "producer", "GET", "/api/items");
    assert!(quipu::validate_shapes(SHAPES, &good).unwrap().conforms);
    for bad in [
        good.replace(&format!("c:sourceHash \"sha256:{}\" ;", "b".repeat(64)), ""),
        good.replace(
            "c:service <urn:example:service>",
            "c:service <urn:example:service>, <urn:example:other>",
        ),
        good.replace(&"a".repeat(40), "working-tree"),
        good.replace("fixture-v1", ""),
    ] {
        assert!(
            !quipu::validate_shapes(SHAPES, &bad).unwrap().conforms,
            "{bad}"
        );
    }
}

#[test]
fn uncertain_consumer_evidence_remains_visible_and_unresolved() {
    let producer = candidate("producer", "producer", "GET", "/api/items");
    let consumer = candidate("consumer", "consumer", "GET", "/api/items");
    for changed in [
        consumer.replace("c:service <urn:example:service> ;", ""),
        consumer.replace(&"a".repeat(40), &"c".repeat(40)),
        consumer.replace(&"b".repeat(64), &"d".repeat(64)),
        consumer.replace("generatedClient false", "generatedClient true"),
        consumer.replace("c:generatedClient false", ""),
        consumer.replace("resolution \"exact\"", "resolution \"unresolved\""),
    ] {
        let result = matches(&store(&(changed + &producer)));
        assert_eq!(result["count"], 1, "{result}");
        assert!(
            result["rows"][0].get("producer").is_none_or(Value::is_null),
            "{result}"
        );
    }
}

#[test]
fn occurrence_identity_and_manifest_are_mandatory() {
    let id = candidate_iri("mount-a", "producer", "GET", "/api/items");
    assert_ne!(
        id,
        candidate_iri("mount-b", "producer", "GET", "/api/items")
    );
    assert_ne!(
        id,
        candidate_iri("mount-a", "consumer", "GET", "/api/items")
    );
    assert_ne!(
        id,
        candidate_iri("mount-a", "producer", "POST", "/api/items")
    );
    let good = candidate("producer", "producer", "GET", "/api/items");
    for bad in [
        good.replace("c:sourceManifest <urn:example:manifest-producer> ;", ""),
        good.replace(
            "c:candidateConfidence \"candidate\"",
            "c:candidateConfidence \"authoritative\"",
        ),
        good.replace("c:manifestBody", "c:omittedBody"),
        good.replace("c:httpCandidateContract", "c:omittedContract"),
    ] {
        assert!(
            !quipu::validate_shapes(SHAPES, &bad).unwrap().conforms,
            "{bad}"
        );
    }
}
