//! Deep freeze — relocate a whole named graph's rows into a read-only
//! archive pack, keeping the graph addressable and composable at query time.
//!
//! Design: `docs/design/graph-kinds-and-deep-freeze.md`. The safety argument,
//! stated once: "a thing that existed is a fact about the past" is a claim
//! about the COMPOSED store, not about which file holds the bytes. Freeze
//! deletes `main.facts` rows for a graph only after a full-history pack
//! (retracted rows, transactions, `retracted_tx` included) has been written
//! and hash-verified, the registry records the relocation, and the pack
//! re-attaches read-only — the same facts stay readable at the same graph
//! IRI, and durability genuinely becomes `backed`. This is camayoc's
//! *durability-declared relocation* (what-belongs-in-the-graph §4b), never a
//! write-time importance filter (§5).
//!
//! What is genuinely lost: `as_of_tx` time travel across the composition —
//! and quipu already refuses `as_of_tx` on stores with attachments
//! (`sparql/mod.rs`), so that boundary is pre-existing and honestly refused,
//! not newly silent. Valid-time travel survives: rows carry their windows
//! verbatim. Local embeddings are dropped in v1 (quipu-0v4).

use rusqlite::{OptionalExtension, params};
use std::path::Path;

use crate::error::{Error, Result};
use crate::lattice_kind::{DataKind, KIND_ARCHIVE, KIND_OPERATIONAL};
use crate::namespace::{QUIPU_FROZEN_AT, QUIPU_FROZEN_INTO, QUIPU_LIFECYCLE_STATE};
use crate::types::{Op, Value};

use super::attach::{self, Attachment};
use super::datasets::DatasetMember;
use super::{Datum, Store};

pub(crate) use super::freeze_io::frozen_attachments;

/// The auto-maintained dataset of frozen graphs — cold-composition opt-in #2:
/// `FROM <urn:quipu:dataset:frozen>` (or the `graph` request param) composes
/// every frozen graph without naming each window.
pub const FROZEN_DATASET_IRI: &str = "urn:quipu:dataset:frozen";

/// What a freeze did, for the caller and the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreezeReport {
    /// The frozen graph's IRI.
    pub graph_iri: String,
    /// The archive pack file.
    pub path: String,
    /// The attachment alias the pack mounts under.
    pub alias: String,
    /// `sha256:` hash of the canonical full history.
    pub content_hash: String,
    /// Fact rows relocated (full history, not just current).
    pub facts: usize,
    /// Transactions carried.
    pub transactions: usize,
}

