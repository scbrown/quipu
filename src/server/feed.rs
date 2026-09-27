//! Read-only change feeds use WAL readers; consumer commits keep the writer.

use super::{
    SharedStore,
    base::{AppError, blocking},
};
use axum::{
    extract::{Query, State},
    http::HeaderMap,
};
use serde_json::{Value as JsonValue, json};

/// Dropped before the store guard, including early errors and abandoned HTTP calls.
struct StoreTiming {
    client: String,
    endpoint: &'static str,
    wait: std::time::Duration,
    acquired: std::time::Instant,
}

impl StoreTiming {
    fn acquired(client: String, endpoint: &'static str, wait: std::time::Duration) -> Self {
        Self {
            client,
            endpoint,
            wait,
            acquired: std::time::Instant::now(),
        }
    }
}

impl Drop for StoreTiming {
    fn drop(&mut self) {
        quipu::metrics::metrics().observe_store_time(
            &self.client,
            self.endpoint,
            self.wait.as_secs_f64(),
            self.acquired.elapsed().as_secs_f64(),
        );
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct TransactionParams {
    since: Option<i64>,
    limit: Option<i64>,
}

/// Query parameters for the event-log pull API (event-log P1).
#[derive(serde::Deserialize)]
pub(crate) struct EventParams {
    /// Return events with offset STRICTLY AFTER this. Explicit `since` wins
    /// over `consumer` so a caller can inspect any window without moving (or
    /// consulting) its durable cursor.
    since: Option<i64>,
    limit: Option<i64>,
    /// Comma-separated event types (e.g. `edge.added,type.new`).
    types: Option<String>,
    /// Filter to a single `group_id` (episode grouping, e.g. `aegis-ontology`).
    group: Option<String>,
    /// Resume from this consumer's durable committed offset.
    consumer: Option<String>,
}

/// Query parameters for the change feed (quipu-2ae).
#[derive(serde::Deserialize)]
pub(crate) struct ChangeParams {
    /// Serve transactions STRICTLY AFTER this id (default 0 = from genesis).
    since: Option<i64>,
    /// Max transactions per page (whole transactions only).
    limit: Option<i64>,
    /// `new_values` (default) | `old_and_new_values` | `new_row`.
    capture: Option<String>,
    /// Graph IRI to scope to; absent spans all graphs.
    graph: Option<String>,
}

/// GET /changes — pull fact-level change records in commit order.
///
/// The page's `next_tx` is the cursor for the next call; `watermark_tx` /
/// `watermark_timestamp` distinguish an idle store from a broken feed. Per
/// entity, records arrive in commit order; across entities there is no
/// ordering promise. See `store::changes` for the full contract.
pub(crate) async fn changes_get(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    Query(p): Query<ChangeParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let client = super::query_usage::client(&headers);
    blocking(move || {
        // READ POOL, not the writer (aegis-4cfnck). Every call below is a
        // SELECT; on the writer mutex each poll of the feed serialised against
        // every write in the store, and a reactor polling it is the heaviest
        // steady reader it has.
        let waiting = std::time::Instant::now();
        let store = store.read();
        let _timing = StoreTiming::acquired(client, "/changes", waiting.elapsed());
        let capture = match p.capture.as_deref() {
            None => quipu::store::changes::Capture::NewValues,
            Some(name) => quipu::store::changes::Capture::parse(name).ok_or_else(|| {
                quipu::Error::InvalidValue(format!(
                    "capture must be new_values, old_and_new_values, or new_row; got {name:?}"
                ))
            })?,
        };
        let graph =
            match p.graph.as_deref() {
                None => None,
                Some(iri) => Some(store.lookup(iri)?.ok_or_else(|| {
                    quipu::Error::InvalidValue(format!("unknown graph IRI: {iri}"))
                })?),
            };
        let limit = usize::try_from(p.limit.unwrap_or(100).clamp(1, 10_000)).unwrap_or(100);
        let page = store.changes_after(p.since.unwrap_or(0), limit, capture, graph)?;
        Ok(axum::Json(page.to_json()))
    })
    .await
}

/// GET /events — pull a batch of graph-change events in offset order.
/// Response: `{events, next_offset, lag, committed_offset?}`; pass
/// `next_offset` back as `since` (or POST /events/commit it) to page forward.
pub(crate) async fn events_get(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    Query(p): Query<EventParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let client = super::query_usage::client(&headers);
    blocking(move || {
        // READ POOL (aegis-4cfnck). events_after and latest_event_offset are
        // two statements on a pooled reader, so a write committing between
        // them can make `lag` read slightly HIGH for one poll. `next_offset`,
        // the cursor, is unaffected, and lag is a gauge, not an ordering promise.
        let waiting = std::time::Instant::now();
        let store = store.read();
        let _timing = StoreTiming::acquired(client, "/events", waiting.elapsed());
        let committed: Option<i64> = match (&p.since, &p.consumer) {
            (None, Some(c)) => Some(store.consumer_committed(c)?),
            _ => None,
        };
        let since = p.since.unwrap_or_else(|| committed.unwrap_or(0));
        let limit = usize::try_from(p.limit.unwrap_or(100).clamp(1, 10_000)).unwrap_or(100);
        let types: Option<Vec<String>> = p.types.as_deref().map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        });
        let rows = store.events_after(since, limit, types.as_deref(), p.group.as_deref())?;
        // next_offset is the cursor for the NEXT call; when the batch is empty
        // it stays at `since` so polling is a fixpoint, not a rewind.
        let next_offset = rows.last().map_or(since, |r| r.offset);
        // Lag counts ALL events beyond the cursor, unfiltered — it answers
        // "how far behind the log am I", not "how many match my filter".
        let latest = store.latest_event_offset()?;
        let lag = (latest - next_offset).max(0);
        let events: Vec<JsonValue> = rows
            .iter()
            .map(quipu::store::events::EventRow::to_json)
            .collect();
        let mut body = json!({
            "events": events,
            "next_offset": next_offset,
            "lag": lag,
        });
        if let Some(c) = committed {
            body["committed_offset"] = json!(c);
        }
        Ok(axum::Json(body))
    })
    .await
}

