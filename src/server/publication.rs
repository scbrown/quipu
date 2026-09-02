//! Export, share, import, promotion, and knot publication endpoints.

use axum::{extract::State, response::IntoResponse};
use serde_json::Value as JsonValue;

use super::{
    SharedStore,
    base::{AppError, blocking},
    tools::finish_deferred_embed,
};

pub(crate) async fn export(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
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

pub(crate) async fn import_share(
    State(store): State<SharedStore>,
    principal: Option<axum::Extension<quipu::http_auth::AuthenticatedPrincipal>>,
    axum::Json(input): axum::Json<quipu::share_import::ShareImportRequest>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let actor = principal.map(|axum::Extension(value)| value.as_str());
    blocking(move || {
        let (result, work) = {
            let mut st = store.lock();
            let result =
                quipu::share_import::import_share(&mut st, &input, &quipu::time::now_iso(), actor)?;
            (result, st.take_deferred_embed())
        };
        if let Some(work) = work {
            finish_deferred_embed(&store, &work)?;
        }
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
    let actor = principal.map(|axum::Extension(value)| value.as_str());
    blocking(move || {
        let (result, work) = {
            let mut st = store.lock();
            let result = quipu::share_import::promote_import(
                &mut st,
                &input,
                &quipu::time::now_iso(),
                actor,
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
        Ok(axum::Json(result))
    })
    .await
}
