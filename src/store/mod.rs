//! The core fact log store backed by SQLite.

pub mod ops;
#[cfg(test)]
mod tests;

use rusqlite::{Connection, params};

use crate::error::{Error, Result};
use crate::schema::INIT_SQL;
use crate::types::Value;
use crate::vector::VECTORS_SQL;

/// The core fact log store backed by SQLite.
pub struct Store {
    pub(crate) conn: Connection,
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
        Ok(Self { conn })
    }

    // -- Term dictionary --

    /// Intern an IRI, returning its integer id.
    pub fn intern(&self, iri: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO terms (iri) VALUES (?1)",
            params![iri],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM terms WHERE iri = ?1",
            params![iri],
            |row| row.get(0),
        )?;
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
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM terms WHERE iri = ?1")?;
        let mut rows = stmt.query(params![iri])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Retrieve a transaction by id.
    pub fn get_transaction(&self, tx_id: i64) -> Result<Option<crate::types::Transaction>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, actor, source FROM transactions WHERE id = ?1",
        )?;
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
        let affected = self.conn.execute(
            "DELETE FROM shapes WHERE name = ?1",
            params![name],
        )?;
        Ok(affected > 0)
    }

    /// Get all stored shapes as a list of (name, turtle, loaded_at).
    pub fn list_shapes(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, turtle, loaded_at FROM shapes ORDER BY name",
        )?;
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
