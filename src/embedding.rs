//! Auto-embedding support for entity writes.
//!
//! Provides the [`EmbeddingProvider`] trait for pluggable embedding backends
//! (e.g. Bobbin's ONNX pipeline) and [`build_entity_text`] for constructing
//! embeddable text from an entity's current facts.
//!
//! When `auto_embed` is enabled in config and an `EmbeddingProvider` is
//! attached to the [`Store`], entities are automatically embedded after
//! each successful transaction.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::{Op, Value};

use super::store::Datum;

/// Trait for pluggable embedding backends.
///
/// Implementations must be `Send + Sync` so the provider can be shared
/// via `Arc<dyn EmbeddingProvider>` across threads and subsystems.
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text string.
    fn embed_text(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of texts. The default calls `embed_text` in a loop;
    /// backends with native batching should override for efficiency.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_text(t)).collect()
    }

    /// Return the embedding dimension (e.g. 384 for all-MiniLM-L6-v2).
    fn dimension(&self) -> usize;
}

/// Build embeddable text for an entity from its current facts.
///
/// Text is constructed in priority order:
/// 1. `rdfs:label` — the entity's display name
/// 2. `rdfs:comment` — descriptive text
/// 3. `rdf:type` — resolved type IRI(s)
/// 4. Other literal properties (strings, ints, floats, bools)
///
/// Returns an empty string if the entity has no facts (fully retracted).
pub fn build_entity_text(store: &Store, entity_id: i64) -> Result<String> {
    let facts = store.entity_facts(entity_id)?;
    if facts.is_empty() {
        return Ok(String::new());
    }

    // Track (tx, value) for the single-valued fields so the most recent
    // assertion wins. entity_facts orders only by attribute, so when a label or
    // comment has been updated by a plain assert that left the prior value
    // active, choosing among them by row order would be nondeterministic and
    // could embed stale text — the embedding would drift from current facts
    // (hq-xqc). Selecting the highest tx keeps the embedding tracking the
    // current value regardless of row order.
    let mut label: Option<(i64, String)> = None;
    let mut comment: Option<(i64, String)> = None;
    let mut types = Vec::new();
    let mut literals = Vec::new();

    let rdfs_label = store.lookup(&format!("{}label", namespace::RDFS))?;
    let rdfs_comment = store.lookup(&format!("{}comment", namespace::RDFS))?;
    let rdf_type = store.lookup(namespace::RDF_TYPE)?;

    for fact in &facts {
        if Some(fact.attribute) == rdfs_label {
            if let Value::Str(s) = &fact.value
                && label.as_ref().is_none_or(|(t, _)| fact.tx >= *t)
            {
                label = Some((fact.tx, s.clone()));
            }
        } else if Some(fact.attribute) == rdfs_comment {
            if let Value::Str(s) = &fact.value
                && comment.as_ref().is_none_or(|(t, _)| fact.tx >= *t)
            {
                comment = Some((fact.tx, s.clone()));
            }
        } else if Some(fact.attribute) == rdf_type {
            if let Value::Ref(type_id) = &fact.value
                && let Ok(iri) = store.resolve(*type_id)
            {
                // Use the local name after the last / or #
                let local = iri
                    .rsplit_once('/')
                    .or_else(|| iri.rsplit_once('#'))
                    .map_or(iri.as_str(), |(_, local)| local);
                types.push(local.to_string());
            }
        } else {
            match &fact.value {
                Value::Str(s) => literals.push(s.clone()),
                // Embed the LEXICAL form only — the tag/datatype is metadata,
                // not text a human wrote.
                Value::Lang { lexical, .. } | Value::Typed { lexical, .. } => {
                    literals.push(lexical.clone());
                }
                Value::Int(n) => literals.push(n.to_string()),
                Value::Float(f) => literals.push(f.to_string()),
                Value::Bool(b) => literals.push(b.to_string()),
                Value::Ref(_) | Value::Bytes(_) => {}
            }
        }
    }

    let mut parts = Vec::new();
    if let Some((_, l)) = label {
        parts.push(l);
    }
    if let Some((_, c)) = comment {
        parts.push(c);
    }
    if !types.is_empty() {
        parts.push(format!("type: {}", types.join(", ")));
    }
    for lit in literals {
        parts.push(lit);
    }

    Ok(parts.join(". "))
}

/// Collect unique entity IDs touched in a set of datums.
pub(crate) fn touched_entity_ids(datums: &[Datum]) -> Vec<i64> {
    let mut seen = BTreeSet::new();
    for d in datums {
        seen.insert(d.entity);
    }
    seen.into_iter().collect()
}

/// Embed work collected under the store lock, to be computed OUTSIDE it.
///
/// The ONNX embed of episode content is multi-second CPU work; running it
/// while holding the global store mutex let a sustained write flood starve
/// every reader (the mfg0 incident's write-side driver). The
/// text a deferred embed was built from travels with the work so the apply
/// step can detect staleness: if another transaction changed the entity in
/// the window between collect and apply, the stale vector is SKIPPED — the
/// later writer's own embed pass owns that entity now.
#[derive(Debug)]
pub struct DeferredEmbed {
    /// `(entity_id, text-as-of-collect)` for each entity needing a vector.
    items: Vec<(i64, String)>,
    /// The originating transaction's timestamp; vectors carry it so the
    /// deferred path stamps embeddings exactly as the inline path would.
    timestamp: String,
}

