//! Quipu — AI-native knowledge graph with strict ontology enforcement.
//!
//! This crate implements an immutable bitemporal EAVT fact log backed by SQLite,
//! designed as a foundation for agent-enforced knowledge graphs.

pub mod episode;
pub mod error;
pub mod mcp;
pub mod rdf;
pub mod schema;
pub mod shacl;
pub mod sparql;
pub mod store;
pub mod types;
pub mod vector;

pub use episode::{ingest_episode, Episode};
pub use error::{Error, Result};
pub use mcp::{tool_cord, tool_definitions, tool_episode, tool_knot, tool_query, tool_retract, tool_search, tool_shapes, tool_unravel, tool_validate};
pub use rdf::{export_rdf, ingest_rdf};
pub use shacl::{validate_shapes, ValidationFeedback, Validator};
pub use sparql::{query as sparql_query, QueryResult};
pub use store::Store;
pub use types::{Fact, Op, Term, Transaction, Value};
pub use vector::VectorMatch;
