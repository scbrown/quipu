//! Tests for auto-embedding: provider wiring, entity text, deferred embed.

use super::*;
use crate::rdf::ingest_rdf;
use crate::vector::KnowledgeVectorStore;

/// Dummy embedding provider for tests — returns a deterministic
/// embedding based on text length.
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
fn build_text_with_label_and_comment() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
rdfs:label "Alice" ;
rdfs:comment "A software engineer" ;
ex:age "30" .
"#;
    let (_, _) = ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let alice_id = store.lookup("http://example.org/alice").unwrap().unwrap();
    let text = build_entity_text(&store, alice_id).unwrap();

    assert!(text.starts_with("Alice"));
    assert!(text.contains("A software engineer"));
    assert!(text.contains("type: Person"));
    assert!(text.contains("30"));
}

#[test]
fn build_text_empty_for_unknown_entity() {
    let store = Store::open_in_memory().unwrap();
    let text = build_entity_text(&store, 99999).unwrap();
    assert!(text.is_empty());
}

#[test]
fn auto_embed_on_write() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" .
ex:bob rdfs:label "Bob" .
"#;
    let (_, count) = ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    assert_eq!(count, 2);

    // Both entities should have embeddings.
    assert_eq!(store.vector_count().unwrap(), 2);
}

#[test]
fn deferred_embed_collects_under_lock_and_applies_after() {
    // The deferred-embed server flow: with deferral on, a write COLLECTS embed
    // work instead of running the (multi-second, in production) embed
    // inline; the caller embeds outside the lock and applies. End state
    // must equal the inline path's.
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;
    store.set_defer_auto_embed(true);

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" .
ex:bob rdfs:label "Bob" .
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

    // Phase 1 done: nothing embedded yet, work is pending.
    assert_eq!(
        store.vector_count().unwrap(),
        0,
        "deferral must not embed under the write"
    );
    let work = store
        .take_deferred_embed()
        .expect("a write touching embeddable entities must queue work");
    assert!(store.take_deferred_embed().is_none(), "take drains");
    assert_eq!(work.texts().len(), 2);

    // Phase 2 (lock-free in the server) + phase 3.
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(DummyProvider);
    let embeddings = provider.embed_batch(&work.texts()).unwrap();
    let written = store.apply_deferred_embed(&work, &embeddings).unwrap();
    assert_eq!(written, 2);
    assert_eq!(store.vector_count().unwrap(), 2);

    // Same end state as the inline path (DummyProvider is deterministic
    // on text, so equal text => equal vector).
    let results = store.vector_search(&[0.0f32; 8], 10, None).unwrap();
    let texts: Vec<&str> = results.iter().map(|r| r.text.as_str()).collect();
    assert!(
        texts.contains(&"Alice") && texts.contains(&"Bob"),
        "{texts:?}"
    );
}

#[test]
fn deferred_apply_skips_entities_changed_in_the_window() {
    // The unlocked window means another transaction can touch the same
    // entity between collect and apply. Applying the STALE vector anyway
    // would regress the embedding to pre-update text; it must be skipped
    // (the later writer's own work owns the entity).
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;
    store.set_defer_auto_embed(true);

    let t1 = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" .
"#;
    ingest_rdf(
        &mut store,
        t1.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    let work1 = store.take_deferred_embed().unwrap();

    // A second write updates the entity before work1 is applied.
    let t2 = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice the Great" .
"#;
    ingest_rdf(
        &mut store,
        t2.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-02-01",
        None,
        None,
    )
    .unwrap();
    let work2 = store.take_deferred_embed().unwrap();

    let provider: Arc<dyn EmbeddingProvider> = Arc::new(DummyProvider);

    // The CURRENT writer's work applies...
    let emb2 = provider.embed_batch(&work2.texts()).unwrap();
    assert_eq!(store.apply_deferred_embed(&work2, &emb2).unwrap(), 1);

    // ...and the stale one is skipped, leaving the current embedding.
    let emb1 = provider.embed_batch(&work1.texts()).unwrap();
    assert_eq!(
        store.apply_deferred_embed(&work1, &emb1).unwrap(),
        0,
        "stale work must not overwrite a newer embedding"
    );
    let results = store.vector_search(&[0.0f32; 8], 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "Alice the Great");
}

#[test]
fn auto_embed_disabled_by_default() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    // auto_embed defaults to false — no embeddings generated.

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" .
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

    assert_eq!(store.vector_count().unwrap(), 0);
}

#[test]
fn retract_and_reassert_updates_embedding() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    // Assert initial fact.
    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" .
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

    let alice_id = store.lookup("http://example.org/alice").unwrap().unwrap();

    // Verify initial embedding exists.
    let results = store.vector_search(&[0.0f32; 8], 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "Alice");

    // Retract alice.
    store
        .retract_entity(alice_id, None, "2026-02-01", None)
        .unwrap();

    // Old embedding should be closed, no current embeddings.
    assert_eq!(store.vector_count().unwrap(), 0);

    // Reassert with new label.
    let turtle2 = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice the Great" .
