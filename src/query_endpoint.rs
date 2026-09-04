//! SPARQL Protocol transports and the primary query endpoint.

use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use super::{
    SharedStore,
    base::{AppError, blocking},
};

/// Conservative bounds for the two standard SPARQL Protocol transports.
/// GET is intentionally small enough for common proxies; clients with larger
/// queries have the standard `application/sparql-query` POST form available.
pub(crate) const MAX_QUERY_URI_BYTES: usize = 8 * 1024;
pub(crate) const MAX_SPARQL_QUERY_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
pub(crate) struct QueryParams {
    pub(crate) query: String,
    pub(crate) verbose: Option<String>,
}

/// SPARQL 1.1 Query Protocol GET: `/query?query=...`.
pub(crate) async fn query_get(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(params): Query<QueryParams>,
) -> Result<axum::response::Response, AppError> {
    if uri.to_string().len() > MAX_QUERY_URI_BYTES {
        return Ok((StatusCode::URI_TOO_LONG, "query URI exceeds 8192 bytes").into_response());
    }
    let query = protocol_dataset_query(&params.query, &uri)?;
    query_core(
        store,
        headers,
        json!({"query": query, "verbose": query_flag(params.verbose.as_deref()), "_sparql_protocol": true}),
    )
    .await
}

fn query_flag(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

/// SPARQL 1.1 Query Protocol POST plus the legacy JSON request form.
pub(crate) async fn query_post_http(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    query_post_at(store, headers, uri, body).await
}

#[cfg(test)]
pub(crate) async fn query_post(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    query_post_at(store, headers, "/query".parse().unwrap(), body).await
}

async fn query_post_at(
    store: SharedStore,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    if body.len() > MAX_SPARQL_QUERY_BYTES {
        return Ok((
            StatusCode::PAYLOAD_TOO_LARGE,
            "query body exceeds 1048576 bytes",
        )
            .into_response());
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map_or("", str::trim)
        .to_ascii_lowercase();
    let mut input = match content_type.as_str() {
        "application/json" => serde_json::from_slice(&body).map_err(|e| {
            AppError::from(quipu::Error::InvalidValue(format!(
                "invalid JSON query request: {e}"
            )))
        })?,
        "application/sparql-query" => {
            let text = std::str::from_utf8(&body).map_err(|e| {
                AppError::from(quipu::Error::InvalidValue(format!(
                    "SPARQL query body is not UTF-8: {e}"
                )))
            })?;
            json!({"query": text, "_sparql_protocol": true})
        }
        "application/x-www-form-urlencoded" => {
            let fields: Vec<_> = url::form_urlencoded::parse(&body).collect();
            let queries: Vec<_> = fields
                .iter()
                .filter_map(|(name, value)| (name == "query").then_some(value.as_ref()))
                .collect();
            if queries.len() != 1 {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    "form body must contain exactly one query parameter",
                )
                    .into_response());
            }
            json!({"query": queries[0], "_sparql_protocol": true})
        }
        _ => {
            return Ok((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json, application/sparql-query, or application/x-www-form-urlencoded",
            )
                .into_response());
        }
    };
    if input
        .get("_sparql_protocol")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let query = input.get("query").and_then(JsonValue::as_str).unwrap_or("");
        input["query"] = json!(protocol_dataset_query(query, &uri)?);
    }
    query_core(store, headers, input).await
}

fn protocol_dataset_query(query: &str, uri: &axum::http::Uri) -> Result<String, AppError> {
    let pairs = uri
        .query()
        .map(|query| url::form_urlencoded::parse(query.as_bytes()))
        .into_iter()
        .flatten();
    let mut clauses = String::new();
    for (name, value) in pairs {
        match name.as_ref() {
            "default-graph-uri" => clauses.push_str(&format!(" FROM <{value}>")),
            "named-graph-uri" => clauses.push_str(&format!(" FROM NAMED <{value}>")),
            _ => {}
        }
    }
    if clauses.is_empty() {
        return Ok(query.to_string());
    }
    let upper = query.to_ascii_uppercase();
    let position = if upper.trim_start().starts_with("CONSTRUCT") {
        upper.find("WHERE").ok_or_else(|| {
            AppError::from(quipu::Error::InvalidValue(
                "SPARQL CONSTRUCT dataset requires WHERE".into(),
            ))
        })?
    } else if upper.trim_start().starts_with("DESCRIBE") {
        upper.find("WHERE").unwrap_or(query.len())
    } else {
        upper
            .find("WHERE")
            .or_else(|| query.find('{'))
            .ok_or_else(|| {
                AppError::from(quipu::Error::InvalidValue(
                    "SPARQL protocol dataset requires a query graph pattern".into(),
                ))
            })?
    };
    Ok(format!(
        "{}{} {}",
        &query[..position],
        clauses,
        &query[position..]
    ))
}

