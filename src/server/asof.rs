//! `POST /share/asof`: the replica bootstrap cut (docs/design/as-of-share.md).
//!
//! The cut is spooled to disk under one read transaction, which closes before
//! the first byte is sent. The response is then an SSE stream read from the
//! spool. A cut that fits one frame is a single `quipu.asof` {manifest,
//! records}. A larger one is `quipu.asof.begin` (manifest, totals, digest,
//! source), numbered `quipu.asof.part` frames and a `quipu.asof.commit`
//! barrier, built by the same part builder as the change stream's oversized
//! transactions.
//!
//! Every refusal and every abort is answered before the stream starts, so a
//! follower never receives part of a cut it should not apply.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::KeepAlive},
};
use futures::stream::unfold;
use quipu::asof_share::{Cut, CutError, Limits, Request};
use serde_json::json;
use tokio::io::AsyncBufReadExt;

use crate::{
    SharedStore,
    base::AppError,
    feed_stream::{FeedFrame, stream_slot},
};

/// Seconds a refused follower is told to wait before asking again.
const RETRY_AFTER_SECS: u64 = 60;

/// Spools older than this are swept when a new cut starts.
const SPOOL_MAX_AGE: Duration = Duration::from_secs(3600);

fn spool_dir() -> PathBuf {
    std::env::var_os("QUIPU_ASOF_SPOOL_DIR").map_or_else(
        || std::env::temp_dir().join("quipu-asof-spool"),
        PathBuf::from,
    )
}

fn unavailable(error: &str, extra: &serde_json::Value) -> Response {
    let mut body = json!({"error": error, "retry_after_secs": RETRY_AFTER_SECS});
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(
            axum::http::header::RETRY_AFTER,
            RETRY_AFTER_SECS.to_string(),
        )],
        axum::Json(body),
    )
        .into_response()
}

/// One read phase per source. A second request is refused, not queued:
/// queued cuts would hold the pinned window open back to back.
static READING: AtomicBool = AtomicBool::new(false);

/// Released when the read phase ends, however it ends.
struct ReadPhase(&'static AtomicBool);

impl Drop for ReadPhase {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) async fn share_asof(
    State(store): State<SharedStore>,
    axum::Json(request): axum::Json<Request>,
) -> Response {
    // A send slot first: a cut nobody can be sent is a pinned WAL for nothing.
    let Some(slot) = stream_slot() else {
        return unavailable("stream_capacity_exhausted", &json!({}));
    };
    if READING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return unavailable("cut_in_progress", &json!({}));
    }
    let phase = ReadPhase(&READING);
    let dir = spool_dir();
    let result = crate::admission::read_blocking(move || {
        let _phase = phase;
        sweep_expired(&dir);
        let store = store.read();
        Ok(quipu::asof_share::cut(
            &store,
            &request,
            &dir,
            &Limits::default(),
        ))
    })
    .await;
    let cut = match result {
        Ok(Ok(cut)) => cut,
        Ok(Err(CutError::Aborted(reason))) => {
            return unavailable("cut_aborted", &json!({"reason": reason.as_str()}));
        }
        Ok(Err(CutError::Refused(error))) => return AppError::from(error).into_response(),
        Err(error) => return error.into_response(),
    };
    stream_cut(cut, slot)
}

/// Spools a crashed or abandoned send left behind.
fn sweep_expired(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > SPOOL_MAX_AGE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Deletes the spool once the stream is done with it, sent or abandoned.
struct Spool(PathBuf);

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

enum Next {
    Begin,
    Parts(Box<tokio::io::Lines<tokio::io::BufReader<tokio::fs::File>>>),
    Done,
}

struct Sending {
    next: Next,
    begin: Option<FeedFrame>,
    commit: Option<FeedFrame>,
    spool: Spool,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

/// A cut small enough for one frame: `quipu.asof` {manifest, records}.
fn single_frame(cut: &Cut) -> Option<FeedFrame> {
    if cut.parts.parts > 1 {
        return None;
    }
    let records = match std::fs::read_to_string(&cut.spool).ok()?.lines().next() {
        Some(line) => serde_json::from_str::<serde_json::Value>(line).ok()?["records"].clone(),
        None => json!([]),
    };
    let data = json!({"manifest": cut.manifest, "records": records}).to_string();
    quipu::feed_parts::fits_frame_len(data.len()).then_some(FeedFrame {
        event: "quipu.asof",
        id: Some(cut.manifest.watermark_tx),
        data,
    })
}

fn stream_cut(cut: Cut, slot: tokio::sync::OwnedSemaphorePermit) -> Response {
    if let Some(frame) = single_frame(&cut) {
        let spool = Spool(cut.spool);
        let output = futures::stream::once(async move {
            let _spool = spool;
            let _slot = slot;
            Ok::<_, Infallible>(frame.into_sse())
        });
        return Sse::new(output).into_response();
    }
    let source = json!(cut.manifest.source);
    let source_tx = cut.manifest.watermark_tx;
    let begin = FeedFrame {
        event: "quipu.asof.begin",
        id: None,
        data: json!({
            "source_tx": source_tx,
            "parts": cut.parts.parts,
            "records": cut.parts.records,
            "bytes": cut.parts.bytes,
            "digest": cut.parts.digest,
            "source": source,
            "manifest": cut.manifest,
            "manifest_sha256": cut.manifest_sha256,
        })
        .to_string(),
    };
    let commit = FeedFrame {
        event: "quipu.asof.commit",
        id: Some(source_tx),
        data: json!({
            "source_tx": source_tx,
            "parts": cut.parts.parts,
            "records": cut.parts.records,
            "digest": cut.parts.digest,
            "source": source,
            "manifest_sha256": cut.manifest_sha256,
        })
        .to_string(),
    };
    let sending = Sending {
        next: Next::Begin,
        begin: Some(begin),
        commit: Some(commit),
        spool: Spool(cut.spool),
        _slot: slot,
    };
    let output = unfold(sending, |mut sending| async move {
        let frame = match &mut sending.next {
            Next::Begin => {
                let begin = sending.begin.take()?;
                match tokio::fs::File::open(&sending.spool.0).await {
                    Ok(file) => {
                        sending.next =
                            Next::Parts(Box::new(tokio::io::BufReader::new(file).lines()));
                        begin
                    }
                    Err(_) => {
                        sending.next = Next::Done;
                        sending.commit = None;
                        error_frame()
                    }
                }
            }
            Next::Parts(lines) => match lines.next_line().await {
                Ok(Some(data)) => FeedFrame {
                    event: "quipu.asof.part",
                    id: None,
                    data,
                },
                Ok(None) => {
                    sending.next = Next::Done;
                    sending.commit.take()?
                }
                Err(_) => {
                    // No barrier after a failed read: the follower holds an
                    // incomplete cut and must not apply it.
                    sending.next = Next::Done;
                    sending.commit = None;
                    error_frame()
                }
            },
            Next::Done => return None,
        };
        Some((Ok::<_, Infallible>(frame.into_sse()), sending))
    });
    Sse::new(output)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn error_frame() -> FeedFrame {
    FeedFrame {
        event: "quipu.error",
        id: None,
        data: "as-of spool unreadable; discard the staged cut and request a new one".into(),
    }
}

#[cfg(test)]
#[path = "asof_tests.rs"]
mod tests;
