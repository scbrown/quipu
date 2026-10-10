//! Commit-driven transport over existing logs; no shadow log or implicit ACK.

use std::{
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
    event_cursor::{resume_offset, validate_frame},
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
    _slot: OwnedSemaphorePermit,
}

pub(crate) async fn events_stream(
    state: State<SharedStore>,
    headers: HeaderMap,
    params: Query<Params>,
) -> Response {
    stream(state.0, headers, params.0, false)
}

pub(crate) async fn changes_stream(
    state: State<SharedStore>,
    headers: HeaderMap,
    params: Query<Params>,
) -> Response {
    stream(state.0, headers, params.0, true)
}

fn stream(store: SharedStore, headers: HeaderMap, params: Params, changes: bool) -> Response {
    let last = match headers.get("last-event-id").map(|v| v.to_str()).transpose() {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid Last-Event-ID").into_response(),
    };
    let offset = match resume_offset(params.since, last) {
        Ok(offset) => offset,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    if changes && (params.types.is_some() || params.group.is_some())
        || !changes && params.graph.is_some()
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
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| Arc::new(Semaphore::new(64))).clone();
    let slot = match slots.try_acquire_owned() {
        Ok(slot) => slot,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "stream capacity exhausted; reconnect later",
            )
                .into_response();
        }
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
        _slot: slot,
    };
    let output = unfold(cursor, |mut cursor| async move {
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
                    json!({"since": cursor.offset, "limit": 1, "types": cursor.params.types, "group": cursor.params.group}),
                );
                match params {
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

fn headers_for_feed() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-quipu-client", "event-stream".parse().unwrap());
    headers
}

#[cfg(test)]
#[path = "feed_stream_tests.rs"]
mod tests;