/// Direct legacy JSON entry point retained for unit callers.
#[cfg(test)]
pub(crate) async fn query(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::response::Response, AppError> {
    query_core(store, headers, input).await
}

async fn query_core(
    store: SharedStore,
    headers: HeaderMap,
    input: JsonValue,
) -> Result<axum::response::Response, AppError> {
    let query_shape =
        quipu::request_usage::classify_query(input.get("query").and_then(JsonValue::as_str));
    // Content negotiation (aegis-u7ag): an explicit standard Accept header opts
    // into the W3C SPARQL 1.1 results/RDF shape; absent / */* / application/json
    // keeps the default bespoke rows shape byte-for-byte, so existing callers are
    // unaffected.
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Same normaliser the request middleware uses, so the label here and the
    // label on quipu_http_client_* are always the same string. Recomputed rather
    // than threaded through a request extension: it is a short string op, and one
    // shared function is a stronger guarantee of agreement than one shared value
    // passed through two layers.
    let client = super::query_usage::client(&headers);

    blocking(move || {
        // Query TEXT at START, before taking the store lock: the query that
        // never completes — or never gets the mutex — must still be on the
        // record. Completion-only text logging is how the mfg0 killer query
        // stayed invisible for its entire ~4h burn.
        {
            let text = super::query_usage::query_text(&input);
            eprintln!("{} query start: {text}", quipu::time::now_iso());
        }
        // WAIT is measured across the acquisition, HELD from just after it
        // (aegis-vxl81). The two are separated here rather than at the HTTP
        // boundary because that boundary cannot tell them apart, and conflating
        // them is what makes a queued caller look like an expensive one.
        let federation = store.federation.clone();
        let lock_t0 = std::time::Instant::now();
        // POOLED READ. SPARQL is read-only, so this takes a
        // read-only connection instead of the writer's mutex. `wait_secs` keeps
        // meaning the same thing — time spent acquiring — which is what makes
        // the before/after curve comparable rather than merely different.
        let store = store.read();
        let wait_secs = lock_t0.elapsed().as_secs_f64();
        let started = std::time::Instant::now();

        let run = |store: &quipu::Store| -> Result<axum::response::Response, AppError> {
            // quipu-tkh: `"federated": true` fans the WHOLE query text out
            // through the federated provider — the local store plus every
            // configured remote — and reports who answered (`providers` /
            // `complete`). Bespoke JSON shape only, and the temporal/graph
            // params are refused rather than dropped: they shape the LOCAL
            // evaluator's context and are not forwarded, so accepting them
            // would mean something different per member — silently.
            if input
                .get("federated")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                if let Some(p) = ["valid_at", "tx", "graph", "row_labels"]
                    .iter()
                    .find(|p| input.get(**p).is_some())
                {
                    return Err(quipu::Error::InvalidValue(format!(
                        "'{p}' is not supported on a federated query — federation \
                         fans the whole query text out to every member unchanged"
                    ))
                    .into());
                }
                let text = input.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
                    quipu::Error::InvalidValue("missing 'query' parameter".into())
                })?;
                // quipu-fd1: configured [quipu.labels] floors gate the
                // federated path exactly as the local one — the local members
                // via the same dataset fold `tool_query` applies, each remote
                // via the trust/freshness the LOCAL operator declared for it.
                // Before this, `federated: true` was the way around a
                // configured min_trust floor. A no-op when no floor is set.
                quipu::provider::check_federated_floor(store, text, &federation)?;
                let fed = quipu::provider::federated_from_config(store, "local", &federation)?;
                let fq = fed.query_all(text);
                let quipu::QueryResult::Select {
                    variables,
                    mut rows,
                } = fq.result
                else {
                    unreachable!("query_all always merges into a SELECT row set");
                };
                // The same ceiling the local path applies (hq-gkd): a federated
                // union must not become the way around max_sparql_rows.
                let max_rows = store.search_config().max_sparql_rows;
                let truncated = rows.len() > max_rows;
                if truncated {
                    rows.truncate(max_rows);
                }
                let verbose = input
                    .get("verbose")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                let prefixes = (!verbose)
                    .then(|| quipu::compact::PrefixMap::from_store(store))
                    .transpose()?;
                let json_rows: Vec<JsonValue> = rows
                    .iter()
                    .map(|row| {
                        JsonValue::Object(
                            row.iter()
                                .map(|(k, v)| {
                                    let value = prefixes.as_ref().map_or_else(
                                        || quipu::value_to_json(store, v),
                                        |map| quipu::value_to_json_with_prefixes(store, v, map),
                                    );
                                    (k.clone(), value)
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let providers: Vec<JsonValue> = fq
                    .providers
                    .iter()
                    .map(|o| {
                        json!({
                            "name": o.name, "ok": o.ok, "rows": o.rows, "error": o.error,
                            // The operator-declared label; null = undeclared,
                            // never fabricated (quipu-fd1).
                            "label": o.label.as_ref().map(quipu::DeclaredLabel::to_json),
                        })
                    })
                    .collect();
                // The composed label of the whole federated dataset — remote
                // declarations folded in as members, so composition never
                // widens. A fold refusal (cross-chain trust) is reported in
                // the field, not raised, matching the local path.
                let labels = quipu::provider::federated_dataset_labels(store, text, &federation);
                let mut body = json!({
                    "variables": variables,
                    "rows": json_rows,
                    "count": json_rows.len(),
                    "providers": providers,
                    "complete": fq.complete,
                    "labels": quipu::labels_json(&labels),
                });
                if truncated {
                    body["truncated"] = json!(true);
                }
                return Ok(super::query_usage::annotate(
                    axum::Json(body).into_response(),
                    query_shape,
                    json_rows.len(),
                ));
            }

            let format = quipu::w3c::negotiate(&accept).or({
                input
                    .get("_sparql_protocol")
                    .and_then(JsonValue::as_bool)
                    .filter(|enabled| *enabled)
                    .and(match query_shape {
                        quipu::request_usage::QueryShape::Select
                        | quipu::request_usage::QueryShape::Ask => {
                            Some(quipu::w3c::ResultFormat::SparqlJson)
                        }
                        quipu::request_usage::QueryShape::Construct
                        | quipu::request_usage::QueryShape::Describe => {
                            Some(quipu::w3c::ResultFormat::Turtle)
                        }
                        _ => None,
                    })
            });
            if let Some(fmt) = format {
                let (result, _truncated) = quipu::query_result_with_federation(
                    store,
                    &input,
                    Some(std::sync::Arc::new(federation.clone())),
                )?;
                if let Some((content_type, body)) = quipu::w3c::serialize(store, &result, fmt)? {
                    // The spec shapes have nowhere to carry the subclass-inference
                    // marker, so it rides a header instead of being dropped. Without
                    // this, one Accept header reopens the exact silence the marker
                    // closes on the bespoke shape — for SELECT as much as for ASK.
                    let mut headers = HeaderMap::new();
                    if let Ok(ct) = axum::http::HeaderValue::from_str(content_type) {
                        headers.insert(axum::http::header::CONTENT_TYPE, ct);
                    }
                    let inferred = quipu::query_inference(store, &input).unwrap_or_default();
                    if let Some(types) = quipu::inference_header(&inferred)
                        && let Ok(v) = axum::http::HeaderValue::from_str(&types)
                    {
                        headers.insert(axum::http::HeaderName::from_static("x-quipu-inference"), v);
                    }
                    return Ok(super::query_usage::annotate(
                        (headers, body).into_response(),
                        query_shape,
                        super::query_usage::result_size(&result),
                    ));
                }
                // Format did not fit the result variant (e.g. text/turtle for a SELECT);
                // fall through to the default shape rather than erroring.
            }

            let result = quipu::tool_query_with_federation(
                store,
                &input,
                Some(std::sync::Arc::new(federation.clone())),
            )?;
            let result_size = quipu::request_usage::json_result_size(&result);
            Ok(super::query_usage::annotate(
                axum::Json(result).into_response(),
                query_shape,
                result_size,
            ))
        };

        let result = run(&store);
        // Recorded while the guard is STILL ALIVE, so `held` is genuinely the
        // lock-holding interval and not "until the response was serialised".
        quipu::metrics::metrics().observe_store_time(
            &client,
            "/query",
            wait_secs,
            started.elapsed().as_secs_f64(),
        );
        // The request line above has method+status+duration; only a slow or
        // failed query earns its TEXT in the log — that is the one thing the
        // next wedge RCA needs and the one thing the middleware cannot
        // see (the body is consumed by then).
        let elapsed_ms = started.elapsed().as_millis();
        if elapsed_ms > 1_000 || result.is_err() {
            let text = super::query_usage::query_text(&input);
            eprintln!("query {elapsed_ms}ms: {text}");
        }
        result
    })
    .await
}
