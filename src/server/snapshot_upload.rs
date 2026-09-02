//! HTTP transport for resumable snapshot publication.

use axum::{Json, Router, extract::State, routing::post};
use quipu::store::snapshot_upload;
use serde_json::Value as JsonValue;

use super::{
    SharedStore,
    base::{AppError, blocking},
    tools::finish_deferred_embed,
};

pub(crate) fn routes() -> Router<SharedStore> {
    Router::new()
        .route("/import", post(super::base::import_share))
        .route("/import/promote", post(super::base::promote_import))
        .route("/knot", post(super::base::knot))
        .route("/knot/stage", post(stage_part))
        .route("/knot/promote", post(promote))
}

async fn stage_part(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    blocking(move || {
        let mut store = store.lock();
        Ok(Json(snapshot_upload::stage_snapshot_part(
            &mut store, &input,
        )?))
    })
    .await
}

async fn promote(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    blocking(move || {
        let (result, work) = {
            let mut locked = store.lock();
            let result = snapshot_upload::promote_snapshot_upload(&mut locked, &input)?;
            (result, locked.take_deferred_embed())
        };
        if let Some(work) = work {
            finish_deferred_embed(&store, &work)?;
        }
        Ok(Json(result))
    })
    .await
}
