//! Event log: in-transaction emission + pull API (event-log P1).
//!
//! Every committed ROOT-graph write appends semantic change events to the
//! `events` table INSIDE the same `SQLite` savepoint as the fact writes, so:
//!   - offset order == commit order (single `SQLite` writer, AUTOINCREMENT PK);
//!   - a rolled-back transaction leaves NO events (no torn/phantom events);
//!   - a consumer that replays from an offset sees exactly the committed
//!     history, in order, at-least-once (dedup by offset is the consumer's
//!     side of the event-log delivery contract).
//!
//! Event taxonomy (P1):
//!   episode.ingested   an /episode write committed (subject = episode IRI)
//!   entity.added       an entity received its FIRST facts ever
//!   entity.updated     an existing entity gained/lost facts
//!   edge.added         a Ref-valued assertion (`subject_preexisting`: whether
//!                      the subject existed before this tx — Stiwi's "new
//!                      relationship on an EXISTING entity" filter)
//!   edge.retracted     a Ref-valued retraction
//!   type.new           a node TYPE observed for the first time in the store
//!   predicate.new      a predicate IRI observed for the first time
//!
//! type.new / predicate.new are backed by the `schema_terms` seen-table and
//! fire exactly once per term for the store's lifetime (first sight), per the P1 spec. `rdf:type` itself never emits predicate.new — the type
//! system's own plumbing is covered by type.new, not reported as a predicate.
//!
//! Beyond the P1 taxonomy, the spine also carries registry-change events
//! (`shapes.loaded`, `ontology.loaded`, `fork.*` — see `registry.rs`) and,
//! since camayoc-0d3, `write.refused` — the durable record of a write a gate
//! REFUSED, written AFTER the refused write's savepoint rolled back (see the
//! refusals section below).
//!
//! OVERLAY writes (g != 0) do NOT emit events in P1: overlays are transient,
//! compose-only staging (#36); announcing their writes as graph-change events
//! would double-fire when the content is promoted to ROOT. Recorded here so
//! nobody reads the silence as a bug: only ROOT (g=0) commits are events.

use rusqlite::params;
use serde_json::json;
use std::collections::{HashMap, HashSet};

use crate::error::Result;
use crate::types::Value;

use super::{Datum, Store};

/// Versioned event schema tag carried on every event served by the API.
pub const EVENT_SCHEMA: &str = "quipu.event/v1";

/// Event type recorded when a write gate REFUSES a write (camayoc-0d3).
///
/// A refused write never enters the graph, so nothing durable used to record
/// that it was attempted — the incident-rate denominator ("how many writes
/// were attempted and refused, by which gate") had to live with the refusal
/// path or nowhere. Payload: `{gate, graph, actor, source, reason,
/// refused_datums}` — identifying METADATA only, never the refused datum
/// bodies (refused payloads can be junk or sensitive).
pub const REFUSAL_EVENT: &str = "write.refused";

/// The gates that can stamp a `write.refused` event.
pub const REFUSAL_GATES: [&str; 5] = ["shacl", "policy", "authority", "owl", "placement"];

/// Refusal reasons are terse machine-usable identifiers (shape/policy id,
/// constraint name), not documents; anything longer is truncated.
const REFUSAL_REASON_MAX: usize = 500;

