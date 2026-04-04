//! Quipu — AI-native knowledge graph with strict ontology enforcement.
//!
//! This crate implements an immutable bitemporal EAVT fact log backed by SQLite,
//! designed as a foundation for agent-enforced knowledge graphs.

pub mod error;
pub mod rdf;
pub mod schema;
pub mod store;
pub mod types;

pub use error::{Error, Result};
pub use rdf::{export_rdf, ingest_rdf};
pub use store::Store;
pub use types::{Fact, Op, Term, Transaction, Value};
