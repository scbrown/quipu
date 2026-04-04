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
pub mod mcp;
pub mod namespace;
pub mod provider;
pub mod rdf;
pub mod schema;
#[cfg(feature = "shacl")]
pub mod shacl;
pub mod sparql;
pub mod store;
pub mod types;
pub mod vector;
#[cfg(feature = "lancedb")]
pub mod vector_lance;

pub use config::{EmbeddingConfig, FederationConfig, QuipuConfig, RemoteEndpoint, ServerConfig};
pub use context::{
    ContextPipeline, ContextPipelineConfig, KnowledgeContext, KnowledgeEntity, KnowledgeFact,
    KnowledgeRelevance, tool_context, tool_unified_search,
};
pub use embedding::{EmbeddingProvider, build_entity_text};
pub use episode::{Episode, episode_provenance, ingest_batch, ingest_episode};
pub use error::{Error, Result};
pub use graph::{ProjectedGraph, tool_project};
pub use mcp::tools::{
    tool_cord, tool_episode, tool_hybrid_search, tool_retract, tool_search, tool_shapes,
    tool_unravel, tool_validate,
};
pub use mcp::{tool_definitions, tool_knot, tool_query, value_to_json};
pub use provider::{FederatedProvider, GraphProvider, LocalProvider, ProviderStatus};
pub use rdf::{export_rdf, ingest_rdf};
#[cfg(feature = "shacl")]
pub use shacl::{ValidationFeedback, Validator, validate_shapes};
pub use sparql::{
    QueryResult, TemporalContext, Triple, query as sparql_query,
    query_temporal as sparql_query_temporal,
};
pub use store::Store;
pub use types::{Fact, Op, Term, Transaction, Value};
pub use vector::{KnowledgeVectorStore, VectorMatch};
#[cfg(feature = "lancedb")]
pub use vector_lance::LanceVectorStore;
