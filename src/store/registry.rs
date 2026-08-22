//! Bitemporal shapes/ontologies registries (quipu #71) — close, don't
//! overwrite.
//!
//! Split from `mod.rs` (quipu-bu3). `shapes` and `ontologies` have identical
//! schemas and the identical problem, so they share one versioned
//! implementation rather than two that can drift.

use rusqlite::params;

use super::{AsOf, Store};
use crate::error::Result;

impl Store {
    // -- Shape storage --

    /// Store a named SHACL shape graph, **closing** any prior version
    /// (quipu #71).
    ///
    /// The prior open row's `valid_to` is set to `timestamp`, so the two
    /// versions form an adjacent, gapless pair and an as-of read lands on
    /// exactly one. Emits a `shapes.loaded` event carrying the tx watermark —
    /// before this, the audit spine had **no record that the rules changed**.
    pub fn load_shapes(&self, name: &str, turtle: &str, timestamp: &str) -> Result<()> {
        self.load_versioned("shapes", name, turtle, timestamp)?;
        self.emit_registry_event("shapes.loaded", name, timestamp)
    }

    /// Close a stored shape graph's current version. **Never deletes** — the
    /// history stays queryable, which is the whole point of #71.
    pub fn remove_shapes(&self, name: &str) -> Result<bool> {
        let closed = self.close_versioned("shapes", name, &crate::time::now_iso())?;
        if closed {
            self.emit_registry_event("shapes.removed", name, &crate::time::now_iso())?;
        }
        Ok(closed)
    }

    /// Get the CURRENT stored shapes as a list of (name, turtle, `loaded_at`).
    ///
    /// Open rows only, so this returns exactly what it returned before #71.
    pub fn list_shapes(&self) -> Result<Vec<(String, String, String)>> {
        self.list_versioned("shapes", None)
    }

    /// The stored shapes as they stood at `as_of` (quipu #71).
    pub fn list_shapes_as_of(&self, as_of: &AsOf) -> Result<Vec<(String, String, String)>> {
        self.list_versioned("shapes", Some(as_of))
    }

    /// Get all stored shapes concatenated as a single Turtle string.
    pub fn get_combined_shapes(&self) -> Result<Option<String>> {
        Self::combine(&self.list_shapes()?)
    }

    /// The combined shapes as they stood at `as_of` — the `as_of` twin
    /// `POST /validate` and the MCP tool use to validate against a prior
    /// version's semantics.
    pub fn get_combined_shapes_as_of(&self, as_of: &AsOf) -> Result<Option<String>> {
        Self::combine(&self.list_shapes_as_of(as_of)?)
    }

