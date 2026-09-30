//! Single-use nonces for sealed decisions (aegis-kzt0ql.9.3).
//!
//! A decision presentation carries one nonce, and a verdict over it may be
//! accepted once. The spend and the verdict facts commit in ONE savepoint, so
//! either both land or neither does: a failed write gives the nonce back, and
//! an accepted verdict keeps it spent across restarts.
//!
//! Separate from `attestation_nonces` on purpose. That table is pruned after a
//! short clock-skew horizon because a session envelope older than the horizon
//! is refused before its nonce is consulted. A decision presentation can stay
//! open for hours, and its nonce must stay spent for as long as the
//! presentation could be answered. Human decisions are few, so this table is
//! never pruned.

use rusqlite::{Connection, OptionalExtension, params};

use super::{Datum, Store};
use crate::error::{Error, Result};

impl Store {
    pub(super) fn migrate_decision_nonces(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS decision_nonces (
                 nonce       TEXT PRIMARY KEY,
                 decision    TEXT NOT NULL,
                 consumed_at TEXT NOT NULL
             );",
        )?;
        Ok(())
    }

    /// Spend `nonce` for `decision` and transact `datums` atomically.
    ///
    /// `Ok(Some(tx))` when the nonce was fresh and the facts landed;
    /// `Ok(None)` when the nonce was already spent (a replay), in which case
    /// nothing is written.
    pub fn transact_spending_decision_nonce(
        &mut self,
        nonce: &str,
        decision: &str,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<Option<i64>> {
        self.conn.execute_batch("SAVEPOINT quipu_decision_nonce")?;
        let result = (|| -> Result<Option<i64>> {
            let inserted = self.conn.execute(
                "INSERT OR IGNORE INTO decision_nonces (nonce, decision, consumed_at)
                 VALUES (?1, ?2, ?3)",
                params![nonce, decision, timestamp],
            )?;
            if inserted == 0 {
                // An ignored insert is a replay only if the row is really
                // there; otherwise it is a schema fault, not an attacker.
                let present: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM decision_nonces WHERE nonce = ?1",
                        params![nonce],
                        |row| row.get(0),
                    )
                    .optional()?;
                return match present {
                    Some(_) => Ok(None),
                    None => Err(Error::InvalidValue(format!(
                        "decision nonce could not be recorded for {decision}"
                    ))),
                };
            }
            Ok(Some(self.transact(datums, timestamp, actor, source)?))
        })();
        match result {
            Ok(Some(tx)) => {
                self.conn.execute_batch("RELEASE quipu_decision_nonce")?;
                Ok(Some(tx))
            }
            Ok(None) => {
                self.conn.execute_batch(
                    "ROLLBACK TO quipu_decision_nonce; RELEASE quipu_decision_nonce",
                )?;
                Ok(None)
            }
            Err(e) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO quipu_decision_nonce; RELEASE quipu_decision_nonce",
                );
                Err(e)
            }
        }
    }

    /// Whether `nonce` has been spent.
    pub fn decision_nonce_spent(&self, nonce: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM decision_nonces WHERE nonce = ?1",
                params![nonce],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some())
    }
}
