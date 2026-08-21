//! Tests for entity resolution.
//!
//! Split out of `resolution.rs` when the module became a directory: the file
//! was 477 lines against a 500-line ceiling, and everything added here since
//! (the union-scoped label read, `quipu:distinctFrom`, the episode-level
//! contention pass) needed room the single file did not have.

use std::sync::Arc;

use super::*;
use crate::embedding::EmbeddingProvider;
use crate::rdf::ingest_rdf;

/// Deterministic test embedding provider — produces embeddings based
/// on text length so that similar-length texts have similar vectors.
struct DummyProvider;

impl EmbeddingProvider for DummyProvider {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let seed = text.len() as f32;
        Ok((0..8).map(|i| (seed + i as f32 * 0.1).sin()).collect())
    }

    fn dimension(&self) -> usize {
        8
    }
}

#[test]
fn resolve_finds_exact_name_match() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:Alice a ex:Person ;
rdfs:label "Alice" ;
rdfs:comment "A software engineer" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Exact name match.
    let result = resolve_entity(&store, "Alice", &[], 0.85, 3).unwrap();
    assert!(result.has_matches);
    assert!(!result.candidates.is_empty());
    assert!(result.candidates[0].iri.contains("Alice"));
    assert_eq!(result.candidates[0].score, 1.0);
    assert!(
        result.candidates[0]
            .matched_on
            .starts_with("canonical_name:exact")
    );
}

#[test]
fn tool_resolve_entity_is_a_genuine_read_commits_nothing() {
    use crate::vector::KnowledgeVectorStore;
    // The /resolve route's whole reason to exist is asking
    // "what would resolution say?" WITHOUT writing — before it, consumers
    // had to POST an episode and retract to see embedding matches. And
    // ro_handler! is a naming convention, not a type guarantee
    //: a `&Store` can commit through interior mutability, so
    // "read-only" must be asserted, not assumed.
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:Alice a ex:Person ;
rdfs:label "Alice" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let tx_before = store.latest_tx_id().unwrap();
    let vecs_before = store.vector_count().unwrap();

    // The exact payload shape the HTTP route serves.
    let out = crate::tool_resolve_entity(
        &store,
        &serde_json::json!({"name": "Alice", "threshold": 0.85, "top_k": 3}),
    )
    .unwrap();
    assert_eq!(out["has_matches"], true);
    assert!(out["count"].as_u64().unwrap() >= 1);
    assert!(
        out["candidates"][0]["iri"]
            .as_str()
            .unwrap()
            .contains("Alice")
    );

    assert_eq!(
        store.latest_tx_id().unwrap(),
        tx_before,
        "a resolution dry-run must not commit a transaction"
    );
    assert_eq!(
        store.vector_count().unwrap(),
        vecs_before,
        "a resolution dry-run must not write vectors"
    );
}

#[test]
fn resolve_finds_similar_name_jaro_winkler() {
    let mut store = Store::open_in_memory().unwrap();

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice_smith a ex:Person ;
rdfs:label "Alice Smith" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Similar name should match via Jaro-Winkler.
    let result = resolve_entity(&store, "Alice Smth", &[], 0.85, 3).unwrap();
    assert!(
        result.has_matches,
        "expected 'Alice Smth' to match 'Alice Smith' via Jaro-Winkler"
    );
    assert!(result.candidates[0].score > 0.85);
    assert!(
        result.candidates[0].matched_on.contains("jaro_winkler"),
        "expected Jaro-Winkler match, got: {}",
        result.candidates[0].matched_on
    );
}

#[test]
fn resolve_no_match_for_dissimilar_name() {
    let mut store = Store::open_in_memory().unwrap();

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
rdfs:label "Alice" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Very different name should not match.
    let result = resolve_entity(&store, "Zebra Corporation", &[], 0.85, 3).unwrap();
    assert!(
        !result.has_matches,
        "expected no match for 'Zebra Corporation' vs 'Alice'"
    );
}

#[test]
fn resolve_with_embedding_similarity() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
rdfs:label "Alice" ;
rdfs:comment "A software engineer" .

ex:bob a ex:Person ;
rdfs:label "Bob" ;
rdfs:comment "A data scientist" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Resolve with a name that has similar embedding.
    let result = resolve_entity(&store, "Alic", &[], 0.50, 5).unwrap();
    // With DummyProvider, similar-length texts → similar embeddings.
    // At least the name-based match should fire.
    assert!(
        result.has_matches,
        "expected at least a name-based match for 'Alic' vs 'Alice'"
    );
}

