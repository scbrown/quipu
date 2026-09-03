//! Core HTTP surface: static UI, health/version/metrics, the /stats cache,
//! and the primary query endpoint.
//!
//! Also owns two things every other handler module depends on: [`blocking`],
//! which keeps store work off the async reactor, and [`AppError`], the
//! error-to-response mapping.

use std::sync::{Arc, Mutex};

use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use super::SharedStore;

pub(crate) fn load_config(args: &[String]) -> quipu::QuipuConfig {
    let flag = |name| {
        args.windows(2)
            .find(|window| window[0] == name)
            .map(|window| window[1].as_str())
    };
    quipu::QuipuConfig::load(std::path::Path::new("."))
        .with_db_override(flag("--db"))
        .with_bind_override(flag("--bind"))
}

/// quipu #47: report the configured federation remotes at startup, and prove
/// they are REACHED rather than merely parsed. The declared label rides each
/// line (quipu-fd1): "undeclared" for a remote the operator has not labelled,
/// so a floor-configured deployment can see at startup which remotes a
/// federated query will be refused over.
/// Announce what the `[[quipu.attachments]]` declarations became (quipu-at2).
/// A composed layer nobody can see is how "it returned no rows" becomes a
/// mystery; a missing file already refused the open before this runs.
pub(crate) fn report_attachments(store: &quipu::Store) {
    for line in quipu::config::describe_attachments(store) {
        eprintln!("attached: {line}");
    }
}

/// Install the configured vector backend, or exit naming the refusal
/// (quipu-lv7).
///
/// Exits rather than continuing: a deployment that has run
/// `quipu migrate-vectors` and asked for `lancedb` would otherwise serve every
/// search out of the `SQLite` table it migrated away from, which looks like a
/// working server returning wrong answers.
pub(crate) fn apply_vector_backend(store: &mut quipu::Store, config: &quipu::QuipuConfig) {
    match quipu::config::install_vector_backend(store, config) {
        Ok(Some(path)) => eprintln!("vector backend: lancedb at {path}"),
        Ok(None) => {}
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn report_federation(store: &quipu::Store, federation: &quipu::FederationConfig) {
    match quipu::provider::federated_from_config(store, "local", federation) {
        Ok(fed) => {
            for status in fed.health_all() {
                let label = status
                    .label
                    .as_ref()
                    .map_or_else(|| "undeclared".to_string(), ToString::to_string);
                match (status.healthy, status.message.as_deref()) {
                    (true, _) => {
                        eprintln!("federation: '{}' reachable (label: {label})", status.name);
                    }
                    (false, Some(why)) => eprintln!(
                        "federation: '{}' NOT reachable (label: {label}) — {why}",
                        status.name
                    ),
                    (false, None) => {
                        eprintln!(
                            "federation: '{}' NOT reachable (label: {label})",
                            status.name
                        );
                    }
                }
            }
        }
        // A malformed label declaration (quipu-fd1): every federated query
        // will refuse with this same error, so say it once at startup too.
        Err(e) => eprintln!("federation: INVALID remote declaration — {e}"),
    }
}

pub(crate) async fn health() -> impl IntoResponse {
    axum::Json(json!({"status": "ok"}))
}

/// Prometheus scrape endpoint (usage measurement). Counters come from the
/// request middleware and the policy handler; graph-size gauges are computed
/// here with one cheap SQL aggregate — deliberately NOT /stats' full scan,
/// which must never run on every scrape while holding the store mutex.
pub(crate) async fn metrics_handler(
    State(store): State<SharedStore>,
) -> Result<impl IntoResponse, AppError> {
    let (entities, facts, predicates) = blocking(move || {
        // Metrics is read-only and must not join the writer queue. Prometheus
        // abandons timed-out responses, but spawn_blocking keeps their queued
        // work alive; one scrape per interval otherwise consumes task slots
        // until TasksMax starves unrelated store endpoints (aegis-vimo5).
        let store = store.read();
        Ok(store.graph_counts()?)
    })
    .await?;
    let body = quipu::metrics::metrics().render(entities, facts, predicates);
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    ))
}

/// What build is actually running (aegis-odnr).
///
/// `/health` answers `{"status":"ok"}` identically on every build ever made, so
/// "is the fix deployed?" could not be answered from outside — a P0 was filed
/// against the wrong root cause because source was read and the DEPLOYED server
/// was measured. The git SHA is the field that matters: a semantic version does
/// not move when a fix lands (shantytown's stayed 0.0.1 through every install
/// and never signalled drift once).
///
/// `dirty` is reported because a build from an uncommitted tree is NOT the SHA
/// it claims, and a deploy check that cannot see that is back where it started.
/// `features` is included so "compiled with shacl" stops being an inference
/// from the presence of a route.
pub(crate) async fn version() -> impl IntoResponse {
    axum::Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "git_sha": env!("QUIPU_GIT_SHA"),
        "git_dirty": env!("QUIPU_GIT_DIRTY") == "true",
        "features": compiled_features(),
    }))
}

