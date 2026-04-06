//! The core fact log store backed by `SQLite`.

pub mod ops;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use rusqlite::{Connection, params};

use crate::config::EmbeddingConfig;
use crate::embedding::EmbeddingProvider;
use crate::error::{Error, Result};
use crate::schema::INIT_SQL;
use crate::types::Value;
use crate::vector::{KnowledgeVectorStore, VECTORS_SQL};
use crate::vector_delegate::{DelegatingVectorStore, VectorSearchDelegate};

/// The core fact log store backed by `SQLite`.
pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pub(crate) embedding_config: EmbeddingConfig,
    /// When set, vector search is delegated to an external provider (e.g.
    /// Bobbin's `LanceDB`). Auto-embedding on write is skipped.
    pub(crate) vector_delegate: Option<DelegatingVectorStore>,
    /// When set, vector operations use this local backend instead of the
    /// built-in `SQLite` vectors table. Unlike `vector_delegate`, this is a
    /// full read+write backend and auto-embedding still works.
    pub(crate) local_vector_backend: Option<Box<dyn KnowledgeVectorStore + Send + Sync>>,
}

/// A write-side assertion or retraction within a transaction.
pub struct Datum {
    pub entity: i64,
    pub attribute: i64,
    pub value: Value,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub op: crate::types::Op,
}

/// Temporal query parameters.
pub struct AsOf {
    /// Maximum transaction id to consider (None = latest).
    pub tx: Option<i64>,
    /// Point-in-time for valid-time filtering (None = current).
    pub valid_at: Option<String>,
}

impl Store {
    /// Open (or create) a Quipu store at the given path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Create an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(INIT_SQL)?;
        conn.execute_batch(VECTORS_SQL)?;
        Ok(Self {
            conn,
            embedding_provider: None,
            embedding_config: EmbeddingConfig::default(),
            vector_delegate: None,
            local_vector_backend: None,
        })
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

    /// Returns `true` if an embedding provider is attached.
    pub fn has_embedding_provider(&self) -> bool {
        self.embedding_provider.is_some()
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

    // -- Term dictionary --

    /// Intern an IRI, returning its integer id.
    pub fn intern(&self, iri: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO terms (iri) VALUES (?1)",
            params![iri],
        )?;
        let id: i64 =
            self.conn
                .query_row("SELECT id FROM terms WHERE iri = ?1", params![iri], |row| {
                    row.get(0)
                })?;
        Ok(id)
    }

    /// Resolve a term id back to its IRI.
    pub fn resolve(&self, id: i64) -> Result<String> {
        self.conn
            .query_row("SELECT iri FROM terms WHERE id = ?1", params![id], |row| {
                row.get(0)
            })
            .map_err(|_| Error::UnknownTerm(id))
    }

    /// Look up a term id by IRI, returning None if not interned.
    pub fn lookup(&self, iri: &str) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM terms WHERE iri = ?1")?;
        let mut rows = stmt.query(params![iri])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Retrieve a transaction by id.
    pub fn get_transaction(&self, tx_id: i64) -> Result<Option<crate::types::Transaction>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, timestamp, actor, source FROM transactions WHERE id = ?1")?;
        let mut rows = stmt.query(params![tx_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(crate::types::Transaction {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                actor: row.get(2)?,
                source: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    // -- Shape storage --

    /// Store a named SHACL shape graph.
    pub fn load_shapes(&self, name: &str, turtle: &str, timestamp: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO shapes (name, turtle, loaded_at) VALUES (?1, ?2, ?3)",
            params![name, turtle, timestamp],
        )?;
        Ok(())
    }

    /// Remove a stored shape graph by name.
    pub fn remove_shapes(&self, name: &str) -> Result<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM shapes WHERE name = ?1", params![name])?;
        Ok(affected > 0)
    }

    /// Get all stored shapes as a list of (name, turtle, `loaded_at`).
    pub fn list_shapes(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, turtle, loaded_at FROM shapes ORDER BY name")?;
        let mut shapes = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            shapes.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(shapes)
    }

    /// Get all stored shapes concatenated as a single Turtle string.
    pub fn get_combined_shapes(&self) -> Result<Option<String>> {
        let shapes = self.list_shapes()?;
        if shapes.is_empty() {
            return Ok(None);
        }
        let combined = shapes
            .iter()
            .map(|(_, turtle, _)| turtle.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Some(combined))
    }

    // -- SQL access (for SPARQL evaluator) --

    /// Prepare a SQL statement against the underlying connection.
    pub(crate) fn prepare(&self, sql: &str) -> Result<rusqlite::Statement<'_>> {
        Ok(self.conn.prepare(sql)?)
    }
}