impl DeferredEmbed {
    /// The texts to embed, in item order — feed to
    /// [`EmbeddingProvider::embed_batch`] outside the lock, then hand the
    /// vectors to [`Store::apply_deferred_embed`](crate::Store::apply_deferred_embed).
    #[must_use]
    pub fn texts(&self) -> Vec<&str> {
        self.items.iter().map(|(_, t)| t.as_str()).collect()
    }

    /// True when there is nothing to embed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Fold another batch of work into this one (multiple transactions before
    /// a drain). Later items win on staleness anyway, so order is preserved
    /// but not load-bearing.
    pub(crate) fn merge(&mut self, mut other: DeferredEmbed) {
        self.items.append(&mut other.items);
        self.timestamp = other.timestamp;
    }
}

/// Phase 1 of auto-embed, run UNDER the store lock: close retired embeddings
/// (cheap `SQLite` writes) and build the texts to embed. The expensive
/// `embed_batch` is deliberately NOT here — the caller runs it lock-free and
/// then applies via [`apply_deferred_embed`].
pub(crate) fn collect_embed_work(
    store: &Store,
    entity_ids: &[i64],
    timestamp: &str,
    datums: &[Datum],
) -> Result<DeferredEmbed> {
    // Route vector writes through the configured backend (LanceDB, delegate,
    // or built-in SQLite) so auto-embed works with any vector backend.
    let vs = store.vector_store();

    // Determine which entities had retractions (need close_embedding).
    let retracted: BTreeSet<i64> = datums
        .iter()
        .filter(|d| d.op == Op::Retract)
        .map(|d| d.entity)
        .collect();

    // Close embeddings for entities that had retractions.
    for &eid in &retracted {
        vs.close_embedding(eid, timestamp)?;
    }

    // Build texts for all touched entities.
    let mut items: Vec<(i64, String)> = Vec::new();
    for &eid in entity_ids {
        let text = build_entity_text(store, eid)?;
        if !text.is_empty() {
            // For assertions on entities without prior retractions,
            // close the old embedding before creating a new one.
            if !retracted.contains(&eid) {
                vs.close_embedding(eid, timestamp)?;
            }
            items.push((eid, text));
        }
    }

    Ok(DeferredEmbed {
        items,
        timestamp: timestamp.to_string(),
    })
}

/// Phase 3 of the deferred path, run under a fresh store lock: write the
/// vectors computed outside it. STALENESS CHECK per entity: the current
/// entity text is rebuilt and compared against the text the vector was
/// computed from; on mismatch the vector is skipped — a later transaction
/// touched the entity and its own embed pass (inline or deferred) owns the
/// current state. Writing the stale vector anyway would silently regress the
/// embedding to pre-update text.
///
/// Returns the number of embeddings written.
pub(crate) fn apply_deferred_embed(
    store: &Store,
    work: &DeferredEmbed,
    embeddings: &[Vec<f32>],
) -> Result<usize> {
    let vs = store.vector_store();
    let mut written = 0;
    for ((eid, text), emb) in work.items.iter().zip(embeddings.iter()) {
        let current = build_entity_text(store, *eid)?;
        if current != *text {
            continue; // stale — a newer writer owns this entity's embedding
        }
        vs.embed_entity(*eid, text, emb, &work.timestamp)?;
        written += 1;
    }
    Ok(written)
}

/// Auto-embed entities after a transaction (the INLINE path — collect, embed
/// and write all under the caller's lock; the CLI and library default).
///
/// For each entity:
/// 1. Close any existing embedding (temporal retirement)
/// 2. Build entity text from current facts
/// 3. If text is non-empty, generate and store new embedding
///
/// Entities are processed in batches of `batch_size` for efficiency.
/// Returns the number of entities embedded.
pub(crate) fn auto_embed_entities(
    store: &Store,
    provider: &Arc<dyn EmbeddingProvider>,
    entity_ids: &[i64],
    timestamp: &str,
    batch_size: usize,
    datums: &[Datum],
) -> Result<usize> {
    let work = collect_embed_work(store, entity_ids, timestamp, datums)?;
    let vs = store.vector_store();

    let batch_sz = if batch_size == 0 { 32 } else { batch_size };
    let mut embedded = 0;

    for chunk in work.items.chunks(batch_sz) {
        let texts: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        let embeddings = provider.embed_batch(&texts)?;

        for ((eid, text), emb) in chunk.iter().zip(embeddings.iter()) {
            vs.embed_entity(*eid, text, emb, timestamp)?;
            embedded += 1;
        }
    }

    Ok(embedded)
}

#[cfg(test)]
mod tests {
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
}