    fn combine(rows: &[(String, String, String)]) -> Result<Option<String>> {
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            rows.iter()
                .map(|(_, turtle, _)| turtle.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
        ))
    }

    // -- Shared bitemporal registry mechanics (quipu #71) --
    //
    // `shapes` and `ontologies` have identical schemas and the identical
    // problem, so they get one implementation rather than two that can drift.

    /// Close the open row for `name`, then insert a new open row.
    fn load_versioned(&self, table: &str, name: &str, turtle: &str, timestamp: &str) -> Result<()> {
        let tx = self.latest_tx_id()?;
        self.conn
            .execute_batch(&format!("SAVEPOINT quipu_load_{table}"))?;
        let result = (|| -> Result<()> {
            // Close strictly-earlier open versions. `valid_from < ?ts` matters:
            // a reload at the SAME instant must not close itself into a
            // zero-width window and then collide on the primary key.
            self.conn.execute(
                &format!(
                    "UPDATE {table} SET valid_to = ?2 \
                     WHERE name = ?1 AND valid_to IS NULL AND valid_from < ?2"
                ),
                params![name, timestamp],
            )?;
            // A reload at the same instant REPLACES that instant's version —
            // there is no meaningful ordering within one timestamp.
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE name = ?1 AND valid_from = ?2"),
                params![name, timestamp],
            )?;
            self.conn.execute(
                &format!(
                    "INSERT INTO {table} (name, turtle, loaded_at, valid_from, valid_to, tx) \
                     VALUES (?1, ?2, ?3, ?3, NULL, ?4)"
                ),
                params![name, turtle, timestamp, tx],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn
                    .execute_batch(&format!("RELEASE quipu_load_{table}"))?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch(&format!(
                    "ROLLBACK TO quipu_load_{table}; RELEASE quipu_load_{table}"
                ));
                Err(e)
            }
        }
    }

    /// Close the open row for `name`. Returns whether one was open.
    fn close_versioned(&self, table: &str, name: &str, timestamp: &str) -> Result<bool> {
        let affected = self.conn.execute(
            &format!("UPDATE {table} SET valid_to = ?2 WHERE name = ?1 AND valid_to IS NULL"),
            params![name, timestamp],
        )?;
        Ok(affected > 0)
    }

    /// Rows in effect now (`as_of = None`) or at a point in time / transaction.
    ///
    /// A row is in effect at `valid_at` when `valid_from <= valid_at` and it is
    /// either still open or closed strictly after — the half-open interval that
    /// makes adjacent versions unambiguous.
    fn list_versioned(
        &self,
        table: &str,
        as_of: Option<&AsOf>,
    ) -> Result<Vec<(String, String, String)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match as_of {
            None => (
                format!(
                    "SELECT name, turtle, loaded_at FROM {table} \
                     WHERE valid_to IS NULL ORDER BY name"
                ),
                vec![],
            ),
            Some(AsOf {
                valid_at: Some(at), ..
            }) => (
                format!(
                    "SELECT name, turtle, loaded_at FROM {table} \
                     WHERE valid_from <= ?1 AND (valid_to IS NULL OR valid_to > ?1) \
                     ORDER BY name"
                ),
                vec![Box::new(at.clone())],
            ),
            Some(AsOf { tx: Some(t), .. }) => (
                format!(
                    "SELECT name, turtle, loaded_at FROM {table} t1 \
                     WHERE t1.tx <= ?1 AND NOT EXISTS ( \
                         SELECT 1 FROM {table} t2 \
                         WHERE t2.name = t1.name AND t2.tx <= ?1 AND t2.valid_from > t1.valid_from \
                     ) ORDER BY name"
                ),
                vec![Box::new(*t)],
            ),
            Some(_) => (
                format!(
                    "SELECT name, turtle, loaded_at FROM {table} \
                     WHERE valid_to IS NULL ORDER BY name"
                ),
                vec![],
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(AsRef::as_ref).collect();
        let mut out = Vec::new();
        let mut rows = stmt.query(refs.as_slice())?;
        while let Some(row) = rows.next()? {
            out.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(out)
    }

    /// Append a registry-change event so the audit spine records that the rules
    /// moved. Carries the tx watermark, which is what makes an `as_of_tx`
    /// replay able to ask "which shapes were in force then".
    /// `pub(crate)` since quipu-gp5: the fork registry (`store::forks`) records
    /// `fork.created`/`fork.promoted`/`fork.dropped` through the same spine.
    pub(crate) fn emit_registry_event(
        &self,
        event_type: &str,
        name: &str,
        timestamp: &str,
    ) -> Result<()> {
        let tx = self.latest_tx_id()?;
        self.conn.execute(
            "INSERT INTO events (type, ts, subject, group_id, tx_id, payload) \
             VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
            params![
                event_type,
                timestamp,
                name,
                tx,
                serde_json::json!({ "name": name, "tx": tx }).to_string()
            ],
        )?;
        Ok(())
    }

    // -- Ontology storage --

    /// Store a named OWL ontology, **closing** any prior version (quipu #71).
    pub fn load_ontology(&self, name: &str, turtle: &str, timestamp: &str) -> Result<()> {
        // NOTE: &self, so the cache is invalidated by the /ontology tool after
        // this returns (see invalidate_owl_cache). Kept here as the reminder that
        // a stale cache would enforce yesterday's axioms.
        self.load_versioned("ontologies", name, turtle, timestamp)?;
        self.emit_registry_event("ontology.loaded", name, timestamp)
    }

    /// Close a stored ontology's current version. **Never deletes** — history
    /// stays queryable.
    pub fn remove_ontology(&self, name: &str) -> Result<bool> {
        let closed = self.close_versioned("ontologies", name, &crate::time::now_iso())?;
        if closed {
            self.emit_registry_event("ontology.removed", name, &crate::time::now_iso())?;
        }
        Ok(closed)
    }

    /// The stored ontologies as they stood at `as_of` (quipu #71).
    pub fn list_ontologies_as_of(&self, as_of: &AsOf) -> Result<Vec<(String, String, String)>> {
        self.list_versioned("ontologies", Some(as_of))
    }

    /// Get all stored ontologies as a list of (name, turtle, `loaded_at`).
    pub fn list_ontologies(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, turtle, loaded_at FROM ontologies WHERE valid_to IS NULL ORDER BY name",
        )?;
        let mut ontologies = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            ontologies.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(ontologies)
    }

    /// Get all stored ontologies concatenated as a single Turtle string.
    pub fn get_combined_ontologies(&self) -> Result<Option<String>> {
        let ontologies = self.list_ontologies()?;
        if ontologies.is_empty() {
            return Ok(None);
        }
        let combined = ontologies
            .iter()
            .map(|(_, turtle, _)| turtle.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Some(combined))
    }
}
