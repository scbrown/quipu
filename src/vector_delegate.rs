//! Delegated vector search for external embedding providers.
//!
//! When Quipu is used as a Bobbin dependency (knowledge feature), embeddings
//! are rebuildable derived data that belong in the index layer (Bobbin), not
//! the durable knowledge layer (Quipu). The [`VectorSearchDelegate`] trait
//! allows Bobbin to own the embedding lifecycle while Quipu routes search
//! calls to it transparently.

use std::sync::Arc;

use crate::error::Result;
use crate::vector::{KnowledgeVectorStore, VectorMatch};

/// Trait for delegating vector search to an external provider (e.g. Bobbin's
/// `LanceDB`).
///
/// Setting a delegate on [`crate::Store`] causes all search methods on
/// [`KnowledgeVectorStore`] to forward to the delegate, and auto-embedding
/// on write is skipped.
pub trait VectorSearchDelegate: Send + Sync {
    /// Search for similar entities by cosine similarity.
    fn vector_search(
        &self,
        query: &[f32],
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>>;

    /// Search with an optional predicate pushdown filter.
    fn vector_search_filtered(
        &self,
        query: &[f32],
        limit: usize,
        filter: Option<&str>,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        let oversample = if filter.is_some() { limit * 5 } else { limit };
        self.vector_search(query, oversample, valid_at)
    }

    /// Full-text search over entity text.
    fn text_search(
        &self,
        _query: &str,
        _limit: usize,
        _valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        Ok(vec![])
    }

    /// Return the number of current embeddings.
    fn vector_count(&self) -> Result<usize>;

    /// Ensure a full-text search index exists.
    fn ensure_fts_index(&self) -> Result<()> {
        Ok(())
    }
}

/// Wrapper that adapts a [`VectorSearchDelegate`] into a full
/// [`KnowledgeVectorStore`]. Write operations are no-ops because the delegate
/// owns the embeddings externally.
pub(crate) struct DelegatingVectorStore {
    delegate: Arc<dyn VectorSearchDelegate>,
}

impl DelegatingVectorStore {
    pub(crate) fn new(delegate: Arc<dyn VectorSearchDelegate>) -> Self {
        Self { delegate }
    }
}

impl KnowledgeVectorStore for DelegatingVectorStore {
    fn embed_entity(
        &self,
        _entity_id: i64,
        _text: &str,
        _embedding: &[f32],
        _valid_from: &str,
    ) -> Result<()> {
        Ok(()) // no-op: embeddings belong in the delegate
    }

    fn close_embedding(&self, _entity_id: i64, _valid_to: &str) -> Result<()> {
        Ok(()) // no-op: delegate manages lifecycle
    }

    fn vector_search(
        &self,
        query: &[f32],
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        self.delegate.vector_search(query, limit, valid_at)
    }

    fn vector_search_filtered(
        &self,
        query: &[f32],
        limit: usize,
        filter: Option<&str>,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        self.delegate
            .vector_search_filtered(query, limit, filter, valid_at)
    }

    fn vector_count(&self) -> Result<usize> {
        self.delegate.vector_count()
    }

    fn text_search(
        &self,
        query: &str,
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        self.delegate.text_search(query, limit, valid_at)
    }

    fn ensure_fts_index(&self) -> Result<()> {
        self.delegate.ensure_fts_index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake delegate that returns canned results for testing.
    struct FakeDelegate {
        results: Vec<VectorMatch>,
    }

    impl VectorSearchDelegate for FakeDelegate {
        fn vector_search(
            &self,
            _query: &[f32],
            limit: usize,
            _valid_at: Option<&str>,
        ) -> Result<Vec<VectorMatch>> {
            Ok(self.results.iter().take(limit).cloned().collect())
        }

        fn vector_count(&self) -> Result<usize> {
            Ok(self.results.len())
        }

        fn text_search(
            &self,
            _query: &str,
            limit: usize,
            _valid_at: Option<&str>,
        ) -> Result<Vec<VectorMatch>> {
            Ok(self.results.iter().take(limit).cloned().collect())
        }
    }

    #[test]
    fn delegate_forwards_search() {
        let canned = vec![VectorMatch {
            entity_id: 42,
            text: "delegated result".into(),
            score: 0.95,
            valid_from: "2026-01-01".into(),
            valid_to: None,
        }];
        let delegate = Arc::new(FakeDelegate {
            results: canned.clone(),
        });

        let mut store = crate::Store::open_in_memory().unwrap();
        store.set_vector_search_delegate(delegate);

        let vs = store.vector_store();
        let results = vs.vector_search(&[0.0; 8], 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "delegated result");
        assert_eq!(results[0].entity_id, 42);

        // text_search should also forward.
        let fts = vs.text_search("query", 10, None).unwrap();
        assert_eq!(fts.len(), 1);
        assert_eq!(fts[0].text, "delegated result");

        // count should forward.
        assert_eq!(vs.vector_count().unwrap(), 1);
    }

    #[test]
    fn delegate_write_is_noop() {
        let delegate = Arc::new(FakeDelegate { results: vec![] });
        let mut store = crate::Store::open_in_memory().unwrap();
        store.set_vector_search_delegate(delegate);

        let vs = store.vector_store();

        // embed_entity through delegate wrapper is a no-op.
        vs.embed_entity(1, "text", &[0.0; 8], "2026-01-01").unwrap();
        vs.close_embedding(1, "2026-02-01").unwrap();

        // Count comes from delegate (empty), not local `SQLite`.
        assert_eq!(vs.vector_count().unwrap(), 0);
    }

    #[test]
    fn no_delegate_uses_local_store() {
        let store = crate::Store::open_in_memory().unwrap();
        assert!(!store.has_vector_delegate());

        let eid = store.intern("http://example.org/test").unwrap();
        let emb: Vec<f32> = (0..8).map(|i| (1.0 + i as f32 * 0.1).sin()).collect();

        store
            .embed_entity(eid, "local entity", &emb, "2026-01-01")
            .unwrap();

        let vs = store.vector_store();
        let results = vs.vector_search(&emb, 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "local entity");
    }

    #[test]
    fn has_vector_delegate_reflects_state() {
        let mut store = crate::Store::open_in_memory().unwrap();
        assert!(!store.has_vector_delegate());

        let delegate = Arc::new(FakeDelegate { results: vec![] });
        store.set_vector_search_delegate(delegate);
        assert!(store.has_vector_delegate());
    }
}
