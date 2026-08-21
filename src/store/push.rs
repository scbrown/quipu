//! Event push delivery (event-log P2): subscription registry + webhook push.
//!
//! DESIGN (y1lb §4c/§7, decided — not relitigated here):
//!   - at-least-once: a subscription's cursor advances ONLY after its webhook
//!     answered 2xx for a delivery; anything else re-delivers next tick.
//!     Consumers dedup by `offset`, which every delivered event carries.
//!   - no auth toward internal LAN targets.
//!   - cursors reuse the P1 `consumers` table under `sub:<consumer_id>` — pull
//!     and push share one offset semantics and one durability story.
//!
//! TESTABILITY IS THE ARCHITECTURE: [`deliver_tick`] takes the HTTP poster as
//! a function, so the entire acceptance surface (offset order, filtering,
//! kill/revive replay, batch cutting, no-advance-on-failure) runs
//! deterministically in unit tests with no network and no clock sleeps. The
//! server's background task is a thin loop around it with the real poster.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store::Store;

/// A registered push subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Stable registry id.
    pub id: i64,
    /// The consumer identity; the delivery cursor is `sub:<consumer_id>`.
    pub consumer_id: String,
    /// Event types to deliver; `None` = all types.
    pub types: Option<Vec<String>>,
    /// `realtime` or `batch`.
    pub mode: String,
    /// Where matching events are `POST`ed.
    pub webhook_url: String,
    /// Batch mode: deliver when this many events are pending…
    pub batch_size: usize,
    /// …or when the oldest pending event is at least this old (seconds).
    pub batch_window_s: i64,
}

