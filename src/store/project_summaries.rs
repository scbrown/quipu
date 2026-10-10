//! Opt-in coverage fence for project summaries. No endpoint, automatic registration,
//! recount or SDK routing is installed here. This is the invalidation foundation:
//! authoritative counter decoding/maintenance is a separate integration step.
//!
//! SQL triggers invalidate cached results in the SAME transaction as raw fact or
//! graph mutations, including writers that bypass Store callbacks. A cache can
//! be published only against the captured scope/identity/schema/generation. A
//! cached read is bounded and never recounts. Arbitrary replacement of a database
//! with an old byte-identical identity still requires a trusted storage epoch;
//! these hooks cannot certify an uncooperative writer that removes them.
use super::Store;
use crate::{Error, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
const SCHEMA_ID: &str = "project-summary-coverage-v1";
const MAX_BYTES: usize = 65_536;
/// Executable opt-in local metadata schema; excluded from portable shares.
pub const PROJECT_SUMMARY_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS project_summary_scopes (
  project INTEGER PRIMARY KEY,
  ephemeral INTEGER NOT NULL UNIQUE,
  store_id TEXT NOT NULL,
  schema_id TEXT NOT NULL,
  generation INTEGER NOT NULL DEFAULT 0 CHECK(typeof(generation)='integer' AND generation>=0),
  published_generation INTEGER,
  evaluated_at TEXT,
  valid_until TEXT,
  payload TEXT
);
";
/// A scope-bound coverage watermark, not a semantic-state transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryCoverage {
    /// Registered main graph term id.
    pub project: i64,
    /// Registered ephemeral graph term id.
    pub ephemeral: i64,
    /// Store identity observed with the watermark.
    pub store_id: String,
    /// Projection schema contract.
    pub schema_id: String,
    /// Raw coverage mutation generation; intentionally distinct from source tx.
    pub generation: i64,
}
/// Bounded cached result with explicit coverage and evaluation metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedProjectSummary {
    /// Scope and generation under which the payload was published.
    pub coverage: SummaryCoverage,
    /// Caller-supplied evaluation instant, not a clock read by the store.
    pub evaluated_at: String,
    /// Earliest known time boundary; cache refuses at or after this instant.
    pub valid_until: Option<String>,
    /// Actual aggregate schema/semantic stamp is the producer's responsibility.
    pub payload: String,
}
fn unavailable(reason: &str) -> Error {
    Error::CannotVerify(format!(
        "project summary unavailable: {reason}; no recount fallback"
    ))
}
fn definitions() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for table in ["facts", "graphs"] {
        for event in ["INSERT", "UPDATE", "DELETE"] {
            let name = format!("project_summary_{}_{}", table, event.to_lowercase());
            let condition = match event {
                "INSERT" => "project=NEW.g OR ephemeral=NEW.g",
                "DELETE" => "project=OLD.g OR ephemeral=OLD.g",
                _ => "project=NEW.g OR ephemeral=NEW.g OR project=OLD.g OR ephemeral=OLD.g",
            };
            out.push((name.clone(), format!("CREATE TRIGGER {name} AFTER {event} ON {table} BEGIN UPDATE project_summary_scopes SET generation=generation+1,published_generation=NULL WHERE {condition}; END")));
        }
    }
    for event in ["UPDATE", "DELETE"] {
        let name = format!("project_summary_identity_{}", event.to_lowercase());
        out.push((name.clone(), format!("CREATE TRIGGER {name} AFTER {event} ON store_identity BEGIN UPDATE project_summary_scopes SET generation=generation+1,published_generation=NULL; END")));
    }
    out
}
fn normalized(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
impl Store {
    fn summary_snapshot<T>(&self, read: impl FnOnce() -> Result<T>) -> Result<T> {
        // Instrumentation and coverage must come from one SQLite snapshot. A
        // concurrent schema/data change then causes publication to refuse the
        // snapshot upgrade rather than trusting a generation it bypassed.
        self.conn.execute_batch("SAVEPOINT quipu_summary_view")?;
        match read() {
            Ok(value) => {
                self.conn.execute_batch("RELEASE quipu_summary_view")?;
                Ok(value)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO quipu_summary_view; RELEASE quipu_summary_view")?;
                Err(error)
            }
        }
    }
    fn verify_summary_instrumentation(&self) -> Result<()> {
        if !self.attachments.is_empty() {
            return Err(unavailable("attached read views are not covered"));
        }
        for (name, expected) in definitions() {
            let sql: Option<String> = self
                .conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                    params![name],
                    |row| row.get(0),
                )
                .optional()?;
            if sql.as_deref().map(normalized) != Some(normalized(&expected)) {
                return Err(unavailable("coverage trigger missing or changed"));
            }
        }
        Ok(())
    }
    /// Explicit opt-in on a private/reviewed store. Requires both graphs already
    /// registered. Does not mark a summary valid or scan source facts.
    pub fn register_project_summary_scope(&self, project: i64, ephemeral: i64) -> Result<()> {
        if project <= 0 || ephemeral <= 0 || project == ephemeral || !self.attachments.is_empty() {
            return Err(unavailable("distinct registered local graph pair required"));
        }
        for graph in [project, ephemeral] {
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM graphs WHERE g=?1)",
                params![graph],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(unavailable("graph must be registered before projection"));
            }
        }
        self.conn
            .execute_batch("SAVEPOINT quipu_summary_register")?;
        let result = (|| {
            let initialized: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='project_summary_scopes')", [], |row| row.get(0))?;
            if !initialized {
                self.conn.execute_batch(PROJECT_SUMMARY_SCHEMA)?;
                for (_, sql) in definitions() {
                    self.conn.execute_batch(&sql)?;
                }
            }
            self.verify_summary_instrumentation()?;
            let identity: String = self.conn.query_row(
                "SELECT store_id FROM store_identity WHERE id=1",
                [],
                |row| row.get(0),
            )?;
            let overlap: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM project_summary_scopes WHERE (project IN (?1,?2) OR ephemeral IN (?1,?2)) AND NOT(project=?1 AND ephemeral=?2 AND store_id=?3 AND schema_id=?4))", params![project,ephemeral,identity,SCHEMA_ID], |row| row.get(0))?;
            if overlap {
                return Err(unavailable("scope overlaps or identity/schema changed"));
            }
            self.conn.execute("INSERT OR IGNORE INTO project_summary_scopes(project,ephemeral,store_id,schema_id) VALUES(?1,?2,?3,?4)", params![project,ephemeral,identity,SCHEMA_ID])?;
            Ok(())
        })();
        match result {
            Ok(()) => self.conn.execute_batch("RELEASE quipu_summary_register")?,
            Err(error) => {
                self.conn.execute_batch(
                    "ROLLBACK TO quipu_summary_register; RELEASE quipu_summary_register",
                )?;
                return Err(error);
            }
        }
        Ok(())
    }
    /// Capture a coverage token. The reconciler must read its source under the
    /// same SQLite snapshot, then publish using this token; never attach a global
    /// MAX(tx) or an SDK's zero receipt to the token as a semantic-state stamp.
    pub fn project_summary_coverage(&self, project: i64) -> Result<SummaryCoverage> {
        self.summary_snapshot(|| {
        self.verify_summary_instrumentation()?;
        self.conn.query_row("SELECT s.project,s.ephemeral,s.store_id,s.schema_id,s.generation FROM project_summary_scopes s JOIN store_identity i ON i.id=1 AND i.store_id=s.store_id JOIN graphs gp ON gp.g=s.project JOIN graphs ge ON ge.g=s.ephemeral WHERE s.project=?1 AND s.schema_id=?2", params![project,SCHEMA_ID], |row| {
            Ok(SummaryCoverage { project:row.get(0)?,ephemeral:row.get(1)?,store_id:row.get(2)?,schema_id:row.get(3)?,generation:row.get(4)? })
        }).optional()?.ok_or_else(|| unavailable("scope/identity/schema not current"))
        })
    }
    /// Publish a previously reconciled payload only if no covered mutation has
    /// occurred. This one statement joins the source store identity atomically.
    /// A stale reconcile refuses and cannot overwrite an intervening writer.
    pub fn publish_project_summary(
        &self,
        token: &SummaryCoverage,
        payload: &str,
        evaluated_at: &str,
        valid_until: Option<&str>,
    ) -> Result<()> {
        self.summary_snapshot(|| {
        self.verify_summary_instrumentation()?;
        if payload.len() > MAX_BYTES
            || evaluated_at.is_empty()
            || valid_until.is_some_and(|until| until <= evaluated_at)
        {
            return Err(unavailable("invalid evaluation or response budget"));
        }
        let _: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| unavailable("payload is not JSON"))?;
        let changed = self.conn.execute("UPDATE project_summary_scopes SET payload=?1,evaluated_at=?2,valid_until=?3,published_generation=generation WHERE project=?4 AND ephemeral=?5 AND store_id=?6 AND schema_id=?7 AND generation=?8 AND store_id=(SELECT store_id FROM store_identity WHERE id=1) AND EXISTS(SELECT 1 FROM graphs WHERE g=project) AND EXISTS(SELECT 1 FROM graphs WHERE g=ephemeral)", params![payload,evaluated_at,valid_until,token.project,token.ephemeral,token.store_id,token.schema_id,token.generation])?;
        if changed != 1 {
            return Err(unavailable("reconcile CAS lost"));
        }
        Ok(())
        })
    }
    /// Read the cached row only. Missing/dirty/expired/schema-incompatible or
    /// oversized results refuse; this method never scans facts or aggregates.
    /// Current summaries are not an implementation of historical `as_of`.
    pub fn cached_project_summary(&self, project: i64, now: &str) -> Result<CachedProjectSummary> {
        self.summary_snapshot(|| {
        if now.is_empty() {
            return Err(unavailable("evaluation clock required"));
        }
        self.verify_summary_instrumentation()?;
        let result = self.conn.query_row("SELECT s.ephemeral,s.store_id,s.schema_id,s.generation,s.evaluated_at,s.valid_until,CASE WHEN length(CAST(s.payload AS BLOB))<=?3 THEN s.payload ELSE NULL END FROM project_summary_scopes s JOIN store_identity i ON i.id=1 AND i.store_id=s.store_id JOIN graphs gp ON gp.g=s.project JOIN graphs ge ON ge.g=s.ephemeral WHERE s.project=?1 AND s.schema_id=?2 AND s.published_generation=s.generation AND s.evaluated_at<=?4 AND (s.valid_until IS NULL OR s.valid_until>?4)", params![project,SCHEMA_ID,i64::try_from(MAX_BYTES).map_err(|_| unavailable("response budget out of range"))?,now], |row| {
            Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,String>(4)?,row.get::<_,Option<String>>(5)?,row.get::<_,Option<String>>(6)?))
        }).optional()?;
        let Some((
            ephemeral,
            store_id,
            schema_id,
            generation,
            evaluated_at,
            valid_until,
            Some(payload),
        )) = result
        else {
            return Err(unavailable("missing, dirty, expired or oversized"));
        };
        let _: serde_json::Value = serde_json::from_str(&payload)
            .map_err(|_| unavailable("cached payload is not JSON"))?;
        Ok(CachedProjectSummary {
            coverage: SummaryCoverage {
                project,
                ephemeral,
                store_id,
                schema_id,
                generation,
            },
            evaluated_at,
            valid_until,
            payload,
        })
        })
    }
}
#[cfg(test)]
#[path = "project_summaries_tests.rs"]
mod tests;
