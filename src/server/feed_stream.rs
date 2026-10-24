//! Commit-driven transport over existing logs; no shadow log or implicit ACK.

use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::{Arc, OnceLock},
    time::Duration,
};

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
};
use futures::stream::unfold;
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use super::{
    SharedStore,
    base::blocking,
    event_cursor::{fits_frame, resume_offset, validate_frame},
    feed,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    since: Option<i64>,
    types: Option<String>,
    group: Option<String>,
    graph: Option<String>,
}

struct Cursor {
    store: SharedStore,
    hints: watch::Receiver<()>,
    offset: i64,
    params: Params,
    changes: bool,
    wait: bool,
    terminal: bool,
    /// Frames of one chunked transaction still to send, in order.
    pending: VecDeque<FeedFrame>,
    _slot: OwnedSemaphorePermit,
}

pub(crate) async fn events_stream(
    state: State<SharedStore>,
    headers: HeaderMap,
    params: Query<Params>,
) -> Response {
    stream(state.0, &headers, params.0, false)
}

pub(crate) async fn changes_stream(
    state: State<SharedStore>,
    headers: HeaderMap,
    params: Query<Params>,
) -> Response {
    stream(state.0, &headers, params.0, true)
}

fn stream(store: SharedStore, headers: &HeaderMap, params: Params, changes: bool) -> Response {
    let Ok(last) = headers.get("last-event-id").map(|v| v.to_str()).transpose() else {
        return (StatusCode::BAD_REQUEST, "invalid Last-Event-ID").into_response();
    };
    let offset = match resume_offset(params.since, last) {
        Ok(offset) => offset,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    if changes && (params.types.is_some() || params.group.is_some())
        || params
            .graph
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 2048)
        || params.group.as_ref().is_some_and(|s| s.len() > 2048)
        || params.types.as_ref().is_some_and(|s| s.len() > 2048)
    {
        return (
            StatusCode::BAD_REQUEST,
            "unsupported or oversized feed scope",
        )
            .into_response();
    }
    let Some(slot) = stream_slot() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "stream capacity exhausted; reconnect later",
        )
            .into_response();
    };
    // Subscribe BEFORE the first log read so a concurrent commit cannot fall
    // between the empty read and registering interest.
    let mut hints = store.commit_wake.subscribe();
    hints.borrow_and_update();
    let cursor = Cursor {
        store,
        hints,
        offset,
        params,
        changes,
        wait: false,
        terminal: false,
        pending: VecDeque::new(),
        _slot: slot,
    };
    let output = unfold(cursor, |mut cursor| async move {
        if let Some(frame) = cursor.pending.pop_front() {
            return Some((Ok::<_, Infallible>(frame.into_sse()), cursor));
        }
        if cursor.terminal {
            return None;
        }
        loop {
            if cursor.wait {
                // Hourly metadata reconcile covers external writers that did
                // not install this serving connection's hook. Heartbeats do
                // not query the store. No per-connection polling loop.
                let _ =
                    tokio::time::timeout(Duration::from_secs(3600), cursor.hints.changed()).await;
                let writer = cursor.store.clone();
                if blocking(move || {
                    // Do not settle a request attestation: this is a pure
                    // synchronization barrier, not an authenticated write.
                    drop(writer.writer.lock());
                    Ok(())
                })
                .await
                .is_err()
                {
                    cursor.terminal = true;
                }
            }
            if cursor.terminal {
                return Some((
                    Ok::<_, Infallible>(
                        Event::default()
                            .event("quipu.error")
                            .data("feed read unavailable; reconnect from last applied cursor"),
                    ),
                    cursor,
                ));
            }
            let page = if cursor.changes {
                let params = serde_json::from_value(
                    json!({"since": cursor.offset, "limit": 1, "capture": "old_and_new_values", "graph": cursor.params.graph}),
                );
                match params {
                    Ok(params) => {
                        feed::changes_get(
                            State(cursor.store.clone()),
                            headers_for_feed(),
                            Query(params),
                        )
                        .await
                    }
                    Err(_) => unreachable!("constructed change params"),
                }
            } else {
                let params = serde_json::from_value(
                    json!({"since": cursor.offset, "limit": 1, "types": cursor.params.types, "group": cursor.params.group, "graph": cursor.params.graph}),
                );
                match params {
                    // A named graph streams from its own log; ROOT from `/events`.
                    Ok(params) if cursor.params.graph.is_some() => {
                        feed::events_graph_get(
                            State(cursor.store.clone()),
                            headers_for_feed(),
                            Query(params),
                        )
                        .await
                    }
                    Ok(params) => {
                        feed::events_get(
                            State(cursor.store.clone()),
                            headers_for_feed(),
                            Query(params),
                        )
                        .await
                    }
                    Err(_) => unreachable!("constructed event params"),
                }
            };
            let body = match page {
                Ok(page) => page.0,
                Err(_) => {
                    cursor.terminal = true;
                    return Some((
                        Ok::<_, Infallible>(
                            Event::default()
                                .event("quipu.error")
                                .data("feed read refused; reconnect from last applied cursor"),
                        ),
                        cursor,
                    ));
                }
            };
            let key = if cursor.changes {
                "next_tx"
            } else {
                "next_offset"
            };
            let next = body
                .get(key)
                .and_then(Value::as_i64)
                .unwrap_or(cursor.offset);
            if next <= cursor.offset {
                cursor.wait = true;
                continue;
            }
            let data = body.to_string();
            if cursor.changes && !fits_frame(&data) {
                match chunk_transaction(&body, next) {
                    Ok(frames) => {
                        cursor.offset = next;
                        cursor.wait = false;
                        cursor.pending = frames;
                    }
                    Err(refusal) => {
                        cursor.terminal = true;
                        cursor.pending.push_back(refusal);
                    }
                }
                let first = cursor.pending.pop_front();
                return first.map(|frame| (Ok::<_, Infallible>(frame.into_sse()), cursor));
            }
            if validate_frame(cursor.offset, next, &data).is_err() {
                cursor.terminal = true;
                return Some((
                    Ok::<_, Infallible>(
                        Event::default()
                            .event("quipu.error")
                            .data("feed page exceeds frame budget; cursor not acknowledged"),
                    ),
                    cursor,
                ));
            }
            cursor.offset = next;
            cursor.wait = false;
            // This ID acknowledges only delivery. Changes IDs are transaction
            // IDs; never POST them to /events/commit (event offsets).
            let event = Event::default()
                .event(if cursor.changes {
                    "quipu.changes"
                } else {
                    "quipu.events"
                })
                .id(next.to_string())
                .data(data);
            return Some((Ok::<_, Infallible>(event), cursor));
        }
    });
    Sse::new(output)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// One of the 64 slots every long-lived feed stream holds while it sends,
