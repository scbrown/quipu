//! Quipu — AI-native knowledge graph with strict ontology enforcement.
//!
//! This crate implements an immutable bitemporal EAVT fact log backed by SQLite,
//! designed as a foundation for agent-enforced knowledge graphs.

pub mod error;
pub mod mcp;
pub mod rdf;
pub mod schema;
pub mod shacl;
pub mod sparql;
pub mod store;
pub mod types;

pub use error::{Error, Result};
pub use mcp::{tool_cord, tool_definitions, tool_knot, tool_query, tool_unravel, tool_validate};
pub use rdf::{export_rdf, ingest_rdf};
pub use shacl::{validate_shapes, ValidationFeedback, Validator};
pub use sparql::{query as sparql_query, QueryResult};
pub use store::Store;
pub use types::{Fact, Op, Term, Transaction, Value};
