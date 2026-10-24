//! Export, share, import, promotion, and knot publication endpoints.

use axum::{extract::State, response::IntoResponse};
use serde_json::Value as JsonValue;

use super::{
    SharedStore,
    base::{AppError, blocking},
    tools::finish_deferred_embed,
};

#[path = "asof.rs"]
mod asof;

pub(crate) fn acquisition_routes() -> axum::Router<SharedStore> {
    axum::Router::new()
        .route("/acquire", axum::routing::post(acquire))
        .route("/share/asof", axum::routing::post(asof::share_asof))
        .route("/acquire/composed", axum::routing::post(acquire_composed))
        .route(
            "/acquire/eligibility",
            axum::routing::post(acquire_eligibility),
        )
}

pub(crate) async fn acquire(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<quipu::acquisition::Request>,
) -> Result<axum::Json<quipu::acquisition::Payload>, AppError> {
    super::admission::read_blocking(move || {
        let store = store.read();
        Ok(axum::Json(quipu::acquisition::acquire(&store, &input)?))
    })
    .await
}

pub(crate) async fn acquire_composed(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<quipu::acquisition::composed::Request>,
) -> Result<axum::Json<quipu::acquisition::composed::Payload>, AppError> {
    super::admission::read_blocking(move || {
        // A stable writer handle owns topology and the per-scope historical
        // anchor. Random pooled connections cannot promise that same anchor.
        let store = store.lock();
        Ok(axum::Json(quipu::acquisition::composed::acquire(
            &store, &input,
        )?))
    })
    .await
}

pub(crate) async fn acquire_eligibility(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<quipu::acquisition::eligibility::Request>,
) -> Result<axum::Json<quipu::acquisition::eligibility::Observation>, AppError> {
    super::admission::read_blocking(move || Ok(axum::Json(eligibility(&store, &input)?))).await
}

/// The eligibility handler body, on the calling thread's request identity.
pub(crate) fn eligibility(
    store: &super::StoreHandle,
    input: &quipu::acquisition::eligibility::Request,
) -> Result<quipu::acquisition::eligibility::Observation, AppError> {
    // Refused before any lock: the ordinary writer guard would settle a
    // signed attestation and spend its nonce, and this guard never does.
    if quipu::transaction_auth::current_attestation().is_some() {
        return Err(quipu::Error::InvalidValue(
            "eligibility observation does not accept a signed write attestation".into(),
        )
        .into());
    }
    // The writer handle holds the configured label floor and owns the main
    // connection; `read()` would synchronize reader providers.
    let store = store.metadata_lock();
    Ok(quipu::acquisition::eligibility::observe(&store, input)?)
}

pub(crate) async fn export(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
        // Fail closed: export builds packs and shares that leave the host, and an
        // unrecognised scope key (`scope`, `scope_kind`, `graph_iri`, ...) used to
        // fall through to a ROOT export with HTTP 200. ROOT stays the documented
        // default only when the request names no scope at all.
        if !input.is_object() {
            return Err(quipu::Error::InvalidValue(
                "export request must be a JSON object; nothing was exported".into(),
            )
            .into());
        }
        let unknown = super::input_fields::ignored("quipu_export", &input);
        if !unknown.is_empty() {
            return Err(quipu::Error::InvalidValue(format!(
                "export refused: unrecognised field(s) {}; nothing was exported. \
                 Scope with one of graph, group_id or construct, or omit all three for ROOT",
                unknown.join(", ")
            ))
            .into());
        }
        let format_str = input
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("turtle");
        let (format, content_type) = match format_str {
            "turtle" | "ttl" => (oxrdfio::RdfFormat::Turtle, "text/turtle"),
            "ntriples" | "nt" => (oxrdfio::RdfFormat::NTriples, "application/n-triples"),
            other => {
                return Err(quipu::Error::InvalidValue(format!(
                    "unknown export format: {other} (try: turtle, ntriples)"
                ))
                .into());
            }
        };
        let graph = input.get("graph").and_then(|v| v.as_str());
        let group = input.get("group_id").and_then(|v| v.as_str());
        let construct = input.get("construct").and_then(|v| v.as_str());
        if [graph.is_some(), group.is_some(), construct.is_some()]
            .into_iter()
            .filter(|v| *v)
            .count()
            > 1
        {
            return Err(quipu::Error::InvalidValue(
                "export accepts only one of graph, group_id, or construct".into(),
            )
            .into());
        }
        let store = store.read();
        let (bytes, _) = match (group, construct) {
            (Some(group_id), None) => quipu::export_rdf_group(&store, format, group_id)?,
            (None, Some(query)) => quipu::export_rdf_construct(&store, format, query)?,
            (None, None) => quipu::export_rdf_subset(&store, format, graph)?,
            _ => unreachable!("mutually exclusive export scopes checked above"),
        };
        Ok(([(axum::http::header::CONTENT_TYPE, content_type)], bytes).into_response())
    })
    .await
}

