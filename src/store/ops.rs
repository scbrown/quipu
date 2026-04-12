//! Store write and read operations: transact, query, retract, time-travel.

use rusqlite::params;

use crate::embedding;
use crate::error::Result;
use crate::types::{Fact, Op, Value};

use super::{AsOf, Datum, Store};

impl Store {
    // -- Write path --

    /// Atomically write a batch of datums in a single transaction.
    /// Returns the transaction id.
    pub fn transact(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<i64> {
        // Use savepoint (not transaction) so transact() can nest inside
        // speculate()'s outer savepoint.
        let sp = self.conn.savepoint()?;
        sp.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![timestamp, actor, source],
        )?;
        let tx_id = sp.last_insert_rowid();

        {
            let mut insert = sp.prepare(
                "INSERT INTO facts (e, a, v, tx, valid_from, valid_to, op) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut close_assertion = sp.prepare(
                "UPDATE facts SET valid_to = ?1 \
                 WHERE e = ?2 AND a = ?3 AND v = ?4 AND op = 1 AND valid_to IS NULL",
            )?;
            for d in datums {
                let v_bytes = d.value.to_bytes();
                if d.op == Op::Retract {
                    close_assertion.execute(params![timestamp, d.entity, d.attribute, v_bytes,])?;
                }
                insert.execute(params![
                    d.entity,
                    d.attribute,
                    v_bytes,
                    tx_id,
                    d.valid_from,
                    d.valid_to,
                    d.op as i32,
                ])?;
            }
        }

        sp.commit()?;

        // Post-transact hook: auto-embed touched entities.
        // Skipped when a vector search delegate is set — embeddings belong in
        // the external index layer (e.g. Bobbin's LanceDB).
        if self.embedding_config.auto_embed
            && self.vector_delegate.is_none()
            && let Some(provider) = &self.embedding_provider
        {
            let entity_ids = embedding::touched_entity_ids(datums);
            embedding::auto_embed_entities(
                self,
                provider,
                &entity_ids,
                timestamp,
                self.embedding_config.embed_batch_size,
                datums,
            )?;
        }

        // Notify registered observers. Observers may call store.transact()
        // directly for per-rule provenance. Cloning the Arc vec first
        // avoids a borrow conflict with &mut self.
        #[cfg(feature = "reactive-reasoner")]
        {
            use super::Delta;

            if !self.observers.is_empty() {
                let mut asserts = Vec::new();
                let mut retracts = Vec::new();
                for d in datums {
                    match d.op {
                        Op::Assert => asserts.push(d.clone()),
                        Op::Retract => retracts.push(d.clone()),
                    }
                }
                let delta = Delta {
                    tx: tx_id,
                    asserts,
                    retracts,
                    source: source.map(String::from),
                };

                let observers: Vec<_> = self.observers.clone();
                for obs in &observers {
                    obs.after_commit(self, &delta)?;
                }
            }
        }

        Ok(tx_id)
    }

    /// Execute `f` against a speculative fork of the store with `hypothetical`
    /// datums applied. The fork is discarded after `f` returns — the underlying
    /// store is never mutated. Useful for counterfactual impact analysis
    /// ("what would break if I removed this?").
    ///
    /// Implementation: wraps the hypothetical write and query in a `SQLite`
    /// `SAVEPOINT` that is always rolled back. Because [`transact`](Store::transact)
    /// uses nested savepoints, observers (including the reactive reasoner) fire
    /// normally inside the speculative fork, so derived facts are recomputed.
    pub fn speculate<F, R>(&mut self, hypothetical: &[Datum], timestamp: &str, f: F) -> Result<R>
    where
        F: FnOnce(&Store) -> Result<R>,
    {
        self.conn.execute_batch("SAVEPOINT speculate")?;

        let result = self
            .transact(
                hypothetical,
                timestamp,
                Some("speculate"),
                Some("speculate"),
            )
            .and_then(|_| f(self));

        // Always rollback: speculative state must not persist.
        match self
            .conn
            .execute_batch("ROLLBACK TO speculate; RELEASE speculate")
        {
            Ok(()) => result,
            Err(rollback_err) => match result {
                Err(e) => Err(e),
                Ok(_) => Err(rollback_err.into()),
            },
        }
    }