#[test]
fn resolve_disabled_returns_empty() {
    let store = Store::open_in_memory().unwrap();

    let result = resolve_entity(&store, "Alice", &[], 0.85, 3).unwrap();
    // No entities in store → no matches.
    assert!(!result.has_matches);
    assert!(result.candidates.is_empty());
}

#[test]
fn resolve_threshold_099_effectively_off() {
    let mut store = Store::open_in_memory().unwrap();

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
rdfs:label "Alice" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // Very high threshold should only match exact names.
    let result = resolve_entity(&store, "Alic", &[], 0.99, 3).unwrap();
    assert!(
        !result.has_matches,
        "expected no match at threshold 0.99 for 'Alic' vs 'Alice'"
    );
}

#[test]
fn resolve_deduplicates_by_iri() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
rdfs:label "Alice" ;
rdfs:comment "A software engineer" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    // "Alice" should be found by both embedding and name matching
    // but should appear only once in results.
    let result = resolve_entity(&store, "Alice", &[], 0.50, 10).unwrap();
    let alice_count = result
        .candidates
        .iter()
        .filter(|c| c.iri.contains("alice"))
        .count();
    assert_eq!(
        alice_count, 1,
        "expected exactly one Alice candidate after dedup, got {alice_count}"
    );
}

// ── Composed stores: what each half of resolution can see ──────────────

const T0: &str = "2026-01-01T00:00:00Z";

/// A temp directory that cleans itself up, for the multi-file compositions.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "quipu-resolution-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, n: &str) -> std::path::PathBuf {
        self.0.join(n)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A store with one attached layer that labels `urn:layer:postgres` "Postgres".
///
/// The label goes into a NAMED graph, not the layer's ROOT: an attachment
/// contributes only its named graphs (`contributed_graphs_sql`), so a layer
/// ROOT fact would be invisible to the composition no matter what resolution
/// reads. That is the shape a knowledge pack actually has.
fn store_with_labelled_layer(scratch: &Scratch) -> Store {
    use crate::store::attach::Attachment;
    use crate::types::Value;
    use crate::{Datum, Op};

    let main = scratch.path("main.db");
    {
        Store::open(&main.to_string_lossy()).unwrap();
    }

    let src = scratch.path("layer-src.db");
    {
        let mut s = Store::open(&src.to_string_lossy()).unwrap();
        let subject = s.intern("urn:layer:postgres").unwrap();
        let label = s.intern(&format!("{}label", namespace::RDFS)).unwrap();
        let g = s.intern("urn:layer:graph").unwrap();
        s.conn
            .execute(
                "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
                 VALUES (?1, 'committed', NULL, ?2)",
                rusqlite::params![g, T0],
            )
            .unwrap();
        s.transact_to_graph(
            &[Datum {
                entity: subject,
                attribute: label,
                value: Value::Str("Postgres".into()),
                valid_from: T0.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            T0,
            None,
            None,
            g,
        )
        .unwrap();
    }
    let layer = scratch.path("layer.db");
    crate::store::respace::respace_file(&src, &layer, 31).unwrap();

    Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("layer", &layer.to_string_lossy())],
    )
    .unwrap()
}

#[test]
fn name_resolution_sees_entities_in_attached_layers() {
    // The regression this exists for: `resolve_by_name` read the bare `facts`
    // table, which is `main.facts`. On the composition the whole multi-db design
    // targets — a shared reference layer beside a per-tenant store — resolving a
    // name the LAYER already defines returned nothing, and the tenant minted a
    // duplicate of an entity it was attached to in order to share.
    let scratch = Scratch::new("layer-visible");
    let store = store_with_labelled_layer(&scratch);

    let result = resolve_entity(&store, "Postgres", &[], 0.85, 3).unwrap();
    assert!(
        result.has_matches,
        "the layer's entity must be a resolution candidate"
    );
    assert_eq!(result.candidates[0].iri, "urn:layer:postgres");
    assert_eq!(result.candidates[0].matched_on, "canonical_name:exact");
}

#[test]
fn near_miss_in_a_layer_is_caught_too() {
    // Not just the exact-match arm: the Jaro-Winkler arm reads the same index,
    // so a typo against a layer entity resolves as well.
    let scratch = Scratch::new("layer-typo");
    let store = store_with_labelled_layer(&scratch);

    let result = resolve_entity(&store, "Postgres ", &[], 0.85, 3).unwrap();
    assert!(result.has_matches);
    assert_eq!(result.candidates[0].iri, "urn:layer:postgres");
}