/// Every declared feature and whether this binary has it (aegis-t1u2h).
///
/// Reads the `QUIPU_FEATURES` stamp that `build.rs` derives from `Cargo.toml`,
/// so a feature added to the manifest shows up here without anyone remembering
/// to edit this function. This replaced two hardcoded `cfg!` lines that reported
/// shacl and onnx and stayed silent about owl and reactive-reasoner — silent in
/// BOTH directions, so a binary with the OWL engine compiled out was
/// indistinguishable from one where it worked.
fn compiled_features() -> serde_json::Value {
    let mut map = serde_json::Map::new();
    // Strip the `quipu-features/<v>;` marker the stamp carries so the deploy gate
    // can find it unambiguously in `strings` output (see build.rs).
    let stamp = env!("QUIPU_FEATURES");
    let stamp = stamp.split_once(';').map_or(stamp, |(_, rest)| rest);
    for pair in stamp.split(',').filter(|s| !s.is_empty()) {
        if let Some((name, on)) = pair.split_once('=') {
            map.insert(name.to_string(), json!(on == "1"));
        }
    }
    serde_json::Value::Object(map)
}

/// Run store work on the blocking pool instead of an async worker (deploy: the
/// public-503 starvation).
///
/// Every store call here is synchronous, CPU-bound SQLite/SPARQL work behind a
/// `std::sync::Mutex`, and some of it runs for tens of seconds. Awaiting it
/// directly on a runtime worker parks that worker for the whole request. The
/// deployed unit sets `CPUQuota=100%`, so `available_parallelism()` reports 1-2
/// and tokio starts just **two** workers — a couple of slow requests park the
/// entire reactor, `/health` stops answering, and Traefik's health check (30s
/// interval, 5s timeout) ejects the backend. The public endpoint then 503s
/// "no available server" for everyone until the slow request finishes.
///
/// `spawn_blocking` moves the work to the blocking pool (512 threads by
/// default, independent of worker count), so the reactor stays free to answer
/// `/health` no matter how long a query runs. The store mutex still serializes
/// store access — that is a separate, intended property — but it no longer
/// serializes the HTTP server itself.
pub(crate) async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        // Only reachable if the handler panicked; the mutex is then poisoned and
        // the process is not going to recover on its own either way.
        Err(e) => Err(AppError(quipu::Error::InvalidValue(format!(
            "request handler failed: {e}"
        )))),
    }
}

/// Generation-keyed cache of the /stats aggregate (facts/entities/predicates).
///
/// /stats full-scans every triple (`SELECT ?s ?p ?o`) and builds distinct
/// subject/predicate sets — ~1.0s at 152k triples, paid on EVERY call, which
/// matters when a monitor polls it. The aggregate is cached and
/// keyed on `Store::latest_tx_id()`, the same pattern as `SpotlightCache`: under
/// polling only the first call after a write pays the scan; the rest hold the
/// store lock for one indexed MAX. Any write moves the generation and
/// invalidates naturally, so the counts are exact, never stale.
pub(crate) struct StatsCache {
    pub(crate) generation: i64,
    pub(crate) value: Arc<JsonValue>,
}

pub(crate) static STATS_CACHE: Mutex<Option<StatsCache>> = Mutex::new(None);

pub(crate) async fn stats(
    State(store): State<SharedStore>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let value = {
            let store = store.lock();
            let generation = store.latest_tx_id()?;
            let mut cache = STATS_CACHE.lock().unwrap();
            match cache.as_ref() {
                Some(c) if c.generation == generation => c.value.clone(),
                _ => {
                    let result = quipu::sparql_query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }")?;
                    let mut entities = std::collections::HashSet::new();
                    let mut predicates = std::collections::HashSet::new();
                    for row in result.rows() {
                        if let Some(quipu::Value::Ref(id)) = row.get("s") {
                            entities.insert(*id);
                        }
                        if let Some(quipu::Value::Ref(id)) = row.get("p") {
                            predicates.insert(*id);
                        }
                    }
                    let fresh = Arc::new(json!({
                        "facts": result.rows().len(),
                        "entities": entities.len(),
                        "predicates": predicates.len()
                    }));
                    *cache = Some(StatsCache {
                        generation,
                        value: fresh.clone(),
                    });
                    fresh
                }
            }
        };
        Ok(axum::Json((*value).clone()))
    })
    .await
}

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