impl Store {
    /// Freeze `graph_iri`: export its full history to a pack under `out_dir`,
    /// verify, delete the local rows, and re-attach the pack read-only.
    ///
    /// # Errors
    /// Refuses: ROOT and the meta-graph (reserved), overlay-class graphs
    /// (compose-only staging has no history to freeze), graphs contributed by
    /// an attachment (another owner's artifact), already-frozen graphs, and
    /// unregistered IRIs. Authority: the relabel inside goes through the
    /// meta-graph gate exactly as `set_graph_label` does, and the graph
    /// itself passes `enforce_graph_authority` when a principal chain is set.
    pub fn freeze_graph(
        &mut self,
        graph_iri: &str,
        out_dir: &str,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<FreezeReport> {
        let g = self
            .lookup(graph_iri)?
            .ok_or_else(|| Error::InvalidValue(format!("freeze: unknown graph '{graph_iri}'")))?;
        if g == crate::schema::ROOT_GRAPH {
            return Err(Error::InvalidValue(
                "freeze refuses ROOT: the default graph is the store, not a window".into(),
            ));
        }
        if g == self.meta_graph_id()? {
            return Err(Error::InvalidValue(
                "freeze refuses the label meta-graph: it governs every other graph".into(),
            ));
        }
        let row: Option<(String, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT class, source, lifecycle FROM main.graphs WHERE g = ?1",
                params![g],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((class, source, lifecycle)) = row else {
            return Err(Error::InvalidValue(format!(
                "freeze: '{graph_iri}' is interned but not a registered graph"
            )));
        };
        if class != "committed" {
            return Err(Error::InvalidValue(format!(
                "freeze refuses '{graph_iri}': it is {class}-class; only committed \
                 graphs carry the bitemporal history a freeze relocates"
            )));
        }
        if let Some(alias) = source {
            return Err(Error::InvalidValue(format!(
                "freeze refuses '{graph_iri}': it is contributed by attachment \
                 {alias:?} — another owner's artifact is not ours to relocate"
            )));
        }
        if lifecycle.as_deref() == Some("frozen") {
            return Err(Error::InvalidValue(format!(
                "freeze: '{graph_iri}' is already frozen"
            )));
        }
        self.enforce_graph_authority(g)?;

        // The canonical history and its hash, computed from main BEFORE
        // anything is written, so verification later proves the copy.
        let canonical = super::freeze_io::history_canonical(&self.conn, g)?;
        let content_hash = crate::pack::content_hash(&canonical);

        // Build (space 0), respace into a free space, verify.
        let alias = self.free_freeze_alias(graph_iri)?;
        let final_path = Path::new(out_dir)
            .join(format!("{alias}.qpack.db"))
            .to_string_lossy()
            .to_string();
        let build_path = format!("{final_path}.building");
        for p in [&build_path, &final_path] {
            if Path::new(p).exists() {
                return Err(Error::Store(format!(
                    "freeze: {p} already exists; refusing to overwrite an artifact"
                )));
            }
        }
        let (facts, transactions) = super::freeze_io::export_graph_history(
            self,
            graph_iri,
            g,
            &build_path,
            &content_hash,
            timestamp,
        )?;
        let space = self.next_free_space()?;
        let respaced =
            super::respace::respace_file(Path::new(&build_path), Path::new(&final_path), space);
        for suffix in ["", "-wal", "-shm"] {
            let p = format!("{build_path}{suffix}");
            if Path::new(&p).exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
        respaced?;
        super::freeze_io::verify_history_pack(&final_path, graph_iri, &content_hash)?;

        // The registry mutation, one savepoint: relabel (archive/backed, via
        // the ordinary meta-graph-gated label write), lifecycle facts, row
        // delete, frozen_packs row, dataset membership. On any failure the
        // savepoint takes it all back and only the pack file remains — an
        // inert artifact, named in the error.
        self.conn.execute_batch("SAVEPOINT quipu_freeze")?;
        let result = (|| -> Result<()> {
            let l = self.label_of_id(g)?;
            let label = super::labels::GraphLabel {
                freshness: l.freshness.value,
                durability: Some(crate::lattice::Durability::Backed),
                trust: l.trust.value.clone(),
                policy: l.policy.value.clone(),
                kind: Some(DataKind::parse(KIND_ARCHIVE)?),
            };
            self.set_graph_label(graph_iri, &label, timestamp, actor)?;
            self.write_lifecycle_facts(g, timestamp, actor, &content_hash)?;
            self.conn
                .execute("DELETE FROM main.facts WHERE g = ?1", params![g])?;
            self.conn.execute(
                "UPDATE main.graphs SET lifecycle = 'frozen' WHERE g = ?1",
                params![g],
            )?;
            self.conn.execute(
                "INSERT INTO frozen_packs \
                 (graph_iri, alias, path, space, content_hash, frozen_at, thawed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                params![graph_iri, alias, final_path, space, content_hash, timestamp],
            )?;
            self.frozen_dataset_update(graph_iri, true, timestamp, actor)?;
            Ok(())
        })();
        match result {
            Ok(()) => self.conn.execute_batch("RELEASE quipu_freeze")?,
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_freeze; RELEASE quipu_freeze");
                return Err(Error::Store(format!(
                    "freeze of '{graph_iri}' rolled back ({e}); the pack file \
                     {final_path} remains on disk and can be deleted"
                )));
            }
        }

        // Attach in-process so the freezer's next query already composes.
        self.mount_attachment(Attachment::read_only(&alias, &final_path))?;

        Ok(FreezeReport {
            graph_iri: graph_iri.to_string(),
            path: final_path,
            alias,
            content_hash,
            facts,
            transactions,
        })
    }

    /// Thaw `graph_iri`: verify the pack, detach it, import the full history
    /// back into the local graph, and reopen the graph for writes.
    ///
    /// The pack file is KEPT on disk (deleting it is a human act), and the
    /// `frozen_packs` row is closed with `thawed_at`, never deleted.
    pub fn thaw_graph(
        &mut self,
        graph_iri: &str,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<usize> {
        let row: Option<(i64, String, String, String)> = self
            .conn
            .query_row(
                "SELECT id, alias, path, content_hash FROM frozen_packs \
                 WHERE graph_iri = ?1 AND thawed_at IS NULL",
                params![graph_iri],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((row_id, alias, path, content_hash)) = row else {
            return Err(Error::InvalidValue(format!(
                "thaw: '{graph_iri}' is not frozen"
            )));
        };
        let g = self
            .lookup(graph_iri)?
            .ok_or_else(|| Error::Store(format!("thaw: '{graph_iri}' lost its term")))?;
        self.enforce_graph_authority(g)?;
        super::freeze_io::verify_history_pack(&path, graph_iri, &content_hash)?;

        // Detach first: the pack's registered graph rows leave main.graphs,
        // so the import below is the only source of this graph's rows.
        self.unmount_attachment(&alias)?;

        self.conn.execute_batch("SAVEPOINT quipu_thaw")?;
        let result = (|| -> Result<usize> {
            let facts = super::freeze_io::import_graph_history(self, &path, graph_iri)?;
            self.conn.execute(
                "UPDATE main.graphs SET lifecycle = NULL, data_kind = ?2 WHERE g = ?1",
                params![g, KIND_OPERATIONAL],
            )?;
            // Close the lifecycle fact (bitemporal close, not deletion) and
            // relabel: kind reverts to operational, durability STAYS backed —
            // the pack file still exists.
            let close = Datum {
                entity: g,
                attribute: self.intern(QUIPU_LIFECYCLE_STATE)?,
                value: Value::Str("frozen".into()),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Retract,
            };
            let meta_g = self.meta_graph_id()?;
            self.transact_to_graph(&[close], timestamp, actor, Some("graph-thaw"), meta_g)?;
            let l = self.label_of_id(g)?;
            let label = super::labels::GraphLabel {
                freshness: l.freshness.value,
                durability: Some(crate::lattice::Durability::Backed),
                trust: l.trust.value.clone(),
                policy: l.policy.value.clone(),
                kind: Some(DataKind::parse(KIND_OPERATIONAL)?),
            };
            self.set_graph_label(graph_iri, &label, timestamp, actor)?;
            self.conn.execute(
                "UPDATE frozen_packs SET thawed_at = ?2 WHERE id = ?1",
                params![row_id, timestamp],
            )?;
            self.frozen_dataset_update(graph_iri, false, timestamp, actor)?;
            Ok(facts)
        })();
        match result {
            Ok(facts) => {
                self.conn.execute_batch("RELEASE quipu_thaw")?;
                Ok(facts)
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_thaw; RELEASE quipu_thaw");
                // Remount so the store composes the archive again — the thaw
                // failed, but the graph must not vanish.
                self.mount_attachment(Attachment::read_only(&alias, &path))?;
                Err(e)
            }
        }
    }

    /// The directory holding this store's database file — the default
    /// destination for archive packs. `None` for an in-memory store.
    #[must_use]
    pub fn db_parent_dir(&self) -> Option<String> {
        let p = self.conn.path()?;
        if p.is_empty() {
            return None;
        }
        Some(
            Path::new(p)
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .to_string_lossy()
                .to_string(),
        )
    }

    /// The next term space free of main, active frozen packs, and mounted
    /// attachments.
    fn next_free_space(&self) -> Result<i64> {
        let mut max = self.local_term_space()?;
        let frozen: Option<i64> = self
            .conn
            .query_row("SELECT MAX(space) FROM frozen_packs", [], |r| r.get(0))
            .optional()?
            .flatten();
        max = max.max(frozen.unwrap_or(0));
        for a in &self.attachments {
            max = max.max(attach::attached_term_space(&self.conn, &a.alias)?);
        }
        Ok(max + 1)
    }

    /// A fresh `fz_…` alias derived from the graph's local name, unique
    /// against the frozen registry and mounted attachments.
    fn free_freeze_alias(&self, graph_iri: &str) -> Result<String> {
        let mut base: String = crate::pack::local_name(graph_iri)
            .to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() || c.is_ascii_digit() {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if base.is_empty() || !base.starts_with(|c: char| c.is_ascii_lowercase()) {
            base = format!("g{base}");
        }
        let taken = |alias: &str| -> Result<bool> {
            if self.attachments.iter().any(|a| a.alias == alias) {
                return Ok(true);
            }
            Ok(self
                .conn
                .query_row(
                    "SELECT 1 FROM frozen_packs WHERE alias = ?1",
                    params![alias],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        };
        let candidate = format!("fz_{base}");
        if !taken(&candidate)? {
            attach::validate_alias(&candidate)?;
            return Ok(candidate);
        }
        for n in 2..1000 {
            let candidate = format!("fz_{base}_{n}");
            if !taken(&candidate)? {
                attach::validate_alias(&candidate)?;
                return Ok(candidate);
            }
        }
        Err(Error::Store(format!(
            "freeze: no free alias for '{graph_iri}'"
        )))
    }

    /// The lifecycle facts a freeze writes into the meta-graph — the durable,
    /// queryable record of where the rows went and when.
    fn write_lifecycle_facts(
        &mut self,
        g: i64,
        timestamp: &str,
        actor: Option<&str>,
        content_hash: &str,
    ) -> Result<()> {
        let meta_g = self.meta_graph_id()?;
        let datums = vec![
            Datum {
                entity: g,
                attribute: self.intern(QUIPU_LIFECYCLE_STATE)?,
                value: Value::Str("frozen".into()),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: g,
                attribute: self.intern(QUIPU_FROZEN_INTO)?,
                value: Value::Str(content_hash.to_string()),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: g,
                attribute: self.intern(QUIPU_FROZEN_AT)?,
                value: Value::Str(timestamp.to_string()),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            },
        ];
        self.transact_to_graph(&datums, timestamp, actor, Some("graph-freeze"), meta_g)?;
        Ok(())
    }

    /// Add or remove `graph_iri` in the auto-maintained frozen dataset.
    fn frozen_dataset_update(
        &mut self,
        graph_iri: &str,
        add: bool,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<()> {
        let mut members: Vec<DatasetMember> = if self.is_dataset(FROZEN_DATASET_IRI)? {
            self.dataset_members(FROZEN_DATASET_IRI)?
        } else {
            Vec::new()
        };
        members.retain(|m| m.graph_iri != graph_iri);
        if add {
            members.push(DatasetMember {
                graph_iri: graph_iri.to_string(),
                ord: None,
            });
        }
        if members.is_empty() {
            self.dataset_remove(FROZEN_DATASET_IRI)?;
        } else {
            self.dataset_create(FROZEN_DATASET_IRI, &members, timestamp, actor)?;
        }
        Ok(())
    }

    /// Mount one attachment on the LIVE connection and rebuild the composed
    /// SQL — the in-process half of what `open_with_attachments` does at
    /// startup, so a freeze needs no server restart.
    fn mount_attachment(&mut self, attachment: Attachment) -> Result<()> {
        attach::attach_all(&self.conn, std::slice::from_ref(&attachment))?;
        let mut all = self.attachments.clone();
        all.push(attachment);
        let local_space = self.local_term_space()?;
        attach::verify_attached_schema(&self.conn, &all, local_space)?;
        attach::register_attached_graphs(&self.conn, &all)?;
        self.pack_manifests = attach::attached_pack_manifests(&self.conn, &all)?;
        self.facts_source = attach::build_facts_source(&self.conn, &all)?;
        self.resolve_sql = attach::build_resolve_sql(&all);
        super::alias::build_term_alias(&self.conn, &all)?;
        self.attachments = all;
        Ok(())
    }

    /// Detach one attachment, drop its registered graph rows, and rebuild the
    /// composed SQL.
    fn unmount_attachment(&mut self, alias: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM main.graphs WHERE source = ?1", params![alias])?;
        self.conn
            .execute_batch(&format!("DETACH DATABASE {alias}"))?;
        let all: Vec<Attachment> = self
            .attachments
            .iter()
            .filter(|a| a.alias != alias)
            .cloned()
            .collect();
        self.pack_manifests = attach::attached_pack_manifests(&self.conn, &all)?;
        self.facts_source = attach::build_facts_source(&self.conn, &all)?;
        self.resolve_sql = attach::build_resolve_sql(&all);
        super::alias::build_term_alias(&self.conn, &all)?;
        self.attachments = all;
        Ok(())
    }
}

#[cfg(test)]
#[path = "freeze_tests.rs"]
mod tests;