pub(crate) async fn share_payload(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<quipu::share::SharePayloadRequest>,
) -> Result<axum::Json<quipu::share::SharePayload>, AppError> {
    blocking(move || {
        let limit = input.effective_max_bytes();
        let store = store.read();
        Ok(axum::Json(quipu::share::share_payload(
            &store,
            &input.options(),
            limit,
        )?))
    })
    .await
}

/// `/import` in two phases (aegis-oa5), as `/episode` is (aegis-xi0).
///
/// Entity resolution is the expensive part of an import and it is read-only, so
/// it runs first on a READ connection. The writer is taken only to revalidate it
/// against what committed in between, validate and stage. One deadline, stamped
/// here, covers both phases and every step under the writer: before this, an
/// import had no deadline at all, and one share held the production writer for
/// over half an hour while the client that sent it had long since given up.
pub(crate) async fn import_share(
    State(store): State<SharedStore>,
    principal: Option<axum::Extension<quipu::http_auth::AuthenticatedPrincipal>>,
    axum::Json(input): axum::Json<quipu::share_import::ShareImportRequest>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let actor = principal.map(|axum::Extension(value)| value.as_str().to_owned());
    let input = std::sync::Arc::new(input);
    let (started, deadline) = super::admission::import_budget();
    let prepared = if store.resolves_off_writer() {
        let (reader_store, request) = (store.clone(), input.clone());
        super::admission::episode_read_blocking_until(started, deadline, move || {
            let mut reader = reader_store.vector_read();
            // A duplicate in a frozen layer must not go unreported: if the
            // reader cannot mount them, resolve under the writer instead.
            if reader.sync_frozen_attachments().is_err() {
                return Ok(None);
            }
            Ok(Some(quipu::share_import::prepare_import(
                &reader, &request,
            )?))
        })
        .await?
    } else {
        None
    };
    super::admission::write_blocking_until(std::time::Instant::now(), deadline, move || {
        let (mut result, work) = {
            let mut st = store.write_lock()?;
            quipu::transaction_auth::refuse_if_refused()?;
            let result = quipu::share_import::import_share_prepared(
                &mut st,
                &input,
                &quipu::time::now_iso(),
                actor.as_deref(),
                prepared,
            )?;
            (result, st.take_deferred_embed())
        };
        if let Some(work) = work
            && let Err(error) = finish_deferred_embed(&store, &work)
        {
            result.embedding.state = "failed".into();
            result.embedding.message = Some(format!("Graph staged, embedding failed: {error:?}"));
        }
        quipu::import_embedding::refresh(&store.lock(), &mut result)?;
        Ok(axum::Json(serde_json::to_value(result).map_err(|e| {
            quipu::Error::Serialization(format!("import response: {e}"))
        })?))
    })
    .await
}

pub(crate) async fn promote_import(
    State(store): State<SharedStore>,
    principal: Option<axum::Extension<quipu::http_auth::AuthenticatedPrincipal>>,
    axum::Json(input): axum::Json<quipu::share_import::PromoteImportRequest>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let actor = principal.map(|axum::Extension(value)| value.as_str().to_owned());
    // Promotion copies a whole staged graph into ROOT under the writer, so it
    // takes the same deadline as `/import`: past it, the promotion rolls back
    // whole and releases the writer (aegis-oa5).
    let (started, deadline) = super::admission::import_budget();
    super::admission::write_blocking_until(started, deadline, move || {
        let (result, work) = {
            let mut st = store.write_lock()?;
            quipu::transaction_auth::refuse_if_refused()?;
            let result = quipu::share_import::promote_import(
                &mut st,
                &input,
                &quipu::time::now_iso(),
                actor.as_deref(),
            )?;
            (result, st.take_deferred_embed())
        };
        if let Some(work) = work {
            finish_deferred_embed(&store, &work)?;
        }
        Ok(axum::Json(serde_json::to_value(result).map_err(|e| {
            quipu::Error::Serialization(format!("promotion response: {e}"))
        })?))
    })
    .await
}

pub(crate) async fn knot(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let (result, work) = {
            let mut st = store.lock();
            let result = quipu::tool_knot(&mut st, &input)?;
            (result, st.take_deferred_embed())
        };
        if let Some(work) = work {
            finish_deferred_embed(&store, &work)?;
        }
        Ok(axum::Json(super::input_fields::annotate(
            "quipu_knot",
            &input,
            result,
        )))
    })
    .await
}