/// One gate refusal, staged by the failing gate inside the write's savepoint
/// and recorded by `transact_to_graph`'s error path AFTER the rollback.
#[derive(Debug, Clone)]
pub(crate) struct PendingRefusal {
    /// Which gate refused (one of [`REFUSAL_GATES`]).
    pub gate: &'static str,
    /// Terse machine-usable reason (policy id / constraint name in the gate's
    /// own error text).
    pub reason: String,
    /// How many datums the refused write carried (metadata, not bodies).
    pub refused_datums: usize,
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// One row from the event log, as served by the pull API.
#[derive(Debug, Clone)]
pub struct EventRow {
    /// Monotonic log position; the consumer's cursor currency.
    pub offset: i64,
    /// Event type, e.g. `episode.ingested`, `edge.added`, `type.new`.
    pub event_type: String,
    /// Timestamp of the transaction that emitted this event.
    pub ts: String,
    /// Primary IRI this event is about (entity, type, or predicate).
    pub subject: Option<String>,
    /// Episode grouping (e.g. `aegis-ontology`), when the tx carried one.
    pub group_id: Option<String>,
    /// The graph transaction this event was committed with.
    pub tx_id: i64,
    /// Type-specific payload, stored as a JSON object string.
    pub payload: String,
}

impl EventRow {
    /// The versioned wire form: the v1 envelope with the payload inlined.
    pub fn to_json(&self) -> serde_json::Value {
        let payload: serde_json::Value =
            serde_json::from_str(&self.payload).unwrap_or_else(|_| json!({}));
        json!({
            "schema": EVENT_SCHEMA,
            "offset": self.offset,
            "type": self.event_type,
            "ts": self.ts,
            "subject": self.subject,
            "group_id": self.group_id,
            "tx_id": self.tx_id,
            "payload": payload,
        })
    }
}

impl Store {
    /// Append this transaction's semantic events. MUST be called inside the
    /// `quipu_transact` savepoint (it is — from `stage_and_guard`), so the
    /// events commit or roll back atomically with the facts they describe.
    ///
    /// `asserts` / `retracts` are the datums ACTUALLY written (idempotent
    /// no-op assertions are already filtered out by the caller, so a re-ingest
    /// of identical content emits nothing — matching the write's own no-op).
    pub(crate) fn emit_events(
        &self,
        asserts: &[&Datum],
        retracts: &[&Datum],
        tx_id: i64,
        timestamp: &str,
        source: Option<&str>,
        graph: i64,
    ) -> Result<()> {
        // Take the advisory queue FIRST so it never leaks into a later tx —
        // even when this tx emits no semantic events (early returns below).
        let pending = std::mem::take(&mut *self.pending_write_events.borrow_mut());

        // ROOT-graph commits only (see module docs). Advisory events follow the
        // same rule: an overlay write discards them (compose-only staging).
        if graph != 0 || (asserts.is_empty() && retracts.is_empty() && pending.is_empty()) {
            return Ok(());
        }

        let rdf_type_id: Option<i64> = self.lookup(RDF_TYPE)?;

        // Term-id -> IRI cache so a large tx does not re-query `terms` per datum.
        let mut iri_cache: HashMap<i64, String> = HashMap::new();
        let mut resolve_cached = |store: &Store, id: i64| -> Result<String> {
            if let Some(iri) = iri_cache.get(&id) {
                return Ok(iri.clone());
            }
            let iri = store.resolve(id)?;
            iri_cache.insert(id, iri.clone());
            Ok(iri)
        };

        // The tx-wide group_id: an asserted literal on a `…groupId` predicate
        // (the episode writer's aegis:groupId). Namespace-suffix match because
        // the store layer does not know the server's base_ns.
        let mut group_id: Option<String> = None;
        for d in asserts {
            if let Value::Str(s) = &d.value {
                let a_iri = resolve_cached(self, d.attribute)?;
                if a_iri.ends_with("/groupId") || a_iri.ends_with("#groupId") {
                    group_id = Some(s.clone());
                    break;
                }
            }
        }

        let insert_event = |store: &Store,
                            event_type: &str,
                            subject: Option<&str>,
                            payload: &serde_json::Value|
         -> Result<i64> {
            store.conn.execute(
                "INSERT INTO events (type, ts, subject, group_id, tx_id, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event_type,
                    timestamp,
                    subject,
                    group_id.as_deref(),
                    tx_id,
                    payload.to_string()
                ],
            )?;
            Ok(store.conn.last_insert_rowid())
        };