#[test]
fn vector_scope_distinguishes_no_duplicates_from_never_looked() {
    // An empty candidate list from a composed store is ambiguous, and the
    // ambiguity is the bug: the embedding half cannot search an attached layer's
    // vectors, so "found nothing" and "never searched the layer" produced the
    // same answer. The scope is what separates them.
    let scratch = Scratch::new("scope");
    let composed = store_with_labelled_layer(&scratch);
    let scope = VectorScope::of(&composed);
    assert_eq!(scope, VectorScope::LocalOnly { attached_layers: 1 });
    assert!(scope.is_partial());

    // The name half DID cover the layer, so the report is specific about which
    // half fell short rather than disclaiming the whole result.
    let result = resolve_entity(&composed, "Postgres", &[], 0.85, 3).unwrap();
    assert!(result.has_matches);
    assert_eq!(result.vector_scope, scope);

    let plain = Store::open_in_memory().unwrap();
    assert_eq!(VectorScope::of(&plain), VectorScope::WholeStore);
    assert!(!VectorScope::of(&plain).is_partial());
}

// ── quipu:distinctFrom ─────────────────────────────────────────────────

/// A store labelling `ex:Alice` "Alice", for the override tests.
fn store_with_alice() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:Alice a ex:Person ; rdfs:label "Alice" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    store
}

#[test]
fn declared_distinct_from_excuses_exactly_that_pairing() {
    // Before this, strict mode refused the write and told the caller to "assert
    // quipu:distinctFrom to override" — a term that existed nowhere but in that
    // sentence. The only real escape was turning strict_mode off for the whole
    // store, which drops the check for every entity rather than the one meant.
    let store = store_with_alice();
    let alice = "http://example.org/Alice".to_string();

    let contested = NodeQuery {
        name: "Alice",
        iri: "http://example.org/Alice2".to_string(),
        properties: vec![],
        declared_distinct: &[],
    };
    let refused = resolve_nodes(&store, std::slice::from_ref(&contested), 0.85, 3, true).unwrap();
    assert!(refused.refusal.is_some(), "strict mode must refuse");

    let excused = NodeQuery {
        name: "Alice",
        iri: "http://example.org/Alice2".to_string(),
        properties: vec![],
        declared_distinct: std::slice::from_ref(&alice),
    };
    let ok = resolve_nodes(&store, &[excused], 0.85, 3, true).unwrap();
    assert!(ok.refusal.is_none(), "the declared pairing is excused");
    assert!(ok.hints.is_empty(), "and is not reported as a hint either");
}

#[test]
fn distinct_from_excuses_only_the_named_pairing() {
    // An override that excused everything would be `strict_mode = false` wearing
    // a costume. Declaring distinctness from an UNRELATED entity must leave the
    // real refusal standing.
    let store = store_with_alice();
    let unrelated = ["http://example.org/Somebody".to_string()];

    let node = NodeQuery {
        name: "Alice",
        iri: "http://example.org/Alice2".to_string(),
        properties: vec![],
        declared_distinct: &unrelated,
    };
    let result = resolve_nodes(&store, &[node], 0.85, 3, true).unwrap();
    assert!(
        result.refusal.is_some(),
        "excusing an unrelated IRI must not excuse Alice"
    );
}

#[test]
fn recorded_distinct_from_outlives_the_write_that_declared_it() {
    // The durability half. A writer declares the pairing once; the fact lands in
    // the graph, and a LATER write of the same entity that declares nothing is
    // still excused. Without this the override would have to be re-declared on
    // every re-ingest, and any producer that forgot would be refused again.
    use crate::episode::{Episode, IngestResolutionOpts, Node, ingest_episode_with_resolution};

    let mut store = store_with_alice();
    let base = "http://example.org/";
    let opts = IngestResolutionOpts {
        enabled: true,
        threshold: 0.85,
        top_k: 3,
        strict_mode: true,
    };

    let declaring = Episode {
        name: "declare".into(),
        episode_body: None,
        source: None,
        group_id: None,
        nodes: vec![Node {
            name: "Alice".into(),
            node_type: Some("Person".into()),
            description: None,
            properties: None,
            distinct_from: vec!["http://example.org/Alice".into()],
        }],
        edges: vec![],
        graph: None,
        shapes: None,
        replace_snapshot: false,
    };
    ingest_episode_with_resolution(&mut store, &declaring, T0, base, Some(&opts))
        .expect("the declared pairing is excused");

    // The assertion is a fact now, readable under the IRI the node was written as.
    let iri = crate::episode::node_iri("Alice", base);
    let recorded = recorded_distinct_from(&store, &iri).unwrap();
    assert!(
        recorded.contains(&"http://example.org/Alice".to_string()),
        "expected a durable quipu:distinctFrom, got {recorded:?}"
    );

    // A second write declaring NOTHING is excused by what the graph remembers.
    let silent = Episode {
        name: "silent".into(),
        episode_body: None,
        source: None,
        group_id: None,
        nodes: vec![Node {
            name: "Alice".into(),
            node_type: Some("Person".into()),
            description: None,
            properties: None,
            distinct_from: vec![],
        }],
        edges: vec![],
        graph: None,
        shapes: None,
        replace_snapshot: false,
    };
    ingest_episode_with_resolution(&mut store, &silent, T0, base, Some(&opts))
        .expect("the recorded pairing still excuses the write");
}

