//! HTTP transport for resumable snapshot publication.

use axum::{Json, Router, extract::State, routing::post};
use quipu::store::snapshot_upload;
use serde_json::Value as JsonValue;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use quipu::{promotion_control::PromotionControl, transaction_auth::Identity};
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

const LEASE: Duration = Duration::from_secs(30);
const RECEIPT_TTL: Duration = Duration::from_secs(3600);
const MAX_RECEIPTS: usize = 512;

type Key = (usize, String);
#[derive(Clone)]
enum PromotionState {
    Running,
    Complete(JsonValue),
    Failed(String),
    Cancelled,
}

struct Promotion {
    store: Weak<super::StoreHandle>,
    owner: Option<Identity>,
    control: Arc<PromotionControl>,
    state: PromotionState,
    touched: Instant,
}

fn promotions() -> &'static Mutex<HashMap<Key, Promotion>> {
    static STATES: OnceLock<Mutex<HashMap<Key, Promotion>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn upload_id(input: &JsonValue) -> quipu::Result<String> {
    input
        .get("upload_id")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned)
        .ok_or_else(|| quipu::Error::InvalidValue("missing or invalid upload_id".into()))
}

fn key(store: &SharedStore, id: &str) -> Key {
    (Arc::as_ptr(store) as usize, id.to_owned())
}

fn check_owner(p: &Promotion, identity: &Option<Identity>) -> quipu::Result<()> {
    if &p.owner != identity {
        return Err(quipu::Error::InvalidValue(
            "promotion belongs to another authenticated identity".into(),
        ));
    }
    Ok(())
}

fn response(id: &str, p: &Promotion) -> Result<Json<JsonValue>, AppError> {
    Ok(Json(match &p.state {
        PromotionState::Running => serde_json::json!({
            "upload_id": id, "pending": true,
            "state": if p.control.cancelled() { "cancellation_requested" } else { "running" },
            "lease_seconds": LEASE.as_secs(),
        }),
        PromotionState::Complete(result) => result.clone(),
        PromotionState::Cancelled => serde_json::json!({
            "upload_id": id, "pending": false, "cancelled": true, "state": "cancelled",
        }),
        PromotionState::Failed(error) => {
            return Err(quipu::Error::InvalidValue(error.clone()).into());
        }
    }))
}

use super::{
    SharedStore,
    base::{AppError, blocking},
    tools::finish_deferred_embed,
};

// Share-merge conflict resolution: shares in, like /import (aegis-yavo9c).
#[path = "merge_decisions.rs"]
mod merge_decisions;

pub(crate) fn routes() -> Router<SharedStore> {
    Router::new()
        .merge(merge_decisions::routes())
        .route("/import", post(super::publication::import_share))
        .route("/import/promote", post(super::publication::promote_import))
        .route("/knot", post(super::publication::knot))
        .route("/knot/stage", post(stage_part))
        .route("/knot/promote", post(promote))
        .route("/knot/cancel", post(cancel))
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

async fn cancel(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let id = upload_id(&input)?;
    let mut states = promotions().lock().expect("promotion state lock");
    let p = states
        .get_mut(&key(&store, &id))
        .ok_or_else(|| quipu::Error::InvalidValue("unknown promotion".into()))?;
    check_owner(p, &super::auth::request_identity())?;
    let accepted = matches!(p.state, PromotionState::Running) && p.control.cancel();
    Ok(Json(serde_json::json!({
        "upload_id": id, "cancellation_requested": accepted,
        "state": match p.state {
            PromotionState::Cancelled => "cancelled",
            PromotionState::Running if accepted => "cancellation_requested",
            PromotionState::Running => "committing",
            PromotionState::Complete(_) => "complete",
            PromotionState::Failed(_) => "failed",
        },
    })))
}

async fn promote(
    State(store): State<SharedStore>,
    Json(input): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    promote_with_lease(store, input, LEASE).await
}

async fn promote_with_lease(
    store: SharedStore,
    input: JsonValue,
    lease: Duration,
) -> Result<Json<JsonValue>, AppError> {
    let id = upload_id(&input)?;
    let identity = super::auth::request_identity();
    let key = key(&store, &id);
    let control = {
        // One critical section for lookup and admission: duplicate polls cannot
        // spawn duplicate workers. Keep live jobs even after their lease expires
        // until the worker has actually exited and released the writer.
        let mut states = promotions().lock().expect("promotion state lock");
        states.retain(|_, p| {
            p.store.upgrade().is_some()
                && (matches!(p.state, PromotionState::Running) || p.touched.elapsed() < RECEIPT_TTL)
        });
        if let Some(p) = states.get_mut(&key) {
            check_owner(p, &identity)?;
            let resume = matches!(p.state, PromotionState::Cancelled)
                && input.get("retry_cancelled").and_then(JsonValue::as_bool) == Some(true);
            if !resume {
                if matches!(p.state, PromotionState::Running) {
                    p.control.renew();
                }
                return response(&id, p);
            }
        }
        // Explicit same-owner resumption is allowed only after terminal rollback.
        // Ordinary retries, including polls during cancellation, never restart it.
        let receipts_after_admission = states.len() - usize::from(states.contains_key(&key));
        if receipts_after_admission >= MAX_RECEIPTS
            || states
                .iter()
                .any(|(k, p)| k.0 == key.0 && matches!(p.state, PromotionState::Running))
        {
            return Err(quipu::Error::InvalidValue(
                "promotion admission full; retry after active worker exits".into(),
            )
            .into());
        }
        let control = PromotionControl::new(lease);
        states.insert(
            key.clone(),
            Promotion {
                store: Arc::downgrade(&store),
                owner: identity.clone(),
                control: control.clone(),
                state: PromotionState::Running,
                touched: Instant::now(),
            },
        );
        control
    };
    let kind = super::request_middleware::request_write_kind();
    let provenance = super::request_middleware::request_write_provenance();
    tokio::task::spawn_blocking(move || {
        let result = quipu::transaction_auth::with_identity(identity, || {
            quipu::write_provenance::scoped(provenance, || {
                quipu::write_kind::scoped(kind, || {
                    quipu::promotion_control::scoped(control.clone(), || {
                        let (result, work) = {
                            let mut locked = store.promotion_lock(&control)?;
                            let result =
                                snapshot_upload::promote_snapshot_upload(&mut locked, &input)?;
                            (result, locked.take_deferred_embed())
                        };
                        if let Some(work) = work {
                            finish_deferred_embed(&store, &work)?;
                        }
                        Ok::<_, AppError>(result)
                    })
                })
            })
        });
        let mut states = promotions().lock().expect("promotion state lock");
        if let Some(p) = states.get_mut(&key) {
            p.state = match result {
                _ if control.cancelled() => PromotionState::Cancelled,
                Ok(result) => PromotionState::Complete(result),
                Err(error) => PromotionState::Failed(format!("{error:?}")),
            };
            p.touched = Instant::now();
        }
    });
    Ok(Json(serde_json::json!({
        "upload_id": id, "pending": true, "state": "running", "lease_seconds": LEASE.as_secs(),
    })))
}

#[cfg(test)]
#[path = "snapshot_upload_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "snapshot_cancel_contract_probe.rs"]
mod contract_probe;