/// POST /events/commit `{consumer_id, offset}` — durably record a consumer's
/// cursor. Any offset >= 0 is accepted, including a LOWER one (the explicit
/// replay knob; delivery is at-least-once and consumers dedup by offset).
pub(crate) async fn events_commit(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let consumer_id = input
            .get("consumer_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| quipu::Error::InvalidValue("consumer_id is required".into()))?
            .to_string();
        let offset = input
            .get("offset")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| quipu::Error::InvalidValue("offset (integer) is required".into()))?;
        if offset < 0 {
            return Err(quipu::Error::InvalidValue("offset must be >= 0".into()).into());
        }
        let store = store.lock();
        let now = quipu::time::now_iso();
        store.commit_consumer(&consumer_id, offset, &now)?;
        Ok(axum::Json(json!({
            "consumer_id": consumer_id,
            "committed_offset": offset,
        })))
    })
    .await
}

pub(crate) async fn transactions(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    Query(p): Query<TransactionParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let client = super::query_usage::client(&headers);
    blocking(move || {
        // READ POOL (aegis-4cfnck): list_transactions* is SELECT-only.
        let waiting = std::time::Instant::now();
        let store = store.read();
        let _timing = StoreTiming::acquired(client, "/transactions", waiting.elapsed());
        // Cursor for pollers (Shantytown's event subscription): `?since=<tx>`
        // returns only newer transactions so a watermarked poll is O(new), not
        // O(whole log). No params -> the full log, preserving prior behaviour.
        let txns = if p.since.is_none() && p.limit.is_none() {
            store.list_transactions()?
        } else {
            store.list_transactions_since(
                p.since.unwrap_or(0),
                p.limit.unwrap_or(1000).clamp(1, 10_000),
            )?
        };
        let entries: Vec<JsonValue> = txns
            .iter()
            .map(|t| {
                Ok(
                    json!({ "id": t.id, "timestamp": t.timestamp, "actor": t.actor,
                    "source": t.source, "authenticated": store.transaction_auth(t.id)? }),
                )
            })
            .collect::<quipu::Result<_>>()?;
        Ok(axum::Json(
            json!({ "transactions": entries, "count": entries.len() }),
        ))
    })
    .await
}