// ── Episode-level contention ───────────────────────────────────────────

#[test]
fn two_nodes_claiming_one_entity_are_reported_as_contention() {
    // The failure a per-node loop cannot see. Both nodes resolve to Alice, so
    // the write is about to fragment one entity into two — but each node in
    // isolation looks like an ordinary near-miss, and that is all the old loop
    // could report.
    let store = store_with_alice();
    let nodes = [
        NodeQuery {
            name: "Alice",
            iri: "http://example.org/n1".to_string(),
            properties: vec![],
            declared_distinct: &[],
        },
        NodeQuery {
            name: "Alicee",
            iri: "http://example.org/n2".to_string(),
            properties: vec![],
            declared_distinct: &[],
        },
    ];
    let result = resolve_nodes(&store, &nodes, 0.85, 3, false).unwrap();

    assert_eq!(result.contentions.len(), 1, "one contested entity");
    let c = &result.contentions[0];
    assert_eq!(c.iri, "http://example.org/Alice");
    let names: Vec<&str> = c.claimants.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"Alice") && names.contains(&"Alicee"));
    // Claimants come back strongest-first, so a caller reading only the head
    // sees the node with the best claim rather than whichever was listed first.
    assert!(c.claimants[0].1 >= c.claimants[1].1);
}

#[test]
fn distinct_nodes_do_not_contend() {
    // The other direction: two nodes matching two DIFFERENT entities are not a
    // conflict, and reporting them as one would train callers to ignore the field.
    let mut store = store_with_alice();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:Bob a ex:Person ; rdfs:label "Bob" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-02",
        None,
        None,
    )
    .unwrap();

    let nodes = [
        NodeQuery {
            name: "Alice",
            iri: "http://example.org/n1".to_string(),
            properties: vec![],
            declared_distinct: &[],
        },
        NodeQuery {
            name: "Bob",
            iri: "http://example.org/n2".to_string(),
            properties: vec![],
            declared_distinct: &[],
        },
    ];
    let result = resolve_nodes(&store, &nodes, 0.85, 3, false).unwrap();
    assert_eq!(result.hints.len(), 2, "both nodes matched something");
    assert!(
        result.contentions.is_empty(),
        "different entities are not contention: {:?}",
        result.contentions
    );
}

#[test]
fn batch_resolution_agrees_with_the_single_entity_path() {
    // The batch pass exists to scan the label set ONCE for N nodes instead of N
    // times. That is a performance change, so its correctness claim is that it
    // changes nothing else: the same candidates, in the same order.
    let store = store_with_alice();
    let names = ["Alice", "Alicia", "Zebra"];

    let batch = resolve_nodes(
        &store,
        &names
            .iter()
            .map(|n| NodeQuery {
                name: n,
                iri: format!("http://example.org/{n}"),
                properties: vec![],
                declared_distinct: &[],
            })
            .collect::<Vec<_>>(),
        0.85,
        3,
        false,
    )
    .unwrap();

    for name in names {
        let single = resolve_entity(&store, name, &[], 0.85, 3).unwrap();
        let from_batch = batch.hints.iter().find(|(n, _)| n == name);
        match (single.has_matches, from_batch) {
            (true, Some((_, candidates))) => {
                let a: Vec<_> = candidates.iter().map(|c| (&c.iri, &c.matched_on)).collect();
                let b: Vec<_> = single
                    .candidates
                    .iter()
                    .map(|c| (&c.iri, &c.matched_on))
                    .collect();
                assert_eq!(a, b, "batch and single disagree on {name}");
            }
            (false, None) => {}
            other => panic!("batch and single disagree on whether {name} matched: {other:?}"),
        }
    }
}
