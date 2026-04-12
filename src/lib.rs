//! Quipu -- AI-native knowledge graph with strict ontology enforcement.
//!
//! This crate implements an immutable bitemporal EAVT fact log backed by `SQLite`,
//! designed as a foundation for agent-enforced knowledge graphs.

pub mod config;
pub mod context;
pub mod embedding;
pub mod episode;
pub mod error;
pub mod graph;
pub mod impact;
pub mod mcp;
#[cfg(feature = "lancedb")]
pub mod migration;
pub mod namespace;
#[cfg(feature = "onnx")]
pub mod onnx_embedder;
pub mod provider;
pub mod rdf;
pub mod reasoner;
pub mod reconcile;
pub mod resolution;
pub mod schema;
pub mod semweb;
#[cfg(feature = "shacl")]
pub mod shacl;
pub mod sparql;
pub mod store;
pub mod types;
pub mod vector;
pub mod vector_delegate;
#[cfg(feature = "lancedb")]
pub mod vector_lance;

pub use config::{
    EmbeddingConfig, FederationConfig, QuipuConfig, RemoteEndpoint, ResolutionConfig, ServerConfig,
    VectorBackend, VectorConfig,
};
pub use context::{
    ContextPipeline, ContextPipelineConfig, KnowledgeContext, KnowledgeEntity, KnowledgeFact,
    KnowledgeRelevance, tool_context, tool_unified_search,
};
pub use embedding::{EmbeddingProvider, build_entity_text};
pub use episode::{
    Episode, IngestResolutionOpts, IngestResult, episode_provenance, ingest_batch, ingest_episode,
    ingest_episode_with_resolution,
};
pub use error::{Error, Result};
pub use graph::{ProjectedGraph, tool_project};
pub use impact::{DEFAULT_HOPS, ImpactNode, ImpactOptions, ImpactReport, impact, speculate_remove};
pub use mcp::graphiti::tool_episodes_complete;
pub use mcp::impact::tool_impact;
pub use mcp::resolution::tool_resolve_entity;
pub use mcp::search::{tool_search_facts, tool_search_nodes};
pub use mcp::tools::{
    tool_cord, tool_episode, tool_hybrid_search, tool_retract, tool_search, tool_shapes,
    tool_unravel, tool_validate,
};
pub use mcp::{tool_definitions, tool_knot, tool_query, value_to_json};
#[cfg(feature = "lancedb")]
pub use migration::{MigrateResult, migrate_sqlite_to_lancedb};
#[cfg(feature = "onnx")]
pub use onnx_embedder::OnnxEmbeddingProvider;
pub use provider::{FederatedProvider, GraphProvider, LocalProvider, ProviderStatus};
pub use rdf::{export_rdf, ingest_rdf};
#[cfg(feature = "reactive-reasoner")]
pub use reasoner::reactive::ReactiveReasoner;
pub use reconcile::{
    GoResolver, ImportResolver, PythonResolver, ReconcileReport, RustResolver, default_resolvers,
    reconcile,
};
pub use resolution::{EntityCandidate, ResolutionResult, resolve_entity};
#[cfg(feature = "shacl")]
pub use shacl::{ValidationFeedback, Validator, validate_shapes};
pub use sparql::{
    QueryResult, TemporalContext, Triple, query as sparql_query,
    query_temporal as sparql_query_temporal,
};
pub use store::{Datum, Store};
#[cfg(feature = "reactive-reasoner")]
pub use store::{Delta, TransactObserver};
pub use types::{Fact, Op, Term, Transaction, Value};
pub use vector::{KnowledgeVectorStore, VectorMatch};
pub use vector_delegate::VectorSearchDelegate;
#[cfg(feature = "lancedb")]
pub use vector_lance::LanceVectorStore;
