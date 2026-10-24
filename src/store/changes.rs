//! Change feed: the fact log served as a consumer contract (quipu-2ae).
//!
//! The append-only `facts` table has been a change stream all along; this
//! module gives it the API contract Spanner's change streams spell out
//! (docs/design/spanner-capabilities.md §4.4), adapted to a pull surface:
//!
//! - **Records** are `(tx, sequence, op, graph, entity, attribute, value)`,
//!   derived from rows — never a second log to drift from the first. A
//!   retract's op=0 row carries the retracted value, so the old value rides
//!   the record without a lookup. Each record also carries the row's valid
//!   time (`valid_from`, `valid_to`): without it a replica can only guess the
//!   commit time, and a fact valid since last year replays as valid since today.
//!   An assert's `valid_to` is the bound it was written with. A later retract
//!   closes the row in place, so a row a later transaction closed reports
//!   `valid_to: null` here and the closing arrives as that retract's record —
//!   otherwise a record would change after the fact and a replica would assert
//!   an already-closed fact that its retract then cannot find.
//! - **Cursor = transaction id, whole transactions only.** A page always ends
//!   on a tx boundary, so every cursor is a consistent prefix of commit
//!   history — a reader never observes half a transaction, which is the same
//!   contract Spanner words as "all transactions with commit ts ≤ T and none
//!   after".
//! - **Ordering**: per entity, records arrive in commit order (tx, then
//!   `rowid` — the write order inside the tx). Across entities there is no
//!   promise, exactly like Spanner's per-key contract.
//! - **Value capture modes** copied outright: `new_values` (a retract record
//!   identifies the fact but omits the value), `old_and_new_values` (a
//!   retract carries the value it ended as `old_value`), `new_row` (each
//!   record also carries the entity's full state as of that tx, so consumers
//!   skip the read-back Spanner's docs call out as the anti-pattern).
//! - **Watermark instead of heartbeat.** A pull surface has no idle stream to
//!   heartbeat on; every page carries `watermark_tx` + `watermark_timestamp`
//!   (the newest committed transaction) so an empty page with an advancing
//!   watermark reads as "idle", and a watermark that never moves reads as
//!   "check the writer" — the idle-vs-broken distinction the heartbeat exists
//!   for.
//! - **The log is permanent.** Unlike Spanner's 1–30 day retention, the fact
//!   log never garbage-collects, so a cursor never expires.

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::params;
use serde_json::{Value as Json, json};

use crate::error::Result;
use crate::types::Value;

use super::Store;

/// How much value state each change record carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// Asserts carry the value; retract records identify `(entity,
    /// attribute)` but omit the value.
    NewValues,
    /// Retract records also carry the value they ended, as `old_value`.
    OldAndNewValues,
    /// `OldAndNewValues` plus the entity's full state as of the record's
    /// transaction, under `row`.
    NewRow,
}

impl Capture {
    /// Parse the wire name; unknown names are an error, not a default —
    /// a consumer that misspells a capture mode must hear about it.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "new_values" => Some(Self::NewValues),
            "old_and_new_values" => Some(Self::OldAndNewValues),
            "new_row" => Some(Self::NewRow),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::NewValues => "new_values",
            Self::OldAndNewValues => "old_and_new_values",
            Self::NewRow => "new_row",
        }
    }
}

/// One page of the feed. `records` covers whole transactions; pass `next_tx`
/// back as the cursor to page forward (it stays put on an empty page, so
/// polling is a fixpoint).
#[derive(Debug)]
pub struct ChangePage {
    pub records: Vec<Json>,
    pub next_tx: i64,
    pub watermark_tx: i64,
    pub watermark_timestamp: Option<String>,
    pub capture: Capture,
    /// The store these records came from, and its log epoch. A follower that
    /// sees either change is reading a different history (a restore, a clone
    /// or removed records) and must re-bootstrap rather than keep its cursor.
    pub store_id: String,
    pub log_epoch: String,
}

