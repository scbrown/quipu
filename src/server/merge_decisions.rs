//! HTTP transport for resolving share-merge conflicts (aegis-yavo9c).
//!
//! The handlers call the same `tool_merge_*` functions the MCP tools do.
//! `/merge/decisions` is a READ; `/merge/apply` is the only writer.

use axum::{Json, Router, extract::State, routing::post};
use serde_json::Value as JsonValue;

use crate::{
    SharedStore,
    base::{AppError, blocking},
};

pub(crate) fn routes() -> Router<SharedStore> {
    Router::new()
        .route("/merge/decisions", post(merge_decisions))
        .route("/merge/apply", post(merge_apply))
}

/// READ: emits (and optionally proposes) decisions; writes nothing. Served from
/// the read pool: a full ROOT export must not hold the writer (as `/export`).
async fn merge_decisions(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    blocking(move || {
        let st = store.read();
        Ok(Json(crate::input_fields::annotate(
            "quipu_merge_decisions",
            &input,
            quipu::tool_merge_decisions(&st, &input)?,
        )))
    })
    .await
}

/// WRITE: one transaction, or nothing.
async fn merge_apply(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    blocking(move || {
        let mut st = store.lock();
        Ok(Json(crate::input_fields::annotate(
            "quipu_merge_apply",
            &input,
            quipu::tool_merge_apply(&mut st, &input)?,
        )))
    })
    .await
}