        // `e` had any fact from an EARLIER transaction. This tx's own staged
        // rows all carry tx == tx_id, so `tx < tx_id` cleanly excludes them.
        let mut preexisting_cache: HashMap<i64, bool> = HashMap::new();
        let mut preexisting = |store: &Store, e: i64| -> Result<bool> {
            if let Some(&p) = preexisting_cache.get(&e) {
                return Ok(p);
            }
            let p: bool = store
                .conn
                .query_row(
                    "SELECT 1 FROM facts WHERE e = ?1 AND tx < ?2 LIMIT 1",
                    params![e, tx_id],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            preexisting_cache.insert(e, p);
            Ok(p)
        };

        // A schema term is NEW if absent from schema_terms. The insert happens
        // in the same savepoint, so a second sighting WITHIN this tx already
        // sees the row and stays silent — once means once.
        let schema_term_is_new = |store: &Store, term: &str, kind: &str| -> Result<bool> {
            let seen: bool = store
                .conn
                .query_row(
                    "SELECT 1 FROM schema_terms WHERE term = ?1 AND kind = ?2",
                    params![term, kind],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            Ok(!seen)
        };
        let record_schema_term =
            |store: &Store, term: &str, kind: &str, first_offset: i64| -> Result<()> {
                store.conn.execute(
                    "INSERT OR IGNORE INTO schema_terms (term, kind, first_offset) \
                     VALUES (?1, ?2, ?3)",
                    params![term, kind, first_offset],
                )?;
                Ok(())
            };

        // 1. episode.ingested — first, so a consumer keying on it reads the
        //    detail events that follow under the same tx_id.
        if let Some(name) = source.and_then(|s| s.strip_prefix("episode:")) {
            // The episode activity node is the entity carrying contentHash.
            let mut ep_subject: Option<String> = None;
            for d in asserts {
                let a_iri = resolve_cached(self, d.attribute)?;
                if a_iri.ends_with("contentHash") {
                    ep_subject = Some(resolve_cached(self, d.entity)?);
                    break;
                }
            }
            let payload = json!({
                "name": name,
                "facts": asserts.len() + retracts.len(),
            });
            insert_event(self, "episode.ingested", ep_subject.as_deref(), &payload)?;
        }

        // 2. Schema-level events: type.new / predicate.new, first sight only.
        for d in asserts {
            let a_iri = resolve_cached(self, d.attribute)?;
            if Some(d.attribute) == rdf_type_id {
                if let Value::Ref(t) = &d.value {
                    let type_iri = resolve_cached(self, *t)?;
                    if schema_term_is_new(self, &type_iri, "type")? {
                        let payload = json!({ "type_iri": type_iri });
                        let off = insert_event(self, "type.new", Some(&type_iri), &payload)?;
                        record_schema_term(self, &type_iri, "type", off)?;
                    }
                }
            } else if schema_term_is_new(self, &a_iri, "predicate")? {
                let payload = json!({ "predicate_iri": a_iri });
                let off = insert_event(self, "predicate.new", Some(&a_iri), &payload)?;
                record_schema_term(self, &a_iri, "predicate", off)?;
            }
        }

        // 3. entity.added / entity.updated — once per distinct touched entity,
        //    in first-touch order.
        let mut seen_entities: HashSet<i64> = HashSet::new();
        for d in asserts.iter().chain(retracts.iter()) {
            if !seen_entities.insert(d.entity) {
                continue;
            }
            let subject = resolve_cached(self, d.entity)?;
            let event_type = if preexisting(self, d.entity)? {
                "entity.updated"
            } else {
                "entity.added"
            };
            insert_event(self, event_type, Some(&subject), &json!({}))?;
        }

        // 4. edge.added / edge.retracted — every Ref-valued fact except the
        //    rdf:type plumbing (type membership is type.new/entity.* territory).
        for (datums, event_type) in [(asserts, "edge.added"), (retracts, "edge.retracted")] {
            for d in datums {
                if Some(d.attribute) == rdf_type_id {
                    continue;
                }
                let Value::Ref(obj) = &d.value else { continue };
                let subject = resolve_cached(self, d.entity)?;
                let predicate = resolve_cached(self, d.attribute)?;
                let object = resolve_cached(self, *obj)?;
                // subject_preexisting is the specced filter bit (P1 spec). object_preexisting is its additive mirror: an edge
                // ONTO an existing entity is just as much "a new relationship
                // on an existing entity" as one FROM it, and without this bit
                // that half of the set is unfilterable (found in live E2E — a
                // new node's edge to a preexisting node read as all-new).
                let payload = json!({
                    "subject": subject,
                    "predicate": predicate,
                    "object": object,
                    "subject_preexisting": preexisting(self, d.entity)?,
                    "object_preexisting": preexisting(self, *obj)?,
                });
                insert_event(self, event_type, Some(&subject), &payload)?;
            }
        }

        // Event P3: advisory events observed during pre-write validation
        // (`quipu:onViolation "emit"` shapes). Appended INSIDE the savepoint,
        // after the semantic events, carrying this tx's id/timestamp/group —
        // a rolled-back write takes its violation observations with it.
        for ev in &pending {
            insert_event(self, &ev.event_type, ev.subject.as_deref(), &ev.payload)?;
        }

        Ok(())
    }

    // -- Refusals (camayoc-0d3) -------------------------------------------

    /// Stash the refusal a gate just decided, so the write's error path can
    /// record it AFTER the `quipu_transact` savepoint has rolled back — an
    /// event inserted inside the savepoint would be rolled back with it.
    /// Returns `err` unchanged so gate call sites can use it in `map_err`.
    pub(crate) fn stash_refusal(
        &mut self,
        gate: &'static str,
        err: crate::error::Error,
        refused_datums: usize,
    ) -> crate::error::Error {
        self.pending_refusal = Some(PendingRefusal {
            gate,
            reason: err.to_string(),
            refused_datums,
        });
        err
    }

    /// Durably record that a write was REFUSED, as a [`REFUSAL_EVENT`] on the
    /// audit spine (camayoc-0d3).
    ///
    /// Contract:
    /// - MUST be called with no open savepoint that is about to roll back
    ///   (i.e. after `transact_to_graph`'s rollback, or on pre-write gates
    ///   that run before any savepoint opens).
    /// - NEVER fails: a refusal that cannot be recorded must not mask the
    ///   original refusal error, so insert failures are swallowed.
    /// - Refusals under [`Store::speculate`] are SKIPPED: the whole
    ///   speculation rolls back by design, so a hypothetical write's refusal
    ///   is not a real refusal. Speculation is detected by the
    ///   actor/source `"speculate"` that `speculate()` threads into its inner
    ///   transact. (Nested writes inside a speculation are additionally
    ///   contained by the outer `speculate` savepoint, which discards any
    ///   event they insert.)
    /// - Stores identifying METADATA only — gate, destination graph, actor,
    ///   source, terse reason, datum count — never the refused datum bodies.
    pub(crate) fn record_refusal(
        &self,
        refusal: &PendingRefusal,
        graph_iri: &str,
        actor: Option<&str>,
        source: Option<&str>,
        timestamp: &str,
    ) {
        if actor == Some("speculate") || source == Some("speculate") {
            return;
        }
        let reason: String = refusal.reason.chars().take(REFUSAL_REASON_MAX).collect();
        // Watermark, like `emit_registry_event`: the refused write's own tx
        // row was rolled back, so the event carries the last COMMITTED tx.
        let tx = self.latest_tx_id().unwrap_or(0);
        let payload = json!({
            "gate": refusal.gate,
            "graph": graph_iri,
            "actor": actor,
            "source": source,
            "reason": reason,
            "refused_datums": refusal.refused_datums,
        });
        let _ = self.conn.execute(
            "INSERT INTO events (type, ts, subject, group_id, tx_id, payload) \
             VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
            params![REFUSAL_EVENT, timestamp, graph_iri, tx, payload.to_string()],
        );
    }

    /// The IRI of graph `g`, for refusal payloads. ROOT gets its stable IRI;
    /// an unresolvable id degrades to `g:<id>` rather than failing the
    /// (must-not-fail) recording path.
    pub(crate) fn graph_iri_of(&self, graph: i64) -> String {
        if graph == crate::schema::ROOT_GRAPH {
            crate::schema::ROOT_GRAPH_IRI.to_string()
        } else {
            self.resolve(graph).unwrap_or_else(|_| format!("g:{graph}"))
        }
    }

    /// Count [`REFUSAL_EVENT`]s by gate — the incident-rate denominator
    /// (camayoc-0d3). Returns `(gate, count)` pairs, sorted by gate.
    pub fn refusals_by_gate(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(json_extract(payload, '$.gate'), '?') AS gate, COUNT(*) \
             FROM events WHERE type = ?1 GROUP BY gate ORDER BY gate",
        )?;
        let rows = stmt.query_map(params![REFUSAL_EVENT], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    // -- Retention --------------------------------------------------------

    /// Prune the event log (quipu-9z9) without breaking the replay contract.
    ///
    /// Deletes events whose `ts` is strictly older than `older_than`
    /// (ISO-8601) AND that every registered consumer has already committed
    /// past: with any rows in `consumers`, only offsets at or below
    /// `MIN(committed_offset)` are eligible — an event a consumer has not yet
    /// consumed is retained no matter how old, so a committed cursor never
    /// points at a hole and the reactor-down-6wk guarantee survives retention
    /// (the lagging consumer's backlog just stays on disk). With NO
    /// registered consumers there is no cursor to honour and age alone
    /// decides — the browser-pack case the design calls pure overhead.
    ///
    /// Offsets are `AUTOINCREMENT`, so pruning never causes offset reuse.
    /// What retention DOES give up is genesis replay: a consumer registering
    /// after a prune replays from the retained prefix, not from the
    /// beginning. A deployment that needs full-history replay for
    /// yet-unknown consumers must not enable retention.
    ///
    /// Nothing calls this by default — retention is opt-in via
    /// `[quipu.events] retention_days` (see [`crate::config::EventsConfig`])
    /// or a deliberate caller. Returns the number of events deleted.
    pub fn prune_events(&self, older_than: &str) -> Result<u64> {
        let safe: i64 = self.conn.query_row(
            "SELECT COALESCE(MIN(committed_offset), ?1) FROM consumers",
            params![i64::MAX],
            |r| r.get(0),
        )?;
        let deleted = self.conn.execute(
            "DELETE FROM events WHERE ts < ?1 AND \"offset\" <= ?2",
            params![older_than, safe],
        )?;
        Ok(deleted as u64)
    }

    // -- Pull API ---------------------------------------------------------

    /// Events with offset strictly AFTER `since`, in offset order, capped at
    /// `limit`, optionally filtered by type set and/or `group_id`.
    pub fn events_after(
        &self,
        since: i64,
        limit: usize,
        types: Option<&[String]>,
        group: Option<&str>,
    ) -> Result<Vec<EventRow>> {
        // Filters are dynamic; build the WHERE clause with positional params.
        let mut sql = String::from(
            "SELECT \"offset\", type, ts, subject, group_id, tx_id, payload \
             FROM events WHERE \"offset\" > ?1",
        );
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(since)];
        if let Some(ts) = types
            && !ts.is_empty()
        {
            let placeholders: Vec<String> = ts
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", bind.len() + i + 1))
                .collect();
            sql.push_str(&format!(" AND type IN ({})", placeholders.join(",")));
            for t in ts {
                bind.push(Box::new(t.clone()));
            }
        }
        if let Some(g) = group {
            sql.push_str(&format!(" AND group_id = ?{}", bind.len() + 1));
            bind.push(Box::new(g.to_string()));
        }
        sql.push_str(&format!(" ORDER BY \"offset\" LIMIT ?{}", bind.len() + 1));
        bind.push(Box::new(limit as i64));

        let mut stmt = self.conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::ToSql> = bind.iter().map(AsRef::as_ref).collect();
        let rows = stmt.query_map(params_ref.as_slice(), |r| {
            Ok(EventRow {
                offset: r.get(0)?,
                event_type: r.get(1)?,
                ts: r.get(2)?,
                subject: r.get(3)?,
                group_id: r.get(4)?,
                tx_id: r.get(5)?,
                payload: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Highest offset currently in the log (0 when empty). With `events_after`
    /// this gives a consumer its lag without a second scan.
    pub fn latest_event_offset(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(\"offset\"), 0) FROM events", [], |r| {
                r.get(0)
            })?)
    }

    /// The consumer's durable committed offset (0 = never committed — a fresh
    /// consumer replays from the beginning, per the replay-from-zero decision).
    pub fn consumer_committed(&self, consumer_id: &str) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT committed_offset FROM consumers WHERE consumer_id = ?1",
                params![consumer_id],
                |r| r.get(0),
            )
            .unwrap_or(0))
    }

    /// Durably commit a consumer's cursor. Any offset >= 0 is accepted —
    /// including one LOWER than the current cursor, which is the explicit
    /// replay knob (at-least-once delivery; consumers dedup by offset).
    pub fn commit_consumer(&self, consumer_id: &str, offset: i64, now: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO consumers (consumer_id, committed_offset, updated_at) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(consumer_id) DO UPDATE SET \
                 committed_offset = excluded.committed_offset, \
                 updated_at = excluded.updated_at",
            params![consumer_id, offset, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