// ⚠ ro_handler! / rw_handler! ARE A NAMING CONVENTION, NOT A TYPE GUARANTEE
// (aegis-e163). ro_handler! hands the tool a `&Store` and rw_handler! a
// `&mut Store`, but `&Store` is NOT a read-only capability here: `Store` writes
// through `&self` methods (interior mutability over the SQLite handle), so a tool
// with a `&Store` parameter can and does commit transactions. An earlier comment
// on `/resolve` claimed "ro_handler! hands a &Store, so the route cannot write
// even by mistake; that is a type, not a convention" — that was false, and five
// ro_handler! routes wrote (shapes, propose, accept/reject proposal,
// overlay_create), all now rw_handler!.
//
// So the tier you pick here is a CLAIM you are responsible for, not something the
// compiler checks. The invariant that IS enforced: `macro_tier_matches_write_
// classification` (http_auth.rs) fails the build if an rw_handler! route is
// absent from WRITE_ENDPOINTS or an ro_handler! route is present in it — i.e. the
// macro tier and the read-only/auth classification cannot silently disagree
// (that disagreement, one layer down, is the aegis-2f4n bypass). It cannot prove
// an ro_handler! tool does not write — only a real read-only handle could, and
// that is a larger refactor (a `ReadStore` newtype) noted on the bead. Until
// then: if a tool calls a mutating store method, register it rw_handler! AND add
// its route to WRITE_ENDPOINTS.

#[derive(Debug)]
pub(crate) struct AppError(quipu::Error);

impl From<quipu::Error> for AppError {
    fn from(e: quipu::Error) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        // A governance denial is an authorization outcome, not a malformed
        // request — surface it as 403 so callers can distinguish it.
        let status = match &self.0 {
            quipu::Error::PolicyDenied(_) => StatusCode::FORBIDDEN,
            // A query that ran out of budget is neither malformed nor a server
            // fault — 408 lets a caller distinguish "narrow your query" from
            // both.
            quipu::Error::QueryTimeout { .. } => StatusCode::REQUEST_TIMEOUT,
            // A join explosion is a property of the QUERY (its error names the
            // limit and how to fix the query) — 422: well-formed, unprocessable
            // as written. Distinct from 408 so dashboards can tell "slow" from
            // "explodes".
            quipu::Error::QueryComplexity { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::BAD_REQUEST,
        };
        let body = json!({ "error": self.0.to_string() });
        (status, axum::Json(body)).into_response()
    }
}

pub(crate) fn print_usage() {
    println!(
        "quipu-server {} -- REST API for the Quipu knowledge graph

USAGE:
    quipu-server [--db <path>] [--bind <addr>] [--embed-backfill]

OPTIONS:
    --db <path>       Store file (default: from .bobbin/config.toml)
    --bind <addr>     Listen address (default: from .bobbin/config.toml)
    --embed-backfill  Backfill embeddings for all entities on startup
    -V, --version     Print version and exit
    -h, --help        Print this help and exit",
        env!("CARGO_PKG_VERSION")
    );
}

/// Apply the OWL write-time constraint policy (aegis-bmqup) and, opt-in, the
/// reactive materialization observer (quipu-923). Split out of `main` so
/// server.rs stays inside its file-size ratchet baseline.
///
/// Without the `clone_from` the flag is UNREACHABLE: the config field, the
/// store field and the write gate can all exist and `owl.validate_on_write =
/// true` still does nothing, because nothing carries it across — it shipped
/// that way for one test cycle, caught only by an end-to-end write.
pub(crate) fn apply_owl(store: &mut quipu::Store, config: &quipu::QuipuConfig) {
    #[cfg(feature = "owl")]
    {
        store.owl_config_mut().clone_from(&config.owl);
        if config.owl.validate_on_write {
            eprintln!(
                "OWL write-validation enabled (disjointWith + FunctionalProperty enforced on every write)"
            );
        }
        #[cfg(feature = "reactive-reasoner")]
        if config.owl.reactive_materialize {
            store.add_observer(std::sync::Arc::new(quipu::ReactiveOwl));
            eprintln!(
                "reactive OWL materialization enabled — loaded ontologies re-materialize \
                 when touched vocabulary changes"
            );
        }
        // A settable knob this build cannot act on must be LOUD, not silent.
        #[cfg(not(feature = "reactive-reasoner"))]
        if config.owl.reactive_materialize {
            eprintln!(
                "warning: owl.reactive_materialize is set but this build lacks the \
                 reactive-reasoner feature — the observer is NOT registered"
            );
        }
    }
    #[cfg(not(feature = "owl"))]
    {
        let _ = store;
        if config.owl.validate_on_write || config.owl.reactive_materialize {
            eprintln!(
                "warning: [quipu.owl] flags are set but this build lacks the owl feature — ignored"
            );
        }
    }
}
