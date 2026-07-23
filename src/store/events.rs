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
        // ROOT-graph commits only (see module docs).
        if graph != 0 || (asserts.is_empty() && retracts.is_empty()) {
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

        Ok(())
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
mod tests {
    //! Acceptance tests for the P1 event log (event-log P1). Each test maps to a
    //! bead acceptance line; none assume — every claim is asserted against the
    //! store.

    use super::*;
    use crate::episode::{Edge, Episode, Node, ingest_episode};
    use crate::namespace::DEFAULT_BASE_NS;

    fn ep(name: &str, nodes: Vec<Node>, edges: Vec<Edge>) -> Episode {
        Episode {
            name: name.into(),
            episode_body: Some("test body".into()),
            source: Some("test".into()),
            group_id: Some("aegis-ontology".into()),
            nodes,
            edges,
            graph: None,
            shapes: None,
        }
    }

    fn node(name: &str, ty: &str) -> Node {
        Node {
            name: name.into(),
            node_type: Some(ty.into()),
            description: Some(format!("{name} description")),
            properties: None,
        }
    }

    fn edge(s: &str, rel: &str, t: &str) -> Edge {
        Edge {
            source: s.into(),
            target: t.into(),
            relation: rel.into(),
            confidence: None,
        }
    }

    fn all_events(store: &Store) -> Vec<EventRow> {
        store.events_after(0, 10_000, None, None).unwrap()
    }

    /// ACCEPTANCE: ingest -> episode.ingested + one edge.added per edge,
    /// offsets strictly monotonic, tx_id matches the episode's tx.
    #[test]
    fn ingest_emits_episode_and_edges_in_commit_order() {
        let mut store = Store::open_in_memory().unwrap();
        let e = ep(
            "ev-test-1",
            vec![node("svc-a", "Service"), node("host-b", "Host")],
            vec![edge("svc-a", "runs_on", "host-b")],
        );
        let (tx_id, count) =
            ingest_episode(&mut store, &e, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
        assert!(tx_id > 0 && count > 0);

        let events = all_events(&store);
        assert!(!events.is_empty(), "ingest must emit events");

        // Offsets strictly monotonic, all stamped with the episode's tx.
        for w in events.windows(2) {
            assert!(
                w[1].offset > w[0].offset,
                "offsets must be strictly increasing"
            );
        }
        for ev in &events {
            assert_eq!(
                ev.tx_id, tx_id,
                "{}: tx_id must match the ingest tx",
                ev.event_type
            );
            assert_eq!(ev.group_id.as_deref(), Some("aegis-ontology"));
        }

        let of = |t: &str| events.iter().filter(|e| e.event_type == t).count();
        assert_eq!(of("episode.ingested"), 1);
        // Exactly one edge in the episode; wasGeneratedBy provenance links are
        // ALSO Ref-valued edges, so require >= and assert the semantic one.
        assert!(of("edge.added") >= 1);
        let runs_on = events
            .iter()
            .find(|e| e.event_type == "edge.added" && e.payload.contains("runs_on"));
        assert!(
            runs_on.is_some(),
            "the episode's runs_on edge must be an edge.added event"
        );

        // Brand-new store: both node types announced exactly once each.
        assert_eq!(
            of("type.new") >= 2,
            true,
            "Service + Host (+ episode activity type)"
        );
        assert!(of("predicate.new") > 0);
        assert!(of("entity.added") > 0);
    }

    /// ACCEPTANCE (P1 spec): a brand-new type+predicate emits
    /// type.new AND predicate.new ONCE; re-ingesting the same type emits NO
    /// duplicate type.new; an edge on a PRE-EXISTING entity carries
    /// subject_preexisting=true.
    #[test]
    fn schema_events_fire_once_and_preexisting_is_flagged() {
        let mut store = Store::open_in_memory().unwrap();
        let e1 = ep("ev-first", vec![node("svc-a", "Service")], vec![]);
        ingest_episode(&mut store, &e1, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();

        let type_new_1 = all_events(&store)
            .iter()
            .filter(|e| e.event_type == "type.new" && e.payload.contains("Service"))
            .count();
        assert_eq!(type_new_1, 1, "first sight of Service announces once");

        // Second episode: same type (no dup), a NEW predicate, an edge whose
        // subject svc-a already exists.
        let mark = store.latest_event_offset().unwrap();
        let e2 = ep(
            "ev-second",
            vec![node("svc-b", "Service"), node("host-x", "Host")],
            vec![edge("svc-a", "wired_to", "host-x")],
        );
        ingest_episode(&mut store, &e2, "2026-07-23T00:10:00Z", DEFAULT_BASE_NS).unwrap();

        let new_events = store.events_after(mark, 10_000, None, None).unwrap();
        let service_dups = new_events
            .iter()
            .filter(|e| e.event_type == "type.new" && e.payload.contains("Service"))
            .count();
        assert_eq!(
            service_dups, 0,
            "re-ingesting a known type must NOT re-announce it"
        );
        assert_eq!(
            new_events
                .iter()
                .filter(|e| e.event_type == "type.new" && e.payload.contains("Host"))
                .count(),
            1,
            "the genuinely new Host type announces once"
        );
        assert_eq!(
            new_events
                .iter()
                .filter(|e| e.event_type == "predicate.new" && e.payload.contains("wired_to"))
                .count(),
            1,
            "the new predicate announces once"
        );

        let wired = new_events
            .iter()
            .find(|e| e.event_type == "edge.added" && e.payload.contains("wired_to"))
            .expect("wired_to edge event");
        let payload: serde_json::Value = serde_json::from_str(&wired.payload).unwrap();
        assert_eq!(
            payload["subject_preexisting"], true,
            "svc-a existed before this tx — the Stiwi filter bit must be set"
        );
        assert_eq!(
            payload["object_preexisting"], false,
            "host-x is new in this tx — the mirror bit must be clear"
        );

        // And svc-a is entity.updated (existing), svc-b entity.added (new).
        let has = |t: &str, frag: &str| {
            new_events
                .iter()
                .any(|e| e.event_type == t && e.subject.as_deref().unwrap_or("").contains(frag))
        };
        assert!(has("entity.updated", "svc-a"));
        assert!(has("entity.added", "svc-b"));
    }

    /// ACCEPTANCE: identical re-ingest is a no-op write and emits NOTHING —
    /// the log records commits, not attempts.
    #[test]
    fn idempotent_reingest_emits_no_events() {
        let mut store = Store::open_in_memory().unwrap();
        let e = ep("ev-idem", vec![node("svc-a", "Service")], vec![]);
        ingest_episode(&mut store, &e, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
        let before = store.latest_event_offset().unwrap();
        let (tx2, n2) =
            ingest_episode(&mut store, &e, "2026-07-23T00:20:00Z", DEFAULT_BASE_NS).unwrap();
        assert_eq!((tx2, n2), (crate::episode::NOOP_TX, 0));
        assert_eq!(
            store.latest_event_offset().unwrap(),
            before,
            "a no-op ingest must not append events"
        );
    }

    /// ACCEPTANCE: ?types= and ?group= filter; batches page in offset order
    /// with a usable cursor.
    #[test]
    fn pull_filters_and_pagination() {
        let mut store = Store::open_in_memory().unwrap();
        let mut e1 = ep("ev-g1", vec![node("a", "T1")], vec![]);
        e1.group_id = Some("group-one".into());
        let mut e2 = ep("ev-g2", vec![node("b", "T2")], vec![]);
        e2.group_id = Some("group-two".into());
        ingest_episode(&mut store, &e1, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
        ingest_episode(&mut store, &e2, "2026-07-23T00:01:00Z", DEFAULT_BASE_NS).unwrap();

        // types filter
        let only_types = store
            .events_after(0, 100, Some(&["type.new".into()]), None)
            .unwrap();
        assert!(!only_types.is_empty());
        assert!(only_types.iter().all(|e| e.event_type == "type.new"));

        // group filter
        let g2 = store.events_after(0, 100, None, Some("group-two")).unwrap();
        assert!(!g2.is_empty());
        assert!(
            g2.iter()
                .all(|e| e.group_id.as_deref() == Some("group-two"))
        );

        // pagination: limit 1 pages the full log in order via the cursor
        let all = all_events(&store);
        let mut cursor = 0;
        let mut paged = Vec::new();
        loop {
            let batch = store.events_after(cursor, 1, None, None).unwrap();
            match batch.first() {
                None => break,
                Some(ev) => {
                    cursor = ev.offset;
                    paged.push(ev.offset);
                }
            }
        }
        assert_eq!(
            paged,
            all.iter().map(|e| e.offset).collect::<Vec<_>>(),
            "limit-1 paging must walk the identical sequence"
        );
    }

    /// ACCEPTANCE: commit -> resume returns events strictly AFTER the durable
    /// cursor (the 'reactor down 6wk lost events' fix), and an unknown
    /// consumer starts from 0 (full replay).
    #[test]
    fn consumer_commit_and_resume() {
        let mut store = Store::open_in_memory().unwrap();
        ingest_episode(
            &mut store,
            &ep("ev-c1", vec![node("a", "T1")], vec![]),
            "2026-07-23T00:00:00Z",
            DEFAULT_BASE_NS,
        )
        .unwrap();
        let mid = store.latest_event_offset().unwrap();
        ingest_episode(
            &mut store,
            &ep("ev-c2", vec![node("b", "T2")], vec![]),
            "2026-07-23T00:01:00Z",
            DEFAULT_BASE_NS,
        )
        .unwrap();

        assert_eq!(
            store.consumer_committed("reactor").unwrap(),
            0,
            "fresh consumer replays from 0"
        );

        store
            .commit_consumer("reactor", mid, "2026-07-23T00:02:00Z")
            .unwrap();
        assert_eq!(
            store.consumer_committed("reactor").unwrap(),
            mid,
            "cursor is durable"
        );

        let resumed = store
            .events_after(
                store.consumer_committed("reactor").unwrap(),
                100,
                None,
                None,
            )
            .unwrap();
        assert!(!resumed.is_empty());
        assert!(
            resumed.iter().all(|e| e.offset > mid),
            "resume returns only events AFTER the committed offset"
        );
        // Everything resumed belongs to the second episode's tx.
        let first_tx = resumed[0].tx_id;
        assert!(resumed.iter().all(|e| e.tx_id == first_tx));
    }

    /// ACCEPTANCE: the schema is additive — a store written by the pre-events
    /// code path (simulated: facts present, events absent) reopens fine, keeps
    /// its facts, and starts emitting from offset 1 without migration.
    #[test]
    fn additive_schema_reopen() {
        let dir = std::env::temp_dir().join(format!("quipu-ev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("additive.db");
        let path_s = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);

        {
            let mut store = Store::open(path_s).unwrap();
            ingest_episode(
                &mut store,
                &ep("ev-persist", vec![node("a", "T1")], vec![]),
                "2026-07-23T00:00:00Z",
                DEFAULT_BASE_NS,
            )
            .unwrap();
        }
        {
            // Reopen: facts intact, event log intact, both APIs answer.
            let store = Store::open(path_s).unwrap();
            assert!(!store.current_facts().unwrap().is_empty());
            let events = all_events(&store);
            assert!(!events.is_empty(), "events survive reopen (durable log)");
            assert!(store.latest_event_offset().unwrap() > 0);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// The retract paths (`retract_entity` / `retract_triples`) route through
    /// `transact`, so retractions EMIT — this was mis-reported as a gap when the
    /// event log first landed, and this test is the correction: claim by
    /// mechanism, not by memory of the code.
    #[test]
    fn retraction_emits_edge_retracted() {
        let mut store = Store::open_in_memory().unwrap();
        let e = ep(
            "ev-retract",
            vec![node("svc-a", "Service"), node("host-b", "Host")],
            vec![edge("svc-a", "runs_on", "host-b")],
        );
        ingest_episode(&mut store, &e, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
        let mark = store.latest_event_offset().unwrap();

        // Retract everything on svc-a (label, type, description, the edge).
        let svc_a = store
            .lookup(&format!("{DEFAULT_BASE_NS}svc-a"))
            .unwrap()
            .expect("svc-a interned");
        let (tx, n) = store
            .retract_entity(svc_a, None, "2026-07-23T01:00:00Z", None)
            .unwrap();
        assert!(tx > 0 && n > 0, "retraction must be a real tx");

        let evs = store.events_after(mark, 1000, None, None).unwrap();
        assert!(!evs.is_empty(), "retraction must emit events");
        assert!(evs.iter().all(|e| e.tx_id == tx));
        let retracted: Vec<_> = evs
            .iter()
            .filter(|e| e.event_type == "edge.retracted")
            .collect();
        assert!(
            retracted.iter().any(|e| e.payload.contains("runs_on")),
            "the runs_on edge retraction must be an edge.retracted event; got {retracted:?}"
        );
        // svc-a existed before the retraction tx -> entity.updated, not .added.
        assert!(evs.iter().any(
            |e| e.event_type == "entity.updated" && e.subject.as_deref().unwrap_or("").contains("svc-a")
        ));
        // And no phantom episode.ingested from a non-episode source.
        assert!(!evs.iter().any(|e| e.event_type == "episode.ingested"));
    }
}
