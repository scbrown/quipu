//! The shared applier, slice 1 (`docs/design/shared-applier-api.md`).
//!
//! ONE component turns the committed feed into a read-only replica, so the
//! HA follower, a scoped follower and the in-page browser store never write
//! their own projection logic. This slice consumes the feed frames that ship
//! today (`/changes` pages and the chunked `begin`/`part`/`commit` frames of an
//! oversized transaction) as `(event, data)` text pairs, exactly as SSE or a
//! WebSocket delivers them. It does no I/O of its own, so it builds for the
//! native server and for `wasm32` alike.
//!
//! The rules are the contract's, in the order [`Applier::apply_frame`] checks
//! them:
//!
//! - **One source, one epoch.** The first frame binds the replica to its source
//!   `(store_id, log_epoch)`. A frame from any other source means a restore, a
//!   clone or removed history: re-bootstrap, never splice.
//! - **Whole transactions only.** Each source transaction is applied in ONE
//!   local savepoint together with the progress record, so the replica is
//!   always at a source transaction boundary. A chunked transaction is
//!   buffered, its digest chain checked part by part, and nothing is visible
//!   until the commit verifies.
//! - **Idempotent, and checked.** A transaction at or below the cursor is a
//!   duplicate only when its content digest equals the one retained when it
//!   was applied. A different digest, or none retained, stops the replica:
//!   its history and the source's disagree, and nothing more is applied.
//! - **No gaps.** A page covers every transaction in scope after the `since`
//!   it names, so a page (or a chunked transaction) whose `since` is above
//!   the cursor would skip whatever lies between: resume from the cursor.
//!   Until a bootstrap exists, a new replica starts at transaction 0. A page
//!   arriving while a chunked transaction is still open breaks the sequence
//!   the same way: the chunk is discarded and the caller resumes.
//! - **Refuse rather than guess.** A record the replica cannot reproduce
//!   exactly (a byte value sent as its length only, an unknown value shape)
//!   refuses the whole transaction. Writing an approximation would make the
//!   replica disagree with its source without saying so.
//!
//! Freshness is reported as (last applied transaction, time since the last
//! frame) and never claims a bound: a bounded claim needs the source's
//! freshness challenge, which a later slice adds.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

#[path = "applier_digest.rs"]
mod digest;
pub use digest::Stop;

/// The source log a replica follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The source store's identity, minted at its creation.
    pub store_id: String,
    /// The source log's epoch; re-minted by any history-removing operation.
    pub log_epoch: String,
}

/// The replica's cursor, persisted with every applied transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The source this replica follows.
    pub source: Source,
    /// The last source transaction applied in full.
    pub source_tx: i64,
}

/// What one frame did, and what the caller must do next.
///
/// Serialized (for the browser bridge) as an object tagged by `kind`, in
/// snake case: `{"kind": "applied", "source_txs": [..]}`, `{"kind": "resume",
/// "from": N}`, `{"kind": "duplicate"}`, and so on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// These source transactions are now visible, with the cursor moved past them.
    Applied {
        /// The source transactions applied, in commit order.
        source_txs: Vec<i64>,
    },
    /// A part of a chunked transaction is held; nothing is visible yet.
    Buffered {
        /// The chunked source transaction being assembled.
        source_tx: i64,
    },
    /// Everything in the frame was at or below the cursor.
    Duplicate,
    /// A page with no records: the feed has nothing new.
    Idle,
    /// The frame does not continue from the cursor (a dropped page, or a
    /// broken or interrupted chunk): nothing was applied; reconnect from `from`.
    Resume {
        /// The cursor to reconnect from (the last applied source transaction).
        from: i64,
    },
    /// The source changed: build a new replica from a fresh bootstrap.
    Rebootstrap {
        /// Why the replica cannot continue from its cursor.
        reason: String,
    },
    /// The frame cannot be applied exactly. Nothing from the refused source
    /// transaction was written; earlier whole transactions in the same page
    /// may have been, and [`Applier::progress`] says where the cursor is.
    Refused {
        /// Why the frame cannot be applied exactly.
        reason: String,
    },
    /// An event this slice does not act on (heartbeats, newer frame kinds).
    Ignored,
    /// A redelivered transaction does not match the one applied, or cannot
    /// be checked. Nothing from it was written, and every later frame answers
    /// the same until the replica is rebuilt (a new source still answers
    /// [`Outcome::Rebootstrap`]).
    Stopped {
        /// Which check failed.
        reason: Stop,
    },
}