    /// Retract all current facts for an entity (or just those matching a predicate).
    /// Returns `(tx_id, count)`.
    pub fn retract_entity(
        &mut self,
        entity: i64,
        predicate: Option<i64>,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<(i64, usize)> {
        let facts = if let Some(pred) = predicate {
            let all = self.entity_facts(entity)?;
            all.into_iter()
                .filter(|f| f.attribute == pred)
                .collect::<Vec<_>>()
        } else {
            self.entity_facts(entity)?
        };

        if facts.is_empty() {
            return Ok((0, 0));
        }

        let datums: Vec<Datum> = facts
            .iter()
            .map(|f| Datum {
                entity: f.entity,
                attribute: f.attribute,
                value: f.value.clone(),
                valid_from: f.valid_from.clone(),
                valid_to: None,
                op: Op::Retract,
            })
            .collect();

        let count = datums.len();
        let tx_id = self.transact(&datums, timestamp, actor, Some("retract"))?;
        Ok((tx_id, count))
    }

    // -- Read path --

    /// Return the current state: all asserted facts not yet retracted.
    pub fn current_facts(&self) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE op = 1 AND valid_to IS NULL \
             ORDER BY e, a",
        )?;
        Self::collect_facts(&mut stmt, params![])
    }

    /// Return facts for a specific entity (current state).
    pub fn entity_facts(&self, entity: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 AND op = 1 AND valid_to IS NULL \
             ORDER BY a",
        )?;
        Self::collect_facts(&mut stmt, params![entity])
    }

    /// Time-travel query: return facts as they were at a given point.
    pub fn facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let mut sql =
            String::from("SELECT e, a, v, tx, valid_from, valid_to, op FROM facts WHERE op = 1");
        if as_of.tx.is_some() {
            sql.push_str(" AND tx <= ?1");
        }
        if as_of.valid_at.is_some() {
            let param_idx = if as_of.tx.is_some() { "?2" } else { "?1" };
            sql.push_str(&format!(
                " AND valid_from <= {param_idx} AND (valid_to IS NULL OR valid_to > {param_idx})"
            ));
        }
        sql.push_str(" ORDER BY e, a");

        let mut stmt = self.conn.prepare(&sql)?;
        match (&as_of.tx, &as_of.valid_at) {
            (Some(tx), Some(vt)) => Self::collect_facts(&mut stmt, params![tx, vt]),
            (Some(tx), None) => Self::collect_facts(&mut stmt, params![tx]),
            (None, Some(vt)) => Self::collect_facts(&mut stmt, params![vt]),
            (None, None) => Self::collect_facts(&mut stmt, params![]),
        }
    }

    /// Detect contradictions: overlapping valid-time intervals for the same
    /// entity+attribute pair.
    pub fn detect_contradictions(&self, entity: i64, attribute: i64) -> Result<Vec<(Fact, Fact)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f1.e, f1.a, f1.v, f1.tx, f1.valid_from, f1.valid_to, f1.op, \
                    f2.e, f2.a, f2.v, f2.tx, f2.valid_from, f2.valid_to, f2.op \
             FROM facts f1 \
             JOIN facts f2 ON f1.e = f2.e AND f1.a = f2.a \
             WHERE f1.e = ?1 AND f1.a = ?2 \
               AND f1.op = 1 AND f2.op = 1 \
               AND f1.rowid < f2.rowid \
               AND f1.v != f2.v \
               AND f1.valid_from < COALESCE(f2.valid_to, '9999-12-31') \
               AND f2.valid_from < COALESCE(f1.valid_to, '9999-12-31')",
        )?;

        let mut pairs = Vec::new();
        let mut rows = stmt.query(params![entity, attribute])?;
        while let Some(row) = rows.next()? {
            let v1_bytes: Vec<u8> = row.get(2)?;
            let v2_bytes: Vec<u8> = row.get(9)?;
            let f1 = Fact {
                entity: row.get(0)?,
                attribute: row.get(1)?,
                value: Value::from_bytes(&v1_bytes)?,
                tx: row.get(3)?,
                valid_from: row.get(4)?,
                valid_to: row.get(5)?,
                op: Op::Assert,
            };
            let f2 = Fact {
                entity: row.get(7)?,
                attribute: row.get(8)?,
                value: Value::from_bytes(&v2_bytes)?,
                tx: row.get(10)?,
                valid_from: row.get(11)?,
                valid_to: row.get(12)?,
                op: Op::Assert,
            };
            pairs.push((f1, f2));
        }
        Ok(pairs)
    }

    /// Return the full history of an entity: all facts (asserts + retracts) ordered by tx.
    pub fn entity_history(&self, entity: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 \
             ORDER BY tx, a",
        )?;
        Self::collect_facts(&mut stmt, params![entity])
    }

    /// List all transactions ordered by id.
    pub fn list_transactions(&self) -> Result<Vec<crate::types::Transaction>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, timestamp, actor, source FROM transactions ORDER BY id")?;
        let mut txns = Vec::new();
        let mut rows = stmt.query(params![])?;
        while let Some(row) = rows.next()? {
            txns.push(crate::types::Transaction {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                actor: row.get(2)?,
                source: row.get(3)?,
            });
        }
        Ok(txns)
    }

    /// Return the full history of a specific entity+attribute pair.
    pub fn attribute_history(&self, entity: i64, attribute: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 AND a = ?2 \
             ORDER BY tx",
        )?;
        Self::collect_facts(&mut stmt, params![entity, attribute])
    }

    // -- Internal --

    fn collect_facts(
        stmt: &mut rusqlite::Statement<'_>,
        params: impl rusqlite::Params,
    ) -> Result<Vec<Fact>> {
        let mut facts = Vec::new();
        let mut rows = stmt.query(params)?;
        while let Some(row) = rows.next()? {
            let v_bytes: Vec<u8> = row.get(2)?;
            let op_raw: i32 = row.get(6)?;
            facts.push(Fact {
                entity: row.get(0)?,
                attribute: row.get(1)?,
                value: Value::from_bytes(&v_bytes)?,
                tx: row.get(3)?,
                valid_from: row.get(4)?,
                valid_to: row.get(5)?,
                op: Op::from_i32(op_raw).unwrap_or(Op::Assert),
            });
        }
        Ok(facts)
    }
}
