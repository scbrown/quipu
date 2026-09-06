//! Core HTTP surface: static UI, health/version/metrics, the /stats cache,
//! and the primary query endpoint.
//!
//! Also owns two things every other handler module depends on: [`blocking`],
//! which keeps store work off the async reactor, and [`AppError`], the
//! error-to-response mapping.

use std::sync::{Arc, Mutex};

use axum::{extract::State, http::StatusCode, response::IntoResponse};
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
    let (entities, facts, predicates, wal_bytes) = blocking(move || {
        // Metrics is read-only and must not join the writer queue. Prometheus
        // abandons timed-out responses, but spawn_blocking keeps their queued
        // work alive; one scrape per interval otherwise consumes task slots
        // until TasksMax starves unrelated store endpoints (aegis-vimo5).
        let store = store.read();
        let (e, f, p) = store.graph_counts()?;
        // Read on the pooled read connection alongside the counts rather than
        // in its own handler: one blocking hop, and the WAL number is then
        // taken at the same instant as the facts it should be read against.
        Ok((e, f, p, store.wal_bytes()))
    })
    .await?;
    let body = quipu::metrics::metrics().render(entities, facts, predicates, wal_bytes);
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
