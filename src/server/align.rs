//! HTTP transport for operator-driven cross-graph concept alignment (aegis-5qmg3r).
//!
//! These handlers call the same `tool_align_*` functions the MCP tools call, so
//! the two surfaces cannot diverge in implementation. What they do NOT share is
//! their arguments, which is why each surface still guards its own inputs — see
//! the module docs on `src/mcp/align.rs`.
//!
//! `propose` and `decide` are READS and take `&Store`, so they run on the WAL
//! read pool; `apply` is the ONLY writer and takes the lock. That split is the
//! engine's rather than a preference: `apply`'s signature is `&mut Store` and
//! the other two are `&Store`.

use axum::{Json, Router, extract::State, routing::post};
use serde_json::Value as JsonValue;

use super::{
    SharedStore,
    base::{AppError, blocking},
};

pub(crate) fn routes() -> Router<SharedStore> {
    Router::new()
        .route("/align/propose", post(align_propose))
        .route("/align/decide", post(align_decide))
        .route("/align/apply", post(align_apply))
}

/// READ — runs on the read pool.
async fn align_propose(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    blocking(move || {
        let st = store.lock();
        Ok(Json(quipu::tool_align_propose(&st, &input)?))
    })
    .await
}

/// READ — touches no store at all.
async fn align_decide(Json(input): Json<JsonValue>) -> Result<Json<JsonValue>, AppError> {
    blocking(move || Ok(Json(quipu::tool_align_decide(&input)?))).await
}

/// WRITE — the only alignment route that takes the writer.
async fn align_apply(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    blocking(move || {
        let mut st = store.lock();
        Ok(Json(quipu::tool_align_apply(&mut st, &input)?))
    })
    .await
}
