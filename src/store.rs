use rusqlite::{Connection, params};

use crate::error::{Error, Result};
use crate::schema::INIT_SQL;
use crate::types::{Fact, Op, Transaction, Value};

/// The core fact log store backed by SQLite.
pub struct Store {
    conn: Connection,
}

/// A write-side assertion or retraction within a transaction.
pub struct Datum {
    pub entity: i64,
    pub attribute: i64,
    pub value: Value,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub op: Op,
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
        Ok(Self { conn })
    }

    // ── Term dictionary ──────────────────────────────────────────

    /// Intern an IRI, returning its integer id.
    /// If the IRI already exists, returns the existing id.
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

    // ── Transactions ─────────────────────────────────────────────

    /// Retrieve a transaction by id.
    pub fn get_transaction(&self, tx_id: i64) -> Result<Option<Transaction>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, actor, source FROM transactions WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![tx_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Transaction {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                actor: row.get(2)?,
                source: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    // ── Write path ───────────────────────────────────────────────

    /// Atomically write a batch of datums in a single transaction.
    ///
    /// Returns the transaction id.
    pub fn transact(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![timestamp, actor, source],
        )?;
        let tx_id = tx.last_insert_rowid();

        {
            let mut insert = tx.prepare(
                "INSERT INTO facts (e, a, v, tx, valid_from, valid_to, op) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut close_assertion = tx.prepare(
                "UPDATE facts SET valid_to = ?1 \
                 WHERE e = ?2 AND a = ?3 AND v = ?4 AND op = 1 AND valid_to IS NULL",
            )?;
            for d in datums {
                let v_bytes = d.value.to_bytes();
                if d.op == Op::Retract {
                    // Close the original assertion by setting its valid_to.
                    close_assertion.execute(params![
                        timestamp,
                        d.entity,
                        d.attribute,
                        v_bytes,
                    ])?;
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

        tx.commit()?;
        Ok(tx_id)
    }

    // ── Read path ────────────────────────────────────────────────

    /// Return the current state: all asserted facts that have not been retracted
    /// and whose valid_to is NULL (i.e. still current).
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
    ///
    /// - `as_of.tx`: only consider facts up to this transaction id.
    /// - `as_of.valid_at`: only facts valid at this timestamp.
    pub fn facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        // Build query dynamically based on which filters are set.
        let mut sql = String::from(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts WHERE op = 1",
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
    /// entity+attribute pair (different values asserted for the same period).
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

    /// Return the full history of a specific entity+attribute pair.
    pub fn attribute_history(&self, entity: i64, attribute: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 AND a = ?2 \
             ORDER BY tx",
        )?;
        Self::collect_facts(&mut stmt, params![entity, attribute])
    }

    // ── SQL access (for SPARQL evaluator) ─────────────────────────

    /// Prepare a SQL statement against the underlying connection.
    pub(crate) fn prepare(&self, sql: &str) -> Result<rusqlite::Statement<'_>> {
        Ok(self.conn.prepare(sql)?)
    }

    // ── Internal ─────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn intern_and_resolve() {
        let store = test_store();
        let id = store.intern("http://example.org/Person").unwrap();
        assert!(id > 0);
        let iri = store.resolve(id).unwrap();
        assert_eq!(iri, "http://example.org/Person");

        // Interning the same IRI returns the same id.
        let id2 = store.intern("http://example.org/Person").unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn lookup_missing() {
        let store = test_store();
        assert_eq!(store.lookup("http://nope").unwrap(), None);
    }

    #[test]
    fn round_trip_write_read() {
        let mut store = test_store();

        let e = store.intern("http://example.org/alice").unwrap();
        let a_name = store.intern("http://example.org/name").unwrap();
        let a_age = store.intern("http://example.org/age").unwrap();

        let tx = store
            .transact(
                &[
                    Datum {
                        entity: e,
                        attribute: a_name,
                        value: Value::Str("Alice".into()),
                        valid_from: "2026-01-01".into(),
                        valid_to: None,
                        op: Op::Assert,
                    },
                    Datum {
                        entity: e,
                        attribute: a_age,
                        value: Value::Int(30),
                        valid_from: "2026-01-01".into(),
                        valid_to: None,
                        op: Op::Assert,
                    },
                ],
                "2026-04-04T00:00:00Z",
                Some("test"),
                Some("unit-test"),
            )
            .unwrap();

        assert!(tx > 0);

        // Current facts should include both.
        let facts = store.current_facts().unwrap();
        assert_eq!(facts.len(), 2);
        let values: Vec<&Value> = facts.iter().map(|f| &f.value).collect();
        assert!(values.contains(&&Value::Str("Alice".into())));
        assert!(values.contains(&&Value::Int(30)));

        // Entity facts.
        let efacts = store.entity_facts(e).unwrap();
        assert_eq!(efacts.len(), 2);

        // Transaction metadata.
        let txn = store.get_transaction(tx).unwrap().unwrap();
        assert_eq!(txn.actor.as_deref(), Some("test"));
        assert_eq!(txn.source.as_deref(), Some("unit-test"));
    }

    #[test]
    fn temporal_query() {
        let mut store = test_store();

        let e = store.intern("http://example.org/server").unwrap();
        let a = store.intern("http://example.org/status").unwrap();

        // TX 1: server was "active" from Jan to Mar.
        let tx1 = store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("active".into()),
                    valid_from: "2026-01-01".into(),
                    valid_to: Some("2026-03-01".into()),
                    op: Op::Assert,
                }],
                "2026-01-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        // TX 2: server became "decommissioned" from Mar onward.
        let _tx2 = store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("decommissioned".into()),
                    valid_from: "2026-03-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-03-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        // As of TX1 only, we should see the "active" fact.
        let facts = store
            .facts_as_of(&AsOf {
                tx: Some(tx1),
                valid_at: None,
            })
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, Value::Str("active".into()));

        // Valid at Feb (within "active" period), considering all TXs.
        let facts = store
            .facts_as_of(&AsOf {
                tx: None,
                valid_at: Some("2026-02-01".into()),
            })
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, Value::Str("active".into()));

        // Valid at April (only "decommissioned" is current).
        let facts = store
            .facts_as_of(&AsOf {
                tx: None,
                valid_at: Some("2026-04-01".into()),
            })
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, Value::Str("decommissioned".into()));

        // Current facts: only the one with valid_to IS NULL.
        let current = store.current_facts().unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].value, Value::Str("decommissioned".into()));
    }

    #[test]
    fn contradiction_detection() {
        let mut store = test_store();

        let e = store.intern("http://example.org/node").unwrap();
        let a = store.intern("http://example.org/ip").unwrap();

        // Two overlapping claims about the same entity+attribute.
        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("10.0.0.1".into()),
                    valid_from: "2026-01-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-01-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("10.0.0.2".into()),
                    valid_from: "2026-02-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-02-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        let contradictions = store.detect_contradictions(e, a).unwrap();
        assert_eq!(contradictions.len(), 1);
        assert_eq!(contradictions[0].0.value, Value::Str("10.0.0.1".into()));
        assert_eq!(contradictions[0].1.value, Value::Str("10.0.0.2".into()));
    }

    #[test]
    fn attribute_history_tracks_all_ops() {
        let mut store = test_store();

        let e = store.intern("http://example.org/svc").unwrap();
        let a = store.intern("http://example.org/port").unwrap();

        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Int(8080),
                    valid_from: "2026-01-01".into(),
                    valid_to: Some("2026-02-01".into()),
                    op: Op::Assert,
                }],
                "2026-01-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Int(9090),
                    valid_from: "2026-02-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-02-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        let history = store.attribute_history(e, a).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].value, Value::Int(8080));
        assert_eq!(history[1].value, Value::Int(9090));
    }

    #[test]
    fn value_round_trip() {
        let cases = vec![
            Value::Ref(42),
            Value::Str("hello world".into()),
            Value::Int(-999),
            Value::Float(3.14),
            Value::Bool(true),
            Value::Bool(false),
            Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        ];
        for v in cases {
            let bytes = v.to_bytes();
            let decoded = Value::from_bytes(&bytes).unwrap();
            assert_eq!(v, decoded, "round-trip failed for {v:?}");
        }
    }

    #[test]
    fn retract_hides_from_current() {
        let mut store = test_store();

        let e = store.intern("http://example.org/thing").unwrap();
        let a = store.intern("http://example.org/label").unwrap();

        // Assert a fact.
        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("old-label".into()),
                    valid_from: "2026-01-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-01-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        assert_eq!(store.current_facts().unwrap().len(), 1);

        // Retract it.
        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("old-label".into()),
                    valid_from: "2026-01-01".into(),
                    valid_to: None,
                    op: Op::Retract,
                }],
                "2026-02-01T00:00:00Z",
                None,
                None,
            )
            .unwrap();

        // After retraction, the original assertion's valid_to is set to the
        // retract timestamp, so it no longer appears in current_facts().
        let current = store.current_facts().unwrap();
        assert_eq!(current.len(), 0, "retracted fact should not appear in current state");

        // The history still shows both entries (assert + retract).
        let history = store.attribute_history(e, a).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].op, Op::Assert);
        assert_eq!(history[0].valid_to, Some("2026-02-01T00:00:00Z".into()));
        assert_eq!(history[1].op, Op::Retract);

        // Time-travel to before the retract still sees the fact.
        let before_retract = store
            .facts_as_of(&AsOf {
                tx: None,
                valid_at: Some("2026-01-15".into()),
            })
            .unwrap();
        assert_eq!(before_retract.len(), 1);
        assert_eq!(before_retract[0].value, Value::Str("old-label".into()));

        // Time-travel to after the retract sees nothing.
        let after_retract = store
            .facts_as_of(&AsOf {
                tx: None,
                valid_at: Some("2026-03-01".into()),
            })
            .unwrap();
        assert_eq!(after_retract.len(), 0);
    }
}
