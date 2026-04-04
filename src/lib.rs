//! Quipu -- AI-native knowledge graph with strict ontology enforcement.
//!
//! This crate implements an immutable bitemporal EAVT fact log backed by SQLite,
//! designed as a foundation for agent-enforced knowledge graphs.

pub mod config;
pub mod context;
pub mod episode;
pub mod error;
pub mod graph;
pub mod mcp;
pub mod provider;
pub mod rdf;
pub mod schema;
pub mod shacl;
pub mod sparql;
pub mod store;
pub mod types;
pub mod vector;

pub use config::{QuipuConfig, ServerConfig, FederationConfig, RemoteEndpoint};
pub use context::{tool_context, ContextPipeline, ContextPipelineConfig, KnowledgeContext, KnowledgeEntity, KnowledgeFact, KnowledgeRelevance};
pub use episode::{episode_provenance, ingest_batch, ingest_episode, Episode};
pub use error::{Error, Result};
pub use graph::{tool_project, ProjectedGraph};
pub use provider::{FederatedProvider, GraphProvider, LocalProvider, ProviderStatus};
pub use mcp::{tool_definitions, tool_knot, tool_query, value_to_json};
pub use mcp::tools::{tool_cord, tool_episode, tool_retract, tool_search, tool_shapes, tool_unravel, tool_validate};
pub use rdf::{export_rdf, ingest_rdf};
pub use shacl::{validate_shapes, ValidationFeedback, Validator};
pub use sparql::{query as sparql_query, query_temporal as sparql_query_temporal, QueryResult, TemporalContext, Triple};
pub use store::Store;
pub use types::{Fact, Op, Term, Transaction, Value};
pub use vector::VectorMatch;