impl ChangePage {
    #[must_use]
    pub fn to_json(&self) -> Json {
        json!({
            "records": self.records,
            "next_tx": self.next_tx,
            "watermark_tx": self.watermark_tx,
            "watermark_timestamp": self.watermark_timestamp,
            "capture": self.capture.as_str(),
            "source": {"store_id": self.store_id, "log_epoch": self.log_epoch},
        })
    }
}

/// The page query: the first `LIMIT ?2` transactions after the cursor `?1`,
/// optionally in graph `?3`.
///
/// `+g` keeps the graph test out of index selection. With no `sqlite_stat1`,
/// a bare `g = ?3` made SQLite pick `idx_geav (g=?)`, read EVERY row of the
/// graph and DISTINCT them in a temp B-tree, whatever the cursor. Measured on a
/// production copy (seeds board graph, 2.1M rows): 0.71-0.79s warm and 15.9s
/// cold for 50 transactions, at any cursor. That one read crossed camayoc
/// ingress's 2.5s guard (aegis-mus). On `idx_tx (tx>?)` the walk follows commit
/// order and stops at the page: <15ms with identical rows.
///
/// The trade: a graph with few rows read from an old cursor now walks every
/// row after the cursor (1.4s warm from cursor 0 on that copy). No production
/// caller reads that way; a `(g, tx)` index is the fix if one appears.
fn page_sql(graph: bool) -> &'static str {
    if graph {
        "SELECT DISTINCT tx FROM facts WHERE tx > ?1 AND +g = ?3 ORDER BY tx LIMIT ?2"
    } else {
        "SELECT DISTINCT tx FROM facts WHERE tx > ?1 ORDER BY tx LIMIT ?2"
    }
}

