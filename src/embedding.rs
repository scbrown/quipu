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
use crate::vector::KnowledgeVectorStore;

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

    let mut label = None;
    let mut comment = None;
    let mut types = Vec::new();
    let mut literals = Vec::new();

    let rdfs_label = store.lookup(&format!("{}label", namespace::RDFS))?;
    let rdfs_comment = store.lookup(&format!("{}comment", namespace::RDFS))?;
    let rdf_type = store.lookup(namespace::RDF_TYPE)?;

    for fact in &facts {
        if Some(fact.attribute) == rdfs_label {
            if let Value::Str(s) = &fact.value {
                label = Some(s.clone());
            }
        } else if Some(fact.attribute) == rdfs_comment {
            if let Value::Str(s) = &fact.value {
                comment = Some(s.clone());
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
                Value::Int(n) => literals.push(n.to_string()),
                Value::Float(f) => literals.push(f.to_string()),
                Value::Bool(b) => literals.push(b.to_string()),
                Value::Ref(_) | Value::Bytes(_) => {}
            }
        }
    }

    let mut parts = Vec::new();
    if let Some(l) = label {
        parts.push(l);
    }
    if let Some(c) = comment {
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

/// Auto-embed entities after a transaction.
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
    // Determine which entities had retractions (need close_embedding).
    let retracted: BTreeSet<i64> = datums
        .iter()
        .filter(|d| d.op == Op::Retract)
        .map(|d| d.entity)
        .collect();

    // Close embeddings for entities that had retractions.
    for &eid in &retracted {
        store.close_embedding(eid, timestamp)?;
    }

    // Build texts for all touched entities.
    let mut to_embed: Vec<(i64, String)> = Vec::new();
    for &eid in entity_ids {
        let text = build_entity_text(store, eid)?;
        if !text.is_empty() {
            // For assertions on entities without prior retractions,
            // close the old embedding before creating a new one.
            if !retracted.contains(&eid) {
                store.close_embedding(eid, timestamp)?;
            }
            to_embed.push((eid, text));
        }
    }

    let batch_sz = if batch_size == 0 { 32 } else { batch_size };
    let mut embedded = 0;

    for chunk in to_embed.chunks(batch_sz) {
        let texts: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        let embeddings = provider.embed_batch(&texts)?;

        for ((eid, text), emb) in chunk.iter().zip(embeddings.iter()) {
            store.embed_entity(*eid, text, emb, timestamp)?;
            embedded += 1;
        }
    }

    Ok(embedded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;

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
            }],
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
}