/// What a reader may say about how current the replica is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Freshness {
    /// The last source transaction applied, if any.
    pub source_tx: Option<i64>,
    /// Milliseconds since the last frame of any kind; `None` before the first.
    pub last_frame_age_ms: Option<u64>,
    /// Always `false` in this slice: no freshness challenge has been answered.
    pub bounded: bool,
}

struct Chunk {
    source_tx: i64,
    parts: usize,
    /// Parts accepted so far; the next part's index must equal it.
    seen: usize,
    records: usize,
    digest: String,
    hasher: Sha256,
    buffered: Vec<Json>,
}

/// The applier for one replica store.
pub struct Applier {
    progress: Option<Progress>,
    chunk: Option<Chunk>,
    last_frame_ms: Option<u64>,
    stopped: Option<Outcome>,
}

const PROGRESS_DDL: &str = "CREATE TABLE IF NOT EXISTS replica_progress (\
     id INTEGER PRIMARY KEY CHECK (id = 1), \
     store_id TEXT NOT NULL, log_epoch TEXT NOT NULL, source_tx INTEGER NOT NULL)";

impl Applier {
    /// Open the applier for `store`, resuming its persisted cursor if any.
    ///
    /// # Errors
    /// [`Error::Sqlite`] if the progress record cannot be read.
    pub fn open(store: &Store) -> Result<Self> {
        store.conn.execute_batch(PROGRESS_DDL)?;
        store.conn.execute_batch(digest::APPLIED_TX_DDL)?;
        let progress = store
            .conn
            .query_row(
                "SELECT store_id, log_epoch, source_tx FROM replica_progress WHERE id = 1",
                [],
                |r| {
                    Ok(Progress {
                        source: Source {
                            store_id: r.get(0)?,
                            log_epoch: r.get(1)?,
                        },
                        source_tx: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(Self {
            progress,
            chunk: None,
            last_frame_ms: None,
            stopped: None,
        })
    }

    /// The persisted cursor, or `None` before the first applied transaction.
    #[must_use]
    pub fn progress(&self) -> Option<&Progress> {
        self.progress.as_ref()
    }

    /// What may be said about freshness at `now_ms` (never a bound here).
    #[must_use]
    pub fn freshness(&self, now_ms: u64) -> Freshness {
        Freshness {
            source_tx: self.progress.as_ref().map(|p| p.source_tx),
            last_frame_age_ms: self.last_frame_ms.map(|t| now_ms.saturating_sub(t)),
            bounded: false,
        }
    }

    /// Apply one feed frame, given as the transport delivered it.
    ///
    /// # Errors
    /// [`Error::Sqlite`] or a write error if the local store fails; the frame
    /// is then not applied and the cursor has not moved.
    pub fn apply_frame(
        &mut self,
        store: &mut Store,
        event: &str,
        data: &str,
        now_ms: u64,
    ) -> Result<Outcome> {
        self.last_frame_ms = Some(now_ms);
        if let Some(stopped) = self.stopped.clone() {
            // A new source still comes first: it means a new replica.
            let changed = serde_json::from_str::<Json>(data)
                .ok()
                .as_ref()
                .and_then(source_of)
                .and_then(|source| self.check_source(&source));
            return Ok(changed.unwrap_or(stopped));
        }
        match event {
            "quipu.changes" => self.apply_page(store, data),
            "quipu.changes.begin" => Ok(self.begin_chunk(data)),
            "quipu.changes.part" => Ok(self.chunk_part(data)),
            "quipu.changes.commit" => self.commit_chunk(store, data),
            "quipu.changes.tx_too_large" => Ok(Outcome::Refused {
                reason: "the source cannot send this transaction in frames".into(),
            }),
            _ => Ok(Outcome::Ignored),
        }
    }

    fn apply_page(&mut self, store: &mut Store, data: &str) -> Result<Outcome> {
        // A page never arrives inside a chunked transaction; if one does, the
        // chunk can no longer complete in order.
        if self.chunk.take().is_some() {
            return Ok(Outcome::Resume {
                from: self.resume_point(),
            });
        }
        let Ok(page) = serde_json::from_str::<Json>(data) else {
            return Ok(refused("page is not JSON"));
        };
        let Some(source) = source_of(&page) else {
            return Ok(refused("page does not name its source store and log epoch"));
        };
        if let Some(outcome) = self.check_source(&source) {
            return Ok(outcome);
        }
        if let Some(outcome) = self.check_since(&page) {
            return Ok(outcome);
        }
        let records = page["records"].as_array().cloned().unwrap_or_default();
        if records.is_empty() {
            return Ok(Outcome::Idle);
        }
        self.apply_records(store, &source, &records)
    }

    fn begin_chunk(&mut self, data: &str) -> Outcome {
        let Ok(begin) = serde_json::from_str::<Json>(data) else {
            return refused("begin frame is not JSON");
        };
        let (Some(source_tx), Some(parts), Some(records), Some(digest)) = (
            begin["source_tx"].as_i64(),
            begin["parts"].as_u64(),
            begin["records"].as_u64(),
            begin["digest"].as_str(),
        ) else {
            return refused("begin frame is incomplete");
        };
        if let Some(outcome) = self.check_since(&begin) {
            self.chunk = None;
            return outcome;
        }
        // A new begin replaces any chunk in progress: a reconnect replays an
        // interrupted transaction from its first part.
        self.chunk = Some(Chunk {
            source_tx,
            parts: usize::try_from(parts).unwrap_or(usize::MAX),
            records: usize::try_from(records).unwrap_or(usize::MAX),
            seen: 0,
            digest: digest.to_string(),
            hasher: Sha256::new(),
            buffered: Vec::new(),
        });
        Outcome::Buffered { source_tx }
    }

    fn chunk_part(&mut self, data: &str) -> Outcome {
        let resume = self.resume_point();
        let Some(chunk) = self.chunk.as_mut() else {
            return Outcome::Resume { from: resume };
        };
        let Ok(part) = serde_json::from_str::<Json>(data) else {
            self.chunk = None;
            return Outcome::Resume { from: resume };
        };
        let expected_before = hex::encode(chunk.hasher.clone().finalize());
        let in_order = part["source_tx"].as_i64() == Some(chunk.source_tx)
            && part["index"].as_u64() == u64::try_from(chunk.seen).ok()
            && part["digest_before"].as_str() == Some(expected_before.as_str());
        if !in_order {
            self.chunk = None;
            return Outcome::Resume { from: resume };
        }
        // The digest covers the part EXACTLY as sent, so it is hashed as text.
        chunk.hasher.update(data.as_bytes());
        chunk.seen += 1;
        chunk
            .buffered
            .extend(part["records"].as_array().cloned().unwrap_or_default());
        Outcome::Buffered {
            source_tx: chunk.source_tx,
        }
    }

    fn commit_chunk(&mut self, store: &mut Store, data: &str) -> Result<Outcome> {
        let resume = self.resume_point();
        let Some(chunk) = self.chunk.take() else {
            return Ok(Outcome::Resume { from: resume });
        };
        let Ok(commit) = serde_json::from_str::<Json>(data) else {
            return Ok(Outcome::Resume { from: resume });
        };
        let digest = hex::encode(chunk.hasher.clone().finalize());
        let complete = commit["source_tx"].as_i64() == Some(chunk.source_tx)
            && commit["digest"].as_str() == Some(chunk.digest.as_str())
            && digest == chunk.digest
            && chunk.seen == chunk.parts
            && chunk.buffered.len() == chunk.records;
        if !complete {
            return Ok(Outcome::Resume { from: resume });
        }
        let Some(source) = source_of(&commit) else {
            return Ok(refused(
                "chunked transaction does not name its source store and log epoch",
            ));
        };
        if let Some(outcome) = self.check_source(&source) {
            return Ok(outcome);
        }
        if let Some(outcome) = self.check_since(&commit) {
            return Ok(outcome);
        }
        self.apply_records(store, &source, &chunk.buffered)
    }

    fn resume_point(&self) -> i64 {
        self.progress.as_ref().map_or(0, |p| p.source_tx)
    }

    /// `None` when the frame continues from the cursor: its `since` is at or
    /// below it (the overlap is skipped as duplicates).
    fn check_since(&self, frame: &Json) -> Option<Outcome> {
        let Some(since) = frame["since"].as_i64() else {
            return Some(refused("frame does not name the cursor it answers (since)"));
        };
        let from = self.resume_point();
        (since > from).then_some(Outcome::Resume { from })
    }

    /// `None` when the frame's source is this replica's (or the first one).
    fn check_source(&self, source: &Source) -> Option<Outcome> {
        match &self.progress {
            Some(p) if p.source != *source => Some(Outcome::Rebootstrap {
                reason: format!(
                    "source changed from {}/{} to {}/{}",
                    p.source.store_id, p.source.log_epoch, source.store_id, source.log_epoch
                ),
            }),
            _ => None,
        }
    }

    /// Apply whole source transactions in commit order, each in one local
    /// savepoint with its progress record.
    fn apply_records(
        &mut self,
        store: &mut Store,
        source: &Source,
        records: &[Json],
    ) -> Result<Outcome> {
        let mut by_tx: BTreeMap<i64, Vec<&Json>> = BTreeMap::new();
        for record in records {
            let Some(tx) = record["tx"].as_i64() else {
                return Ok(refused("record without a source transaction"));
            };
            by_tx.entry(tx).or_default().push(record);
        }
        let mut applied = Vec::new();
        for (tx, records) in by_tx {
            if self.progress.as_ref().is_some_and(|p| tx <= p.source_tx) {
                if let Some(stop) = digest::check_duplicate(store, source, tx, &records)? {
                    self.stopped = Some(stop.clone());
                    return Ok(stop);
                }
                continue;
            }
            match apply_one_tx(store, source, tx, &records)? {
                None => {
                    self.progress = Some(Progress {
                        source: source.clone(),
                        source_tx: tx,
                    });
                    applied.push(tx);
                }
                Some(reason) => return Ok(Outcome::Refused { reason }),
            }
        }
        Ok(if applied.is_empty() {
            Outcome::Duplicate
        } else {
            Outcome::Applied {
                source_txs: applied,
            }
        })
    }
}

fn refused(reason: &str) -> Outcome {
    Outcome::Refused {
        reason: reason.to_string(),
    }
}

fn source_of(frame: &Json) -> Option<Source> {
    Some(Source {
        store_id: frame["source"]["store_id"].as_str()?.to_string(),
        log_epoch: frame["source"]["log_epoch"].as_str()?.to_string(),
    })
}

/// One source transaction, all-or-nothing. `Ok(Some(reason))` refuses it with
/// nothing written; `Ok(None)` committed it and the cursor.
fn apply_one_tx(
    store: &mut Store,
    source: &Source,
    tx: i64,
    records: &[&Json],
) -> Result<Option<String>> {
    let timestamp = records[0]["timestamp"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    // A transaction may touch several graphs: one local write per graph, all
    // inside the same savepoint.
    let mut by_graph: BTreeMap<String, Vec<&Json>> = BTreeMap::new();
    for record in records {
        let graph = record["graph"].as_str().unwrap_or("ROOT").to_string();
        by_graph.entry(graph).or_default().push(record);
    }
    store.conn.execute_batch("SAVEPOINT replica_apply")?;
    let result = (|| -> Result<Option<String>> {
        for (graph, records) in &by_graph {
            let mut datums = Vec::with_capacity(records.len());
            for record in records {
                match record_to_datum(store, record, &timestamp)? {
                    Ok(d) => datums.push(d),
                    Err(reason) => return Ok(Some(format!("source tx {tx}: {reason}"))),
                }
            }
            if graph == "ROOT" {
                store.transact(&datums, &timestamp, None, None)?;
            } else {
                let g = store.graph_create(graph)?;
                store.transact_to_graph(&datums, &timestamp, None, None, g)?;
            }
        }
        // Every record decoded above, so its exact encoding exists.
        let tx_digest = match digest::tx_digest(source, tx, records) {
            Ok(d) => d,
            Err(reason) => return Ok(Some(format!("source tx {tx}: {reason}"))),
        };
        store.conn.execute(
            "INSERT INTO replica_progress (id, store_id, log_epoch, source_tx) VALUES (1, ?1, ?2, ?3) \
             ON CONFLICT(id) DO UPDATE SET store_id = ?1, log_epoch = ?2, source_tx = ?3",
            params![source.store_id, source.log_epoch, tx],
        )?;
        digest::retain(store, source, tx, &tx_digest)?;
        Ok(None)
    })();
    match result {
        Ok(None) => {
            store.conn.execute_batch("RELEASE replica_apply")?;
            Ok(None)
        }
        other => {
            store
                .conn
                .execute_batch("ROLLBACK TO replica_apply; RELEASE replica_apply")?;
            other
        }
    }
}

/// One feed record as the datum that reproduces it, or why it cannot be
/// reproduced exactly. The outer `Result` is a store failure.
///
/// # Errors
/// [`Error::Sqlite`] if interning a term fails.
pub fn record_to_datum(
    store: &Store,
    record: &Json,
    tx_timestamp: &str,
) -> Result<std::result::Result<Datum, String>> {
    let (Some(entity), Some(attribute), Some(op)) = (
        record["entity"].as_str(),
        record["attribute"].as_str(),
        record["op"].as_str(),
    ) else {
        return Ok(Err("record lacks entity, attribute or op".into()));
    };
    let retract = op == "retract";
    let value_field = if retract { "old_value" } else { "value" };
    let value = match value_of(store, &record[value_field])? {
        Ok(v) => v,
        Err(reason) => return Ok(Err(reason)),
    };
    let valid_from = record["valid_from"]
        .as_str()
        .unwrap_or(tx_timestamp)
        .to_string();
    Ok(Ok(Datum {
        entity: store.intern(entity)?,
        attribute: store.intern(attribute)?,
        value,
        valid_from,
        // A retract names the fact it closes; its own row's valid time
        // describes the closing, not an interval to replay.
        valid_to: if retract {
            None
        } else {
            record["valid_to"].as_str().map(String::from)
        },
        op: if retract { Op::Retract } else { Op::Assert },
    }))
}

fn value_of(store: &Store, json: &Json) -> Result<std::result::Result<Value, String>> {
    use base64::Engine as _;
    Ok(Ok(match json {
        Json::String(s) => Value::Str(s.clone()),
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => match n.as_f64() {
                Some(f) => Value::Float(f),
                None => return Ok(Err(format!("unrepresentable number {n}"))),
            },
        },
        Json::Object(o) if o.get("ref").and_then(Json::as_str).is_some() => {
            Value::Ref(store.intern(o["ref"].as_str().unwrap_or_default())?)
        }
        Json::Object(o) if o.contains_key("lang") => Value::Lang {
            lexical: o["value"].as_str().unwrap_or_default().into(),
            lang: o["lang"].as_str().unwrap_or_default().into(),
        },
        Json::Object(o) if o.contains_key("datatype") => Value::Typed {
            lexical: o["value"].as_str().unwrap_or_default().into(),
            datatype: o["datatype"].as_str().unwrap_or_default().into(),
        },
        Json::Object(o) if o.get("base64").and_then(Json::as_str).is_some() => {
            match base64::engine::general_purpose::STANDARD
                .decode(o["base64"].as_str().unwrap_or_default())
            {
                Ok(bytes) => Value::Bytes(bytes),
                Err(_) => return Ok(Err("byte value is not valid base64".into())),
            }
        }
        // A source from before byte bodies were carried sends only the length.
        // Writing empty bytes would be a silent loss: refuse instead.
        Json::Object(o) if o.contains_key("bytes_len") => {
            return Ok(Err(
                "byte value sent as its length only; the source predates lossless bytes".into(),
            ));
        }
        other => return Ok(Err(format!("unreplayable value {other}"))),
    }))
}

/// Live facts of a store as resolved strings, so two stores with different
/// term ids compare by meaning; valid time is part of a fact's identity here.
/// Shared by the applier and feed-replay conformance tests.
#[cfg(test)]
type FactRow = (i64, i64, Vec<u8>, i64, String, Option<String>);

#[cfg(test)]
pub(crate) fn live_digest(store: &Store) -> std::collections::BTreeSet<String> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT e, a, v, g, valid_from, valid_to FROM facts \
             WHERE op = 1 AND retracted_tx IS NULL",
        )
        .unwrap();
    let rows: Vec<FactRow> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    rows.into_iter()
        .map(|(e, a, v, g, from, to)| {
            let value = match Value::from_bytes(&v).unwrap() {
                Value::Ref(id) => format!("ref:{}", store.resolve(id).unwrap()),
                Value::Bytes(b) => format!("bytes:{b:?}"),
                other => format!("{other:?}"),
            };
            let graph = if g == 0 {
                "ROOT".to_string()
            } else {
                store.resolve(g).unwrap()
            };
            format!(
                "{graph} {} {} {value} [{from}, {to:?})",
                store.resolve(e).unwrap(),
                store.resolve(a).unwrap()
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "applier_tests.rs"]
mod tests;