"#;
    ingest_rdf(
        &mut store,
        turtle2.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-03-01",
        None,
        None,
    )
    .unwrap();

    // New embedding should exist with updated text.
    let results = store.vector_search(&[0.0f32; 8], 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "Alice the Great");

    // Time-travel: old embedding should be visible at Jan.
    let results = store
        .vector_search(&[0.0f32; 8], 10, Some("2026-01-15"))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "Alice");
}

#[test]
fn episode_batch_embeds() {
    use crate::episode::{Edge, Episode, Node, ingest_episode};

    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let ep = Episode {
        name: "test-ep".into(),
        episode_body: Some("A test episode".into()),
        source: Some("test".into()),
        group_id: None,
        replace_snapshot: false,
        nodes: vec![
            Node {
                name: "Foo".into(),
                node_type: Some("Service".into()),
                description: Some("The foo service".into()),
                properties: None,
            },
            Node {
                name: "Bar".into(),
                node_type: Some("Service".into()),
                description: Some("The bar service".into()),
                properties: None,
            },
        ],
        edges: vec![Edge {
            source: "Foo".into(),
            target: "Bar".into(),
            relation: "dependsOn".into(),
            confidence: None,
        }],
        graph: None,
        shapes: None,
    };

    let (_, count) =
        ingest_episode(&mut store, &ep, "2026-01-01", namespace::DEFAULT_BASE_NS).unwrap();
    assert!(count > 0);

    // All nodes + episode entity should have embeddings.
    let vec_count = store.vector_count().unwrap();
    assert!(
        vec_count >= 2,
        "expected at least 2 embeddings, got {vec_count}"
    );
}

#[test]
fn auto_embed_skipped_with_delegate() {
    use crate::vector::VectorMatch;
    use crate::vector_delegate::VectorSearchDelegate;

    struct EmptyDelegate;
    impl VectorSearchDelegate for EmptyDelegate {
        fn vector_search(
            &self,
            _q: &[f32],
            _l: usize,
            _v: Option<&str>,
        ) -> crate::error::Result<Vec<VectorMatch>> {
            Ok(vec![])
        }
        fn vector_count(&self) -> crate::error::Result<usize> {
            Ok(0)
        }
    }

    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;
    store.set_vector_search_delegate(Arc::new(EmptyDelegate));

    let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice rdfs:label "Alice" .
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

    // Auto-embed should be skipped — local vectors table stays empty.
    let local_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |row| row.get(0))
        .unwrap();
    assert_eq!(local_count, 0, "auto-embed should be skipped with delegate");
}

#[test]
fn touched_entity_ids_deduplicates() {
    let datums = vec![
        Datum {
            entity: 1,
            attribute: 10,
            value: Value::Str("a".into()),
            valid_from: "t".into(),
            valid_to: None,
            op: Op::Assert,
        },
        Datum {
            entity: 2,
            attribute: 10,
            value: Value::Str("b".into()),
            valid_from: "t".into(),
            valid_to: None,
            op: Op::Assert,
        },
        Datum {
            entity: 1,
            attribute: 11,
            value: Value::Str("c".into()),
            valid_from: "t".into(),
            valid_to: None,
            op: Op::Assert,
        },
    ];
    let ids = touched_entity_ids(&datums);
    assert_eq!(ids, vec![1, 2]);
}

/// hq-xqc: updating an entity's label via a plain assert (no explicit
/// retract) must re-embed it with the new text, and exactly one current
/// embedding should remain.
#[test]
fn plain_label_update_reembeds() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let assert_label = |store: &mut Store, label: &str, ts: &str| {
        let ttl = format!(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             @prefix ex: <http://example.org/> .\n\
             ex:alice rdfs:label \"{label}\" .\n"
        );
        ingest_rdf(
            store,
            ttl.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            ts,
            None,
            None,
        )
        .unwrap();
    };

    assert_label(&mut store, "Alice", "2026-01-01");
    // Plain assert of a new label value — the old value is not retracted.
    assert_label(&mut store, "Alicia", "2026-02-01");

    let results = store.vector_search(&[0.0f32; 8], 10, None).unwrap();
    assert_eq!(results.len(), 1, "exactly one current embedding");
    assert_eq!(results[0].text, "Alicia", "embedding tracks the new label");
}

/// hq-xqc: the same for rdfs:comment, and verified at the text-builder level
/// so it does not depend on row order among multiple active values.
#[test]
fn plain_comment_update_reembeds() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(DummyProvider));
    store.embedding_config_mut().auto_embed = true;

    let assert_comment = |store: &mut Store, comment: &str, ts: &str| {
        let ttl = format!(
            "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
             @prefix ex: <http://example.org/> .\n\
             ex:bob rdfs:label \"Bob\" ;\n    rdfs:comment \"{comment}\" .\n"
        );
        ingest_rdf(
            store,
            ttl.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            ts,
            None,
            None,
        )
        .unwrap();
    };

    assert_comment(&mut store, "first note", "2026-01-01");
    assert_comment(&mut store, "second note", "2026-02-01");

    let bob = store.lookup("http://example.org/bob").unwrap().unwrap();
    let text = build_entity_text(&store, bob).unwrap();
    assert!(
        text.contains("second note"),
        "embedding text should include the newest comment, got: {text}"
    );
    assert!(
        !text.contains("first note"),
        "stale comment must not appear, got: {text}"
    );

    let results = store.vector_search(&[0.0f32; 8], 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, text);
}
