//! Entity, history, event-log and semantic-web handlers: content negotiation
//! on /entity, the /events pull API, /transactions, and the Phase 4 semweb
//! endpoints (/spotlight, /fragments, /reconcile, /preview).

use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse},
};
use serde_json::{Value as JsonValue, json};

use quipu::semweb;

use super::SharedStore;
use super::base::{AppError, blocking};

pub(crate) async fn entity_history(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let iri = input
            .get("iri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| quipu::Error::InvalidValue("missing 'iri' parameter".into()))?;
        let store = store.lock();
        let eid = store
            .lookup(iri)?
            .ok_or_else(|| quipu::Error::InvalidValue(format!("entity not found: {iri}")))?;
        let entries: Vec<JsonValue> = store
            .entity_history(eid)?
            .iter()
            .map(|f| {
                let pred = store.resolve(f.attribute).unwrap_or_default();
                json!({ "op": if f.op == quipu::Op::Assert { "assert" } else { "retract" },
                    "predicate": pred, "value": quipu::value_to_json(&store, &f.value),
                    "valid_from": f.valid_from, "valid_to": f.valid_to, "tx": f.tx })
            })
            .collect();
        Ok(axum::Json(
            json!({ "iri": iri, "history": entries, "count": entries.len() }),
        ))
    })
    .await
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
    Query(p): Query<ChangeParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let store = store.lock();
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
    Query(p): Query<EventParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let store = store.lock();
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
    Query(p): Query<TransactionParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let store = store.lock();
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
                json!({ "id": t.id, "timestamp": t.timestamp, "actor": t.actor, "source": t.source })
            })
            .collect();
        Ok(axum::Json(
            json!({ "transactions": entries, "count": entries.len() }),
        ))
    })
    .await
}

pub(crate) async fn entity_conneg(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
    Query(params): Query<OutputParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    entity_response(
        store,
        semweb::decode_iri(&iri),
        headers,
        params.expanded.unwrap_or(false),
    )
    .await
}

#[derive(serde::Deserialize)]
pub(crate) struct EntityParams {
    pub(crate) iri: String,
    pub(crate) expanded: Option<bool>,
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct OutputParams {
    pub(crate) expanded: Option<bool>,
}

/// Query-form dereference endpoint for IRIs containing `/` and `#`.
pub(crate) async fn entity_query_conneg(
    State(store): State<SharedStore>,
    Query(params): Query<EntityParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    entity_response(store, params.iri, headers, params.expanded.unwrap_or(false)).await
}

async fn entity_response(
    store: SharedStore,
    iri: String,
    headers: HeaderMap,
    expanded: bool,
) -> Result<axum::response::Response, AppError> {
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html");
    let json_ld = accept.contains("application/ld+json") || accept.contains("application/json");
    let turtle = accept.contains("text/turtle") || accept.contains("application/x-turtle");
    if !json_ld && !turtle {
        return blocking(move || {
            Ok(Html(semweb::preview_card(&store.lock(), &iri)?).into_response())
        })
        .await;
    }
    blocking(move || {
        let store = store.lock();
        if json_ld {
            Ok(json_ld_response(semweb::entity_json_ld_mode(
                &store, &iri, expanded,
            )?))
        } else {
            Ok(turtle_response(semweb::entity_turtle(&store, &iri)?))
        }
    })
    .await
}

pub(crate) async fn entity_json(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
    Query(params): Query<OutputParams>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
        let j = semweb::entity_json_ld_mode(
            &store.lock(),
            &semweb::decode_iri(&iri),
            params.expanded.unwrap_or(false),
        )?;
        Ok(json_ld_response(j))
    })
    .await
}

pub(crate) async fn entity_turtle_suffix(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
        let t = semweb::entity_turtle(&store.lock(), &semweb::decode_iri(&iri))?;
        Ok(turtle_response(t))
    })
    .await
}

pub(crate) async fn entity_html(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<Html<String>, AppError> {
    blocking(move || {
        Ok(Html(semweb::preview_card(
            &store.lock(),
            &semweb::decode_iri(&iri),
        )?))
    })
    .await
}

pub(crate) fn json_ld_response(j: JsonValue) -> axum::response::Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/ld+json")],
        axum::Json(j),
    )
        .into_response()
}

pub(crate) fn turtle_response(t: Vec<u8>) -> axum::response::Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/turtle; charset=utf-8",
        )],
        t,
    )
        .into_response()
}

