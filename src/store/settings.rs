//! Runtime configuration accessors and pluggable-backend wiring.
//!
//! Split from `mod.rs` (quipu-bu3). Everything here is a getter/setter over
//! policy the server installs at startup (configs, base namespace) or a
//! backend the store delegates to (embedding provider, vector backends,
//! signing identity, transact observers). No SQL runs in this module.

use std::sync::Arc;

use super::Store;
#[cfg(feature = "reactive-reasoner")]
use super::TransactObserver;
use crate::config::{
    EmbeddingConfig, GovernanceConfig, OwlConfig, ResolutionConfig, SearchConfig, ShaclConfig,
};
use crate::embedding::EmbeddingProvider;
use crate::error::Result;
use crate::vector::KnowledgeVectorStore;
use crate::vector_delegate::{DelegatingVectorStore, VectorSearchDelegate};

impl Store {
    /// Stable identity assigned once to this store and preserved on reopen.
    pub fn store_id(&self) -> Result<String> {
        self.conn
            .query_row(
                "SELECT store_id FROM store_identity WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Defer auto-embedding: writes collect embed work instead of running the
    /// ONNX embed under the caller's lock. The caller MUST drain with
    /// [`Self::take_deferred_embed`] after each write and finish via
    /// [`Self::apply_deferred_embed`], or new/changed entities silently get no
    /// embeddings (the server's write handlers drain uniformly).
    pub fn set_defer_auto_embed(&mut self, on: bool) {
        self.defer_auto_embed = on;
    }

    /// Take any pending deferred-embed work (None when the last write touched
    /// nothing embeddable, or deferral is off).
    pub fn take_deferred_embed(&mut self) -> Option<crate::embedding::DeferredEmbed> {
        self.pending_embed.take()
    }

    /// Write vectors computed outside the lock for previously-taken work.
    /// Entities whose text changed since collection are skipped (a later
    /// writer owns them — see `embedding::apply_deferred_embed`). Returns the
    /// number written.
    pub fn apply_deferred_embed(
        &self,
        work: &crate::embedding::DeferredEmbed,
        embeddings: &[Vec<f32>],
    ) -> Result<usize> {
        crate::embedding::apply_deferred_embed(self, work, embeddings)
    }

    /// Attach an embedding provider for auto-embedding on write.
    pub fn set_embedding_provider(&mut self, provider: Arc<dyn EmbeddingProvider>) {
        self.embedding_provider = Some(provider);
    }

    /// Get a mutable reference to the embedding config.
    pub fn embedding_config_mut(&mut self) -> &mut EmbeddingConfig {
        &mut self.embedding_config
    }

    /// Get a reference to the embedding config.
    pub fn embedding_config(&self) -> &EmbeddingConfig {
        &self.embedding_config
    }

    /// Get a mutable reference to the entity-resolution config.
    pub fn resolution_config_mut(&mut self) -> &mut ResolutionConfig {
        &mut self.resolution_config
    }

    /// Get a reference to the entity-resolution config.
    pub fn resolution_config(&self) -> &ResolutionConfig {
        &self.resolution_config
    }

    /// The base namespace new IRIs are minted under on the episode write paths
    /// (aegis-4h3x). The server sets this from `[quipu].base_ns` at startup.
    pub fn base_ns(&self) -> &str {
        &self.base_ns
    }

    /// Set the base namespace for minted IRIs. Called once at startup from
    /// config; a per-call `--base-ns` still overrides at the ingest call site.
    pub fn set_base_ns(&mut self, base_ns: impl Into<String>) {
        self.base_ns = base_ns.into();
    }

    /// Get a mutable reference to the search/limit config.
    pub fn search_config_mut(&mut self) -> &mut SearchConfig {
        &mut self.search_config
    }

    /// Get a reference to the search/limit config.
    pub fn search_config(&self) -> &SearchConfig {
        &self.search_config
    }

    /// Get a mutable reference to the graph-label floor config (quipu #68).
    pub fn labels_config_mut(&mut self) -> &mut crate::config::LabelsConfig {
        &mut self.labels_config
    }

    /// Get a reference to the graph-label floor config (quipu #68).
    pub fn labels_config(&self) -> &crate::config::LabelsConfig {
        &self.labels_config
    }

    /// Whether vectors live in the built-in `SQLite` table rather than a
    /// delegated or `LanceDB` backend (quipu #81).
    ///
    /// A delegate has no enumerate, so its embeddings cannot be re-keyed by
    /// IRI for a pack. `quipu pack --with-vectors` refuses rather than shipping
    /// a pack silently missing the vectors that were asked for.
    #[must_use]
    pub fn has_sqlite_vector_backend(&self) -> bool {
        self.vector_delegate.is_none() && self.local_vector_backend.is_none()
    }

    /// Get a mutable reference to the SHACL validation config.
    pub fn shacl_config_mut(&mut self) -> &mut ShaclConfig {
        &mut self.shacl_config
    }

    /// Get a reference to the SHACL validation config.
    pub fn shacl_config(&self) -> &ShaclConfig {
        &self.shacl_config
    }

    /// The OWL write-time constraint policy.
    pub fn owl_config(&self) -> &OwlConfig {
        &self.owl_config
    }

    /// Mutable OWL policy, for enabling write-time enforcement at runtime.
    pub fn owl_config_mut(&mut self) -> &mut OwlConfig {
        &mut self.owl_config
    }

    /// Get a mutable reference to the governance enforcement config.
    pub fn governance_config_mut(&mut self) -> &mut GovernanceConfig {
        &mut self.governance_config
    }

    /// Get a reference to the governance enforcement config.
    pub fn governance_config(&self) -> &GovernanceConfig {
        &self.governance_config
    }

    /// Set an external vector search delegate.
    ///
    /// When set, all vector search methods forward to the delegate and
    /// auto-embedding on write is skipped (embeddings belong in the delegate).
    pub fn set_vector_search_delegate(&mut self, delegate: Arc<dyn VectorSearchDelegate>) {
        self.vector_delegate = Some(DelegatingVectorStore::new(delegate));
    }

    /// Returns `true` if vector search is delegated to an external provider.
    pub fn has_vector_delegate(&self) -> bool {
        self.vector_delegate.is_some()
    }

    /// Set a local vector backend (e.g. `LanceDB`) that replaces the built-in
    /// `SQLite` vectors table for all vector operations.
    ///
    /// Unlike [`set_vector_search_delegate`], this is a full read+write backend
    /// and auto-embedding on write still works.
    pub fn set_local_vector_backend(
        &mut self,
        backend: Box<dyn KnowledgeVectorStore + Send + Sync>,
    ) {
        self.local_vector_backend = Some(backend);
    }

    /// Returns `true` if a local vector backend is configured.
    pub fn has_local_vector_backend(&self) -> bool {
        self.local_vector_backend.is_some()
    }

    /// Register a [`TransactObserver`] that will be called after every
    /// successful [`transact`](Store::transact) call.
    #[cfg(feature = "reactive-reasoner")]
    pub fn add_observer(&mut self, observer: Arc<dyn TransactObserver>) {
        self.observers.push(observer);
    }

    /// Returns `true` if an embedding provider is attached.
    pub fn has_embedding_provider(&self) -> bool {
        self.embedding_provider.is_some()
    }

    /// Attach a verdict-signing identity (v1 root of trust, Phase 0). When set,
    /// the committed-tier evaluator signs the verdicts it produces.
    pub fn set_signing_identity(&mut self, identity: Arc<crate::signing::SigningIdentity>) {
        self.signing = Some(identity);
    }

    /// The attached signing identity, if any.
    pub fn signing_identity(&self) -> Option<Arc<crate::signing::SigningIdentity>> {
        self.signing.clone()
    }

    /// Returns a clone of the embedding provider, if one is attached.
    pub fn embedding_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedding_provider.clone()
    }

    /// Embed a query string using the attached provider.
    ///
    /// Returns `None` if no provider is set. This allows search endpoints
    /// to accept natural-language `query` text and auto-embed it rather
    /// than requiring callers to supply pre-computed vectors.
    pub fn embed_query(&self, text: &str) -> Result<Option<Vec<f32>>> {
        match &self.embedding_provider {
            Some(provider) => Ok(Some(provider.embed_text(text)?)),
            None => Ok(None),
        }
    }
}