impl Store {
    /// Register a subscription. REFUSES a `sparql_ask` filter loudly — the
    /// field is reserved but nothing evaluates it yet, and a filter that is
    /// stored-but-ignored would deliver events the subscriber asked to
    /// exclude (the silent-enforcement-gap class).
    #[allow(clippy::too_many_arguments)] // registry insert mirrors the table's columns
    pub fn subscription_create(
        &self,
        consumer_id: &str,
        types: Option<&[String]>,
        sparql_ask: Option<&str>,
        mode: &str,
        webhook_url: &str,
        batch_size: usize,
        batch_window_s: i64,
        now: &str,
    ) -> Result<i64> {
        if sparql_ask.is_some_and(|s| !s.trim().is_empty()) {
            return Err(Error::InvalidValue(
                "sparql_ask filters are not evaluated yet; refusing to store one \
                 that would be silently ignored (types-filter is supported)"
                    .into(),
            ));
        }
        if !matches!(mode, "realtime" | "batch") {
            return Err(Error::InvalidValue(format!(
                "mode must be 'realtime' or 'batch', got '{mode}'"
            )));
        }
        if webhook_url.trim().is_empty() {
            return Err(Error::InvalidValue("webhook_url is required".into()));
        }
        let types_json = match types {
            Some(t) if !t.is_empty() => Some(
                serde_json::to_string(t)
                    .map_err(|e| Error::InvalidValue(format!("types encode: {e}")))?,
            ),
            _ => None,
        };
        self.conn.execute(
            "INSERT INTO subscriptions \
             (consumer_id, types, sparql_ask, mode, webhook_url, batch_size, batch_window_s, created_at) \
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![consumer_id, types_json, mode, webhook_url,
                              batch_size as i64, batch_window_s, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// All registered subscriptions.
    pub fn subscription_list(&self) -> Result<Vec<Subscription>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, consumer_id, types, mode, webhook_url, batch_size, batch_window_s \
             FROM subscriptions ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let types_json: Option<String> = r.get(2)?;
            Ok(Subscription {
                id: r.get(0)?,
                consumer_id: r.get(1)?,
                types: types_json.and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()),
                mode: r.get(3)?,
                webhook_url: r.get(4)?,
                batch_size: usize::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
                batch_window_s: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Delete a subscription by consumer id. Returns whether one existed.
    pub fn subscription_delete(&self, consumer_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM subscriptions WHERE consumer_id = ?1",
            [consumer_id],
        )?;
        Ok(n > 0)
    }
}

/// One delivery attempt's outcome, for the tick report.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    /// `POST`ed and 2xx-acknowledged `n` events; cursor advanced to `to`.
    Delivered { n: usize, to: i64 },
    /// Nothing matching pending (or batch not yet due).
    Nothing,
    /// POST failed or non-2xx; cursor NOT advanced — re-delivered next tick.
    Failed,
}

/// Run one delivery pass over every subscription. `now_epoch_s` is threaded
/// (never read from a clock here) and `post` is the HTTP poster — both
/// injected so tests are deterministic. Returns per-subscription outcomes.
///
/// Batch semantics: deliver when pending >= `batch_size`, OR when the oldest
/// pending event has waited >= `batch_window_s` (tracked via the event `ts`
/// against `now_epoch_s`); otherwise hold. Realtime delivers whatever is
/// pending each tick. Both cap a single POST at 500 events; the cursor
/// advance makes the remainder next tick's work.
pub fn deliver_tick(
    store: &Store,
    now_epoch_s: i64,
    post: &mut dyn FnMut(&str, &serde_json::Value) -> bool,
) -> Result<Vec<(String, Delivery)>> {
    let mut out = Vec::new();
    for sub in store.subscription_list()? {
        let cursor_id = format!("sub:{}", sub.consumer_id);
        let since = store.consumer_committed(&cursor_id)?;
        let events = store.events_after(since, 500, sub.types.as_deref(), None)?;
        if events.is_empty() {
            out.push((sub.consumer_id.clone(), Delivery::Nothing));
            continue;
        }
        if sub.mode == "batch" && events.len() < sub.batch_size {
            // Deliver early only if the oldest pending event has aged out.
            let oldest_age = events
                .first()
                .and_then(|e| ts_epoch(&e.ts))
                .map(|t| now_epoch_s - t);
            if oldest_age.is_none_or(|age| age < sub.batch_window_s) {
                out.push((sub.consumer_id.clone(), Delivery::Nothing));
                continue;
            }
        }
        let last = events.last().map_or(since, |e| e.offset);
        let payload = serde_json::json!({
            "subscription": sub.consumer_id,
            "events": events.iter().map(super::events::EventRow::to_json).collect::<Vec<_>>(),
        });
        if post(&sub.webhook_url, &payload) {
            // Advance ONLY on acknowledged delivery: at-least-once.
            let now_iso = crate::time::now_iso();
            store.commit_consumer(&cursor_id, last, &now_iso)?;
            out.push((
                sub.consumer_id.clone(),
                Delivery::Delivered {
                    n: events.len(),
                    to: last,
                },
            ));
        } else {
            out.push((sub.consumer_id.clone(), Delivery::Failed));
        }
    }
    Ok(out)
}

/// Parse an ISO-8601 `YYYY-MM-DDTHH:MM:SS…` timestamp to epoch seconds,
/// tolerating the suffix. Returns None on anything unparseable (a malformed
/// ts then never triggers age-based batch flush — size still does).
fn ts_epoch(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let (y, mo, d) = (num(&ts[0..4])?, num(&ts[5..7])?, num(&ts[8..10])?);
    let (h, mi, s) = (num(&ts[11..13])?, num(&ts[14..16])?, num(&ts[17..19])?);
    // Days since epoch, civil-from-days inverse (Howard Hinnant's algorithm).
    let y_adj = if mo <= 2 { y - 1 } else { y };
    let era = y_adj.div_euclid(400);
    let yoe = y_adj - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::{Episode, Node, ingest_episode};
    use crate::namespace::DEFAULT_BASE_NS;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn ingest(store: &mut Store, name: &str) {
        let ep = Episode {
            name: name.into(),
            episode_body: Some("b".into()),
            source: Some("t".into()),
            group_id: Some("g".into()),
            nodes: vec![Node {
                name: format!("{name}-node"),
                node_type: Some("Thing".into()),
                description: None,
                properties: None,
                distinct_from: Vec::new(),
            }],
            edges: vec![],
            graph: None,
            shapes: None,
            replace_snapshot: false,
        };
        ingest_episode(store, &ep, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
    }

    /// ACCEPTANCE: matching events `POSTed` in offset order; cursor advances.
    #[test]
    fn realtime_delivers_in_offset_order() {
        let mut s = store();
        s.subscription_create(
            "c1",
            None,
            None,
            "realtime",
            "http://x/hook",
            50,
            30,
            "2026-07-23T00:00:00Z",
        )
        .unwrap();
        ingest(&mut s, "e1");
        ingest(&mut s, "e2");
        let mut seen: Vec<i64> = Vec::new();
        let mut post = |_url: &str, body: &serde_json::Value| {
            for e in body["events"].as_array().unwrap() {
                seen.push(e["offset"].as_i64().unwrap());
            }
            true
        };
        let out = deliver_tick(&s, 0, &mut post).unwrap();
        assert!(matches!(out[0].1, Delivery::Delivered { .. }));
        assert!(!seen.is_empty());
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        assert_eq!(seen, sorted, "offset order");
        // Nothing new -> Nothing.
        let out = deliver_tick(&s, 0, &mut |_, _| true).unwrap();
        assert_eq!(out[0].1, Delivery::Nothing);
    }

    /// ACCEPTANCE: kill the receiver -> no cursor advance; revive -> the
    /// missed events are re-delivered from the cursor (replay).
    #[test]
    fn failed_delivery_replays_after_revival() {
        let mut s = store();
        s.subscription_create(
            "c1",
            None,
            None,
            "realtime",
            "http://x/hook",
            50,
            30,
            "2026-07-23T00:00:00Z",
        )
        .unwrap();
        ingest(&mut s, "e1");
        let out = deliver_tick(&s, 0, &mut |_, _| false).unwrap(); // receiver dead
        assert_eq!(out[0].1, Delivery::Failed);
        ingest(&mut s, "e2"); // more events while dead
        let mut n = 0usize;
        let out = deliver_tick(&s, 0, &mut |_, b| {
            n = b["events"].as_array().unwrap().len();
            true
        })
        .unwrap(); // revived
        match out[0].1 {
            Delivery::Delivered { n: dn, .. } => assert_eq!(dn, n),
            ref other => panic!("expected delivery, got {other:?}"),
        }
        assert!(n >= 2, "replay covers events from BOTH ingests, got {n}");
    }

    /// ACCEPTANCE: a non-matching event type is NOT delivered.
    #[test]
    fn type_filter_excludes_non_matching() {
        let mut s = store();
        s.subscription_create(
            "c1",
            Some(&["no.such.type".into()]),
            None,
            "realtime",
            "http://x/hook",
            50,
            30,
            "2026-07-23T00:00:00Z",
        )
        .unwrap();
        ingest(&mut s, "e1");
        let mut called = false;
        let out = deliver_tick(&s, 0, &mut |_, _| {
            called = true;
            true
        })
        .unwrap();
        assert_eq!(out[0].1, Delivery::Nothing);
        assert!(!called, "poster must not be called for non-matching events");
    }

    /// ACCEPTANCE: batch mode holds until size OR window; then delivers.
    #[test]
    fn batch_mode_holds_then_flushes() {
        let mut s = store();
        s.subscription_create(
            "c1",
            None,
            None,
            "batch",
            "http://x/hook",
            1000,
            60,
            "2026-07-23T00:00:00Z",
        )
        .unwrap();
        ingest(&mut s, "e1");
        let base = ts_epoch("2026-07-23T00:00:00Z").unwrap();
        // Under size, within window -> hold.
        let out = deliver_tick(&s, base + 10, &mut |_, _| true).unwrap();
        assert_eq!(
            out[0].1,
            Delivery::Nothing,
            "held: under size, inside window"
        );
        // Window elapsed -> flush.
        let out = deliver_tick(&s, base + 61, &mut |_, _| true).unwrap();
        assert!(
            matches!(out[0].1, Delivery::Delivered { .. }),
            "window flush"
        );
    }

    /// A `sparql_ask` filter is refused loudly, not stored-and-ignored.
    #[test]
    fn sparql_ask_refused_until_evaluated() {
        let s = store();
        let err = s.subscription_create(
            "c1",
            None,
            Some("ASK { ?s ?p ?o }"),
            "realtime",
            "http://x/hook",
            50,
            30,
            "2026-07-23T00:00:00Z",
        );
        assert!(err.is_err());
    }

    #[test]
    fn registry_crud_roundtrip() {
        let s = store();
        s.subscription_create(
            "c1",
            Some(&["episode.ingested".into()]),
            None,
            "realtime",
            "http://x/hook",
            50,
            30,
            "2026-07-23T00:00:00Z",
        )
        .unwrap();
        let subs = s.subscription_list().unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(
            subs[0].types.as_deref(),
            Some(&["episode.ingested".to_string()][..])
        );
        assert!(s.subscription_delete("c1").unwrap());
        assert!(
            !s.subscription_delete("c1").unwrap(),
            "second delete finds nothing"
        );
        assert!(s.subscription_list().unwrap().is_empty());
    }

    #[test]
    fn ts_epoch_parses_iso() {
        assert_eq!(ts_epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(ts_epoch("1970-01-02T00:00:01Z"), Some(86401));
        assert_eq!(ts_epoch("garbage"), None);
    }
}