/// Generation-keyed cache of the labeled-entity list spotlight scans against.
///
/// The first deploy of the reader-starvation fix moved only the SCAN off the
/// store lock and still starved readers — measured: the expensive half is the
/// FETCH itself (full-label SPARQL + per-row IRI resolution, 2-3s at 11k+
/// entities), not the scan. So the fetch result is cached and keyed on
/// `Store::latest_tx_id()`: under a spotlight burst only the first call pays
/// the fetch; the rest hold one read-pool connection for one indexed MAX. Any write
/// moves the generation and invalidates naturally.
pub(crate) struct SpotlightCache {
    generation: i64,
    entities: Arc<Vec<semweb::LabeledEntity>>,
}

pub(crate) static SPOTLIGHT_CACHE: Mutex<Option<SpotlightCache>> = Mutex::new(None);

/// Longer than the measured healthy cold fill, but short enough that an
/// abandoned request cannot occupy a pooled reader indefinitely.
const SPOTLIGHT_FETCH_BUDGET_MS: u64 = 10_000;

pub(crate) async fn spotlight_handler(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let text = input
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| quipu::Error::InvalidValue("missing 'text' parameter".into()))?;
        let confidence = input
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.5);
        // Admit exactly one cache fill BEFORE taking a pooled connection.
        // Followers fail open with no annotations instead of waiting: a timed-
        // out client leaves spawn_blocking running, so a blocking mutex here
        // turns a request burst into a serialized queue of abandoned fills.
        // Acquiring a reader before admission would occupy every pooled reader
        // while followers waited, recreating starvation outside the writer.
        // The cold full-label fetch is read-only and must never take the writer:
        // at production size it takes 2-3s, longer than Bobbin's client timeout,
        // so abandoned requests otherwise amplify into a wedged store.
        let entities = {
            let mut cache = match SPOTLIGHT_CACHE.try_lock() {
                Ok(cache) => cache,
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Ok(axum::Json(semweb::spotlight_over(&[], text, confidence)));
                }
                Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            };
            let store = store.read();
            let generation = store.latest_tx_id()?;
            match cache.as_ref() {
                Some(c) if c.generation == generation => c.entities.clone(),
                _ => {
                    let deadline = quipu::time::Deadline::after_millis(SPOTLIGHT_FETCH_BUDGET_MS);
                    let fresh = Arc::new(semweb::fetch_labeled_entities_until(
                        &store,
                        Some(deadline),
                    )?);
                    *cache = Some(SpotlightCache {
                        generation,
                        entities: fresh.clone(),
                    });
                    fresh
                }
            }
        };
        Ok(axum::Json(semweb::spotlight_over(
            &entities, text, confidence,
        )))
    })
    .await
}

#[derive(serde::Deserialize)]
pub(crate) struct FragmentParams {
    subject: Option<String>,
    predicate: Option<String>,
    object: Option<String>,
    page: Option<usize>,
    #[serde(rename = "pageSize")]
    page_size: Option<usize>,
}

pub(crate) async fn fragments_handler(
    State(store): State<SharedStore>,
    Query(p): Query<FragmentParams>,
) -> Result<axum::response::Response, AppError> {
    let q = semweb::FragmentQuery {
        subject: p.subject,
        predicate: p.predicate,
        object: p.object,
        page: p.page.unwrap_or(1).max(1),
        page_size: p.page_size.unwrap_or(100).min(1000),
    };
    blocking(move || {
        let result = semweb::fragments(&store.lock(), &q)?;
        Ok((
            [
                (axum::http::header::CONTENT_TYPE, "application/json"),
                (axum::http::header::CACHE_CONTROL, "public, max-age=60"),
            ],
            axum::Json(result),
        )
            .into_response())
    })
    .await
}

pub(crate) async fn reconcile_handler(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    if input.get("queries").is_none() {
        return Ok(axum::Json(semweb::reconcile_manifest()));
    }
    blocking(move || {
        let queries = input
            .get("queries")
            .and_then(|v| v.as_object())
            .ok_or_else(|| quipu::Error::InvalidValue("'queries' must be an object".into()))?;
        let store = store.lock();
        Ok(axum::Json(semweb::reconcile(&store, queries)?))
    })
    .await
}

pub(crate) async fn preview_handler(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
        let html = semweb::preview_card(&store.lock(), &semweb::decode_iri(&iri))?;
        Ok((
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response())
    })
    .await
}
