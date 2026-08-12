//! Store read path: current-state, per-entity, and time-travel fact queries.
//!
//! Split from `ops.rs` to keep the file-size ratchet honest (quipu-bu3
//! follow-up): these are the committed-tier read queries `ops`' write path
//! and the export/audit surfaces share.

use rusqlite::params;

use crate::error::Result;
use crate::types::{Fact, Op, Value};

use super::{AsOf, Store};

impl Store {
    // -- Read path --

    /// Return the current state: ROOT's asserted facts not yet retracted.
    ///
    /// **ROOT-scoped** (quipu #56). This filtered `op`/`valid_to` and nothing
    /// else, so once overlays existed it spanned every tenant's graph — the
    /// reasoner derived from overlay premises, `PageRank` counted them, and a
    /// full export leaked them into a ROOT dump. Committed reads are
    /// ROOT-scoped (Decision 4); use [`Store::current_facts_in_graph`] to read
    /// a specific graph.
    pub fn current_facts(&self) -> Result<Vec<Fact>> {
        self.current_facts_in_graph(crate::schema::ROOT_GRAPH)
    }

    /// Current asserted facts in ONE graph (quipu #36 subset export). `g = 0` is
    /// the ROOT / default committed graph; a named graph's `g` is the term id of
    /// its graph IRI. This is a graph's OWN facts (the same scope a
    /// `GRAPH <iri> { … }` read sees), not a composed overlay view.
    pub fn current_facts_in_graph(&self, g: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE op = 1 AND valid_to IS NULL AND g = ?1 \
             ORDER BY e, a",
        )?;
        Self::collect_facts(&mut stmt, params![g])
    }

    /// Return ROOT's facts for a specific entity (current state).
    ///
    /// **ROOT-scoped** (quipu #56). This is the selection `retract_triples`
    /// runs before committing retraction datums to ROOT, so a cross-graph read
    /// here meant a `/retract` could retract another graph's facts *into* ROOT
    /// — the exact "a retraction in graph A does not touch graph B" invariant
    /// #36 claims.
    pub fn entity_facts(&self, entity: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 AND op = 1 AND valid_to IS NULL AND g = ?2 \
             ORDER BY a",
        )?;
        Self::collect_facts(&mut stmt, params![entity, crate::schema::ROOT_GRAPH])
    }

    /// Time-travel query: return ROOT's facts as they were at a given point.
    ///
    /// **ROOT-scoped** (quipu #56) — time travel scopes *within* a graph
    /// (`docs/design/named-graphs.md` §1).
    pub fn facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        // `g = 0` is a literal, not a bound param, so it cannot disturb the
        // positional ?1/?2 indices the tx/valid_at clauses below depend on.
        let mut sql = String::from(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts WHERE op = 1 AND g = 0",
        );
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
    /// entity+attribute pair, **within ROOT** (quipu #56).
    ///
    /// Un-scoped, an overlay asserting a different value for an entity+attribute
    /// would read as a contradiction in its committed parent.
    pub fn detect_contradictions(&self, entity: i64, attribute: i64) -> Result<Vec<(Fact, Fact)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f1.e, f1.a, f1.v, f1.tx, f1.valid_from, f1.valid_to, f1.op, \
                    f2.e, f2.a, f2.v, f2.tx, f2.valid_from, f2.valid_to, f2.op \
             FROM facts f1 \
             JOIN facts f2 ON f1.e = f2.e AND f1.a = f2.a \
             WHERE f1.e = ?1 AND f1.a = ?2 \
               AND f1.op = 1 AND f2.op = 1 \
               AND f1.g = 0 AND f2.g = 0 \
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
             WHERE e = ?1 AND g = ?2 \
             ORDER BY tx, a",
        )?;
        Self::collect_facts(&mut stmt, params![entity, crate::schema::ROOT_GRAPH])
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

    /// Transactions with `id > since`, ordered, capped at `limit`. This is the
    /// cursor a watermarked poller (e.g. Shantytown's event subscription)
    /// advances so each poll is O(new transactions), not O(whole log).
    pub fn list_transactions_since(
        &self,
        since: i64,
        limit: i64,
    ) -> Result<Vec<crate::types::Transaction>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, actor, source FROM transactions \
             WHERE id > ?1 ORDER BY id LIMIT ?2",
        )?;
        let mut txns = Vec::new();
        let mut rows = stmt.query(params![since, limit])?;
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
             WHERE e = ?1 AND a = ?2 AND g = ?3 \
             ORDER BY tx",
        )?;
        Self::collect_facts(
            &mut stmt,
            params![entity, attribute, crate::schema::ROOT_GRAPH],
        )
    }

    // -- Internal --

    pub(crate) fn collect_facts(
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
