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
//!
//! A spend row also names the verdict it admitted and the transaction that
//! recorded it. That is what lets a later re-verification tell a verdict that
//! went through `attest` from one written straight into the graph with a
//! captured signature: the second has no spend, or a spend that names another
//! verdict or another transaction (wu-rev-345 F1).

use rusqlite::{Connection, OptionalExtension, params};

use super::{Datum, Store};
use crate::error::{Error, Result};

/// What a spent decision nonce admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionNonceSpend {
    /// The decision the nonce was presented for.
    pub decision: String,
    /// The one verdict the spend admitted.
    pub verdict: String,
    /// The transaction that recorded that verdict.
    pub tx: Option<i64>,
}

impl Store {
    pub(super) fn migrate_decision_nonces(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS decision_nonces (
                 nonce       TEXT PRIMARY KEY,
                 decision    TEXT NOT NULL,
                 verdict     TEXT NOT NULL,
                 tx          INTEGER,
                 consumed_at TEXT NOT NULL
             );",
        )?;
        Ok(())
    }

    /// Spend `nonce` for `decision`'s `verdict` and transact `datums`
    /// atomically. The spend row records the verdict and the transaction.
    ///
    /// `Ok(Some(tx))` when the nonce was fresh and the facts landed;
    /// `Ok(None)` when the nonce was already spent (a replay), in which case
    /// nothing is written.
    #[allow(clippy::too_many_arguments)]
    pub fn transact_spending_decision_nonce(
        &mut self,
        nonce: &str,
        decision: &str,
        verdict: &str,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<Option<i64>> {
        self.conn.execute_batch("SAVEPOINT quipu_decision_nonce")?;
        let result = (|| -> Result<Option<i64>> {
            let inserted = self.conn.execute(
                "INSERT OR IGNORE INTO decision_nonces (nonce, decision, verdict, consumed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![nonce, decision, verdict, timestamp],
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
            let tx = self.transact(datums, timestamp, actor, source)?;
            self.conn.execute(
                "UPDATE decision_nonces SET tx = ?2 WHERE nonce = ?1",
                params![nonce, tx],
            )?;
            Ok(Some(tx))
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

    /// The spend of `nonce`, if it has been spent.
    pub fn decision_nonce_spend(&self, nonce: &str) -> Result<Option<DecisionNonceSpend>> {
        Ok(self
            .conn
            .query_row(
                "SELECT decision, verdict, tx FROM decision_nonces WHERE nonce = ?1",
                params![nonce],
                |row| {
                    Ok(DecisionNonceSpend {
                        decision: row.get(0)?,
                        verdict: row.get(1)?,
                        tx: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub(super) fn migrate_registry_amendment_nonces(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS registry_amendment_nonces (
                 nonce        TEXT PRIMARY KEY,
                 registration TEXT NOT NULL,
                 amendment    TEXT NOT NULL
             );",
        )?;
        Ok(())
    }

    /// Spend a trust-root registry amendment's nonce (aegis-kzt0ql.9.4).
    /// Called by the gate INSIDE the write's savepoint, so a refused write
    /// gives the nonce back. `false` means it was already spent.
    pub(crate) fn spend_registry_amendment_nonce(
        &self,
        nonce: &str,
        registration: &str,
        amendment: &str,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO registry_amendment_nonces (nonce, registration, amendment)
             VALUES (?1, ?2, ?3)",
            params![nonce, registration, amendment],
        )? == 1)
    }

    /// Transact a console bootstrap: the gate admits exactly `registration`
    /// as the first human key, and nothing else. CLI-only.
    pub(crate) fn transact_trust_root_bootstrap(
        &mut self,
        registration: &str,
        datums: &[Datum],
        timestamp: &str,
    ) -> Result<i64> {
        // Re-checked here, not only in `bootstrap()`: the history rule belongs
        // to the one path that can admit a first key (wu-rev-350 N1).
        if crate::governance::trust_root::ever_bootstrapped(self)? {
            return Err(Error::PolicyDenied(
                "trust root (aegis-kzt0ql.9.4): a human key has already been enrolled in this store"
                    .into(),
            ));
        }
        self.trust_root_bootstrap = Some(registration.to_string());
        let result = self.transact(
            datums,
            timestamp,
            Some("console"),
            Some("trust-root bootstrap"),
        );
        self.trust_root_bootstrap = None;
        result
    }
}
