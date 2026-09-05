//! Quipu -- AI-native knowledge graph with strict ontology enforcement.
//!
//! This crate implements an immutable bitemporal EAVT fact log backed by `SQLite`,
//! designed as a foundation for agent-enforced knowledge graphs.

pub mod align;
pub mod compact;
pub mod config;
mod config_load;
pub mod context;
pub mod derivation;
pub mod embedding;
pub mod episode;
pub mod error;
pub mod governance;
pub mod graph;
pub mod graph_view;
pub mod http_auth;
pub mod impact;
pub mod lattice;
pub mod lattice_fold;
pub mod lattice_kind;
pub mod mcp;
pub mod metrics;
#[cfg(feature = "lancedb")]
pub mod migration;
pub mod namespace;
#[cfg(feature = "onnx")]
pub mod onnx_embedder;
// `explain` resolves OWL axiom families through the `owl` module, so the two
// share the feature gate.
#[cfg(feature = "owl")]
pub mod explain;
#[cfg(feature = "owl")]
pub mod owl;
// Reactive OWL rides the observer infrastructure, which is gated behind
// `reactive-reasoner`; it needs both features.
#[cfg(all(feature = "owl", feature = "reactive-reasoner"))]
mod owl_reactive;
pub mod pack;
#[cfg(not(target_arch = "wasm32"))]
mod pack_load;
mod pack_turtle;
pub mod path;
pub mod proposal;
pub mod provider;
pub mod rdf;
mod rdf_export;
mod rdf_graph_store;
pub mod reasoner;
pub mod reconcile;
pub mod report;
pub mod request_usage;
pub mod resolution;
pub mod schema;
pub mod semweb;
#[cfg(not(target_arch = "wasm32"))]
pub mod session_attestation;
#[cfg(feature = "shacl")]
pub mod shacl;
#[cfg(feature = "shacl")]
pub mod shacl_context;
pub mod share;
// The delta FORMAT is wasm-safe; only its filesystem entry points are gated,
// inside the module. The wasm explorer produces the same `share-delta/v1`
// artifact the CLI does, and it can only do that if the builder compiles there
// (aegis-8fdp8d).
pub mod share_delta;
pub mod share_import;
#[cfg(not(target_arch = "wasm32"))]
pub mod share_merge;
pub mod share_transport;
pub mod signing;
pub mod sparql;
pub mod store;
pub mod time;
pub mod types;
pub mod vector;
pub mod vector_delegate;
#[cfg(feature = "lancedb")]
pub mod vector_lance;
pub mod vocabulary;
pub mod w3c;

/// The crate version this build was compiled from.
///
/// Exported because consumers that are not the CLI or the server still need to
/// say which quipu produced them — the wasm bundle in `wasm/explorer` reports it
/// beside a pack's own `producer.version`, so a reader can see whether the
/// engine reading the artifact is the engine that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The git commit this build was compiled from, as stamped by `build.rs`.
pub const GIT_SHA: &str = env!("QUIPU_GIT_SHA");

pub use rdf_export::{export_rdf_construct, export_rdf_group};