impl Store {
    /// Serve change records for transactions STRICTLY AFTER `since_tx`, at
    /// most `max_txs` transactions (whole transactions only). `graph`
    /// restricts to one graph id (`0` = ROOT); `None` spans all graphs, with
    /// each record naming its own.
    pub fn changes_after(
        &self,
        since_tx: i64,
        max_txs: usize,
        capture: Capture,
        graph: Option<i64>,
    ) -> Result<ChangePage> {
        // The page's transactions: those that wrote a row in scope. Retracts
        // insert an op=0 row at their own tx, so `tx > since` alone sees them.
        let (watermark_tx, watermark_timestamp) = self.latest_transaction()?;
        let mut stmt = self.conn.prepare(page_sql(graph.is_some()))?;
        let tx_ids: Vec<i64> = match graph {
            Some(g) => stmt
                .query_map(params![since_tx, max_txs as i64, g], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?,
            None => stmt
                .query_map(params![since_tx, max_txs as i64], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?,
        };

        let mut records = Vec::new();
        for tx in &tx_ids {
            self.records_for_tx(*tx, capture, graph, &mut records)?;
        }
        Ok(ChangePage {
            records,
            next_tx: tx_ids.last().copied().unwrap_or(since_tx),
            watermark_tx,
            watermark_timestamp,
            capture,
            store_id: self.store_id()?,
            log_epoch: self.log_epoch()?,
        })
    }

    fn latest_transaction(&self) -> Result<(i64, Option<String>)> {
        let row = self
            .conn
            .query_row(
                "SELECT id, timestamp FROM transactions ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .or_else(|_| Ok::<_, rusqlite::Error>((0, None)))?;
        Ok(row)
    }

    fn records_for_tx(
        &self,
        tx: i64,
        capture: Capture,
        graph: Option<i64>,
        out: &mut Vec<Json>,
    ) -> Result<()> {
        let (timestamp, actor, source) = self
            .conn
            .query_row(
                "SELECT timestamp, actor, source FROM transactions WHERE id = ?1",
                params![tx],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap_or((None, None, None));

        let mut sql = String::from(
            "SELECT e, a, v, g, op, valid_from, \
                          CASE WHEN retracted_tx IS NULL THEN valid_to END \
                          FROM facts WHERE tx = ?1",
        );
        if graph.is_some() {
            sql.push_str(" AND g = ?2");
        }
        // rowid is the write order inside the transaction — the per-entity
        // ordering promise rides on it.
        sql.push_str(" ORDER BY rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        type Row = (i64, i64, Vec<u8>, i64, i32, String, Option<String>);
        let rows: Vec<Row> = {
            let map = |r: &rusqlite::Row<'_>| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            };
            match graph {
                Some(g) => stmt
                    .query_map(params![tx, g], map)?
                    .collect::<std::result::Result<_, _>>()?,
                None => stmt
                    .query_map(params![tx], map)?
                    .collect::<std::result::Result<_, _>>()?,
            }
        };

        for (sequence, (e, a, v, g, op, valid_from, valid_to)) in rows.into_iter().enumerate() {
            let value = Value::from_bytes(&v)?;
            let op_name = match op {
                0 => "retract",
                2 => "tombstone",
                _ => "assert",
            };
            let mut record = json!({
                "tx": tx,
                "sequence": sequence,
                "timestamp": timestamp,
                "actor": actor,
                "source": source,
                "op": op_name,
                "graph": if g == 0 { "ROOT".to_string() } else { self.resolve(g)? },
                "entity": self.resolve(e)?,
                "attribute": self.resolve(a)?,
                "valid_from": valid_from,
                "valid_to": valid_to,
            });
            match (op_name, capture) {
                // A retract under new_values identifies the fact but not the
                // value it ended — the mode exists for consumers that only
                // mirror current state.
                ("retract", Capture::NewValues) => {}
                ("retract", _) => {
                    record["old_value"] = self.value_json(&value)?;
                }
                _ => {
                    record["value"] = self.value_json(&value)?;
                }
            }
            if capture == Capture::NewRow {
                record["row"] = self.row_as_of(e, g, tx)?;
            }
            out.push(record);
        }
        Ok(())
    }

    /// The entity's full state in `graph` as of transaction `tx` — the same
    /// as-of predicate the fork snapshot uses, so the two surfaces cannot
    /// disagree about what "live at tx N" means.
    fn row_as_of(&self, entity: i64, graph: i64, tx: i64) -> Result<Json> {
        let mut stmt = self.conn.prepare(
            "SELECT a, v FROM facts \
             WHERE e = ?1 AND g = ?2 AND op = 1 AND tx <= ?3 \
               AND (valid_to IS NULL OR retracted_tx > ?3) \
             ORDER BY a, rowid",
        )?;
        let rows: Vec<(i64, Vec<u8>)> = stmt
            .query_map(params![entity, graph, tx], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut row = serde_json::Map::new();
        for (a, v) in rows {
            let attr = self.resolve(a)?;
            let value = self.value_json(&Value::from_bytes(&v)?)?;
            match row.get_mut(&attr) {
                Some(Json::Array(list)) => list.push(value),
                Some(existing) => {
                    let first = existing.take();
                    *existing = json!([first, value]);
                }
                None => {
                    row.insert(attr, value);
                }
            }
        }
        Ok(Json::Object(row))
    }

    /// JSON for a stored value. Refs resolve to their IRI under `"ref"` so a
    /// consumer can tell an edge from a string that happens to look like one.
    pub(crate) fn value_json(&self, value: &Value) -> Result<Json> {
        Ok(match value {
            Value::Ref(id) => json!({"ref": self.resolve(*id)?}),
            Value::Str(s) => json!(s),
            Value::Int(i) => json!(i),
            Value::Float(f) => json!(f),
            Value::Bool(b) => json!(b),
            // The body rides along so a replica built from the feed can hold
            // the value; a length alone replays as a different fact.
            Value::Bytes(b) => json!({"bytes_len": b.len(), "base64": STANDARD.encode(b)}),
            Value::Lang { lexical, lang } => json!({"value": lexical, "lang": lang}),
            Value::Typed { lexical, datatype } => json!({"value": lexical, "datatype": datatype}),
        })
    }
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "changes_replay_tests.rs"]
mod replay_tests;

#[cfg(test)]
#[path = "changes_asof_tests.rs"]
mod asof_tests;
