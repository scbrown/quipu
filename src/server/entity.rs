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
        output_flag(params.expanded.as_deref()),
    )
    .await
}

#[derive(serde::Deserialize)]
pub(crate) struct EntityParams {
    pub(crate) iri: String,
    pub(crate) expanded: Option<String>,
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct OutputParams {
    pub(crate) expanded: Option<String>,
}

/// Query-form dereference endpoint for IRIs containing `/` and `#`.
pub(crate) async fn entity_query_conneg(
    State(store): State<SharedStore>,
    Query(params): Query<EntityParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    entity_response(
        store,
        params.iri,
        headers,
        output_flag(params.expanded.as_deref()),
    )
    .await
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
            output_flag(params.expanded.as_deref()),
        )?;
        Ok(json_ld_response(j))
    })
    .await
}

fn output_flag(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
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
