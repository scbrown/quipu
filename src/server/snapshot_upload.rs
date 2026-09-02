//! HTTP transport for resumable snapshot publication.

use axum::{Json, Router, extract::State, routing::post};
use quipu::store::snapshot_upload;
use serde_json::Value as JsonValue;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

#[derive(Clone)]
enum PromotionState {
    Running,
    Complete(JsonValue),
    Failed(String),
}

fn promotions() -> &'static Mutex<HashMap<String, PromotionState>> {
    static STATES: OnceLock<Mutex<HashMap<String, PromotionState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

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
    let upload_id = input
        .get("upload_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| quipu::Error::InvalidValue("missing upload_id".into()))?
        .to_string();
    if let Some(state) = promotions()
        .lock()
        .expect("promotion state lock")
        .get(&upload_id)
        .cloned()
    {
        return match state {
            PromotionState::Running => Ok(Json(
                serde_json::json!({"upload_id": upload_id, "pending": true}),
            )),
            PromotionState::Complete(result) => Ok(Json(result)),
            PromotionState::Failed(error) => Err(quipu::Error::InvalidValue(error).into()),
        };
    }
    promotions()
        .lock()
        .expect("promotion state lock")
        .insert(upload_id.clone(), PromotionState::Running);
    let response_upload_id = upload_id.clone();
    tokio::task::spawn_blocking(move || {
        let (result, work) = {
            let mut locked = store.lock();
            let result = match snapshot_upload::promote_snapshot_upload(&mut locked, &input) {
                Ok(result) => result,
                Err(error) => {
                    promotions()
                        .lock()
                        .expect("promotion state lock")
                        .insert(upload_id, PromotionState::Failed(error.to_string()));
                    return;
                }
            };
            (result, locked.take_deferred_embed())
        };
        if let Some(work) = work
            && let Err(error) = finish_deferred_embed(&store, &work)
        {
            promotions()
                .lock()
                .expect("promotion state lock")
                .insert(upload_id, PromotionState::Failed(format!("{error:?}")));
            return;
        }
        promotions()
            .lock()
            .expect("promotion state lock")
            .insert(upload_id, PromotionState::Complete(result));
    });
    Ok(Json(
        serde_json::json!({"upload_id": response_upload_id, "pending": true}),
    ))
}