/// or `None` when all are taken.
pub(crate) fn stream_slot() -> Option<OwnedSemaphorePermit> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(64)))
        .clone()
        .try_acquire_owned()
        .ok()
}

/// One feed frame, independent of transport: the event name, the cursor id
/// it acknowledges (if any) and its data, byte-for-byte as sent. SSE builds an
/// `Event` from it only at the edge, so another transport can send the same
/// frames and digests unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeedFrame {
    pub(crate) event: &'static str,
    pub(crate) id: Option<i64>,
    pub(crate) data: String,
}

impl FeedFrame {
    pub(crate) fn into_sse(self) -> Event {
        let event = Event::default().event(self.event);
        let event = match self.id {
            Some(id) => event.id(id.to_string()),
            None => event,
        };
        event.data(self.data)
    }
}

/// Split one change page (a single transaction) that exceeds the frame budget
/// into `quipu.changes.begin`, numbered `quipu.changes.part` frames and a
/// `quipu.changes.commit` barrier (local-sync-mode.md §6, F6).
///
/// Only the barrier carries the SSE id, so `Last-Event-ID` never lands inside a
/// transaction: a consumer that disconnects mid-transaction resumes from the
/// previous one and replays this transaction from its first part. `digest` is
/// SHA-256 over the concatenated `data` of every part, in order, exactly as
/// sent. Each part's `digest_before` is the same hash over the parts before it,
/// so a consumer can check the stream as it arrives without a part hashing
/// itself.
/// Before this, an oversized transaction ended the stream without advancing
/// the cursor, and every reconnect hit the same dead end.
///
/// A single record larger than a frame cannot be split; it is refused with
/// `quipu.changes.tx_too_large` and the cursor is not advanced.
pub(crate) fn chunk_transaction(page: &Value, next: i64) -> Result<VecDeque<FeedFrame>, FeedFrame> {
    let records = page
        .get("records")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut builder = quipu::feed_parts::PartBuilder::new(next);
    let mut parts = Vec::new();
    for record in records {
        match builder.push(&record) {
            Ok(closed) => parts.extend(closed),
            Err(quipu::feed_parts::RecordTooLarge) => {
                return Err(FeedFrame {
                    event: "quipu.changes.tx_too_large",
                    id: None,
                    data: json!({
                        "source_tx": next,
                        "reason": "one record exceeds the transport frame budget; cursor not advanced",
                    })
                    .to_string(),
                });
            }
        }
    }
    let (last, summary) = builder.finish();
    parts.extend(last);
    let digest = summary.digest;
    let mut frames = VecDeque::new();
    let mut commit = json!({
        "source_tx": next,
        "parts": summary.parts,
        "records": summary.records,
        "digest": digest,
    });
    // `source` names the store and log epoch, so a follower can epoch-check a
    // chunked transaction exactly as it checks a whole page (F5).
    for key in [
        "next_tx",
        "watermark_tx",
        "watermark_timestamp",
        "capture",
        "source",
    ] {
        commit[key] = page.get(key).cloned().unwrap_or(Value::Null);
    }
    frames.push_back(FeedFrame {
        event: "quipu.changes.begin",
        id: None,
        data: json!({
            "source_tx": next,
            "parts": summary.parts,
            "records": summary.records,
            "bytes": summary.bytes,
            "digest": digest,
            "source": page.get("source").cloned().unwrap_or(Value::Null),
        })
        .to_string(),
    });
    for part in parts {
        frames.push_back(FeedFrame {
            event: "quipu.changes.part",
            id: None,
            data: part,
        });
    }
    frames.push_back(FeedFrame {
        event: "quipu.changes.commit",
        id: Some(next),
        data: commit.to_string(),
    });
    Ok(frames)
}

fn headers_for_feed() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-quipu-client", "event-stream".parse().unwrap());
    headers
}

#[cfg(test)]
#[path = "feed_stream_tests.rs"]
mod tests;