pub use config::{
    AttachmentConfig, EmbeddingConfig, FederationConfig, GovernanceConfig, QuipuConfig,
    RemoteEndpoint, ResolutionConfig, SearchConfig, ServerConfig, ShaclConfig, VectorBackend,
    VectorConfig, open_with_configured_attachments,
};
pub use context::{
    ContextPipeline, ContextPipelineConfig, KnowledgeContext, KnowledgeEntity, KnowledgeFact,
    KnowledgeRelevance, tool_context, tool_unified_search,
};
pub use derivation::{DerivationMethod, Rederivation};
pub use embedding::{DeferredEmbed, EmbeddingProvider, NO_PROVIDER_HELP, build_entity_text};
pub use episode::{
    Episode, IngestResolutionOpts, IngestResult, episode_provenance, ingest_batch, ingest_episode,
    ingest_episode_with_resolution,
};
pub use error::{Error, Result};
// `project` is the only way to BUILD a ProjectedGraph, so exporting page_rank +
// ProjectedGraph without it left the typed graph API unusable from outside the
// crate — consumers had to reach through `quipu::graph::` or fall back to the
// `tool_project` JSON adapter. Bobbin chose the adapter (bobbin-jdlkh), which is
// how it ended up needing `&mut Store` for a read-only PageRank: tool_project
// takes `&mut` solely for its community-persist branch. Exporting it closes the
// gap (aegis-nwuf / bobbin-kue26).
pub use graph::{
    Communities, PageRankConfig, ProjectedGraph, louvain, page_rank, persist_communities, project,
    tool_project,
};
pub use graph_view::tool_graph_view;
pub use impact::{DEFAULT_HOPS, ImpactNode, ImpactOptions, ImpactReport, impact, speculate_remove};
pub use mcp::align::{tool_align_apply, tool_align_decide, tool_align_propose};
#[cfg(feature = "owl")]
pub use mcp::explain::tool_explain;
pub use mcp::graphiti::tool_episodes_complete;
pub use mcp::impact::tool_impact;
pub use mcp::named_query::tool_ask;
#[cfg(feature = "owl")]
pub use mcp::owl::tool_load_ontology;
pub use mcp::path::{tool_path_backtest, tool_path_cone};
pub use mcp::proposal::{
    tool_accept_proposal, tool_list_proposals, tool_propose_schema_change, tool_reject_proposal,
};
pub use mcp::resolution::tool_resolve_entity;
pub use mcp::search::{tool_search_facts, tool_search_nodes};
pub use mcp::tools::{
    resolve_validation_shapes, tool_cord, tool_datasets, tool_episode, tool_graph_freeze,
    tool_graph_list, tool_graph_thaw, tool_hybrid_search, tool_queries, tool_retract,
    tool_retract_episode, tool_search, tool_set, tool_shapes, tool_subscriptions, tool_unravel,
    tool_validate,
};
pub use mcp::{
    inference_header, labels_json, query_inference, query_result, query_result_with_federation,
    tool_cooccurrence, tool_definitions, tool_export, tool_graph_create, tool_graph_label,
    tool_knot, tool_overlay_compose, tool_overlay_create, tool_overlay_write, tool_policy_check,
    tool_query, tool_query_with_federation, tool_verdict_verify, tool_verifier_authorized,
    value_to_json, value_to_json_compact, value_to_json_with_prefixes,
};
#[cfg(feature = "lancedb")]
pub use migration::{MigrateResult, migrate_sqlite_to_lancedb};
#[cfg(feature = "onnx")]
pub use onnx_embedder::OnnxEmbeddingProvider;
#[cfg(feature = "owl")]
pub use owl::{MaterializeReport, Ontology, OwlViolation};
#[cfg(all(feature = "owl", feature = "reactive-reasoner"))]
pub use owl_reactive::ReactiveOwl;
pub use proposal::{NewProposal, Proposal, ProposalKind, ProposalStatus};
pub use provider::{
    DeclaredLabel, FederatedProvider, FederatedQuery, GraphProvider, LocalProvider,
    ProviderOutcome, ProviderStatus,
};
#[cfg(feature = "remote")]
pub use provider::{RemoteProvider, federated_from_config};
pub use rdf::{export_rdf, export_rdf_subset, ingest_rdf};
pub use rdf_graph_store::{delete_rdf_graph, replace_rdf_graph};
#[cfg(feature = "reactive-reasoner")]
pub use reasoner::reactive::ReactiveReasoner;
pub use reconcile::{
    GoResolver, ImportResolver, PythonResolver, ReconcileReport, RustResolver, default_resolvers,
    reconcile,
};
pub use report::tool_report;
pub use resolution::{EntityCandidate, ResolutionResult, resolve_entity};
#[cfg(feature = "shacl")]
pub use shacl::{ValidationFeedback, Validator, validate_shapes};
pub use sparql::{
    GraphScope, QueryResult, TemporalContext, Triple, query as sparql_query,
    query_temporal as sparql_query_temporal,
};
pub use store::forks::{ForkDiff, ForkInfo, ForkPromotion};
pub use store::{Datum, Store};
#[cfg(feature = "reactive-reasoner")]
pub use store::{Delta, TransactObserver};
pub use types::{Fact, Op, Term, Transaction, Value};
pub use vector::{KnowledgeVectorStore, VectorMatch};
pub use vector_delegate::VectorSearchDelegate;
#[cfg(feature = "lancedb")]
pub use vector_lance::LanceVectorStore;
