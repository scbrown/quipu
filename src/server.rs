//! Quipu REST API server — HTTP interface to the knowledge graph.
//! Usage: `quipu-server [--db <path>] [--bind <addr>]`

use std::sync::{Arc, Mutex};

use parking_lot::FairMutex;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use quipu::{EmbeddingProvider, semweb};
use serde_json::{Value as JsonValue, json};

/// FAIR (FIFO) mutex on purpose: std's Mutex is unfair, so a
/// sustained stream of episode writers could re-acquire the lock ahead of
/// readers indefinitely — during the mfg0 incident a `SELECT ... LIMIT 1`
/// measured a 38.5s wait behind a write flood. `FairMutex` hands the lock to
/// the longest waiter, bounding every request's wait to the queue ahead of
/// it. (`parking_lot`'s `lock()` has no poison Result — a panic while holding
/// the lock simply unlocks, which is fine: Store keeps its invariants in
/// `SQLite` transactions, not in Rust-visible state.)
type SharedStore = Arc<FairMutex<quipu::Store>>;

const UI_HTML: &str = include_str!("../ui/index.html");
const COMPONENTS_JS: &str = include_str!("../ui/quipu-components.js");

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Asking the binary who it is must NOT touch disk (aegis-j0nq). These are
    // pure reads of compiled-in constants and must stay above Store::open.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("quipu-server {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return;
    }

    let db_flag = args
        .windows(2)
        .find(|w| w[0] == "--db")
        .map(|w| w[1].as_str());

    let bind_flag = args
        .windows(2)
        .find(|w| w[0] == "--bind")
        .map(|w| w[1].as_str());

    let config = quipu::QuipuConfig::load(std::path::Path::new("."))
        .with_db_override(db_flag)
        .with_bind_override(bind_flag);

    let db_path = config.store_path.to_string_lossy().to_string();
    let bind_addr = config.server.bind.clone();

    // warn LOUDLY about documented, settable knobs this server does not
    // act on (vector.backend = lancedb, federation.remotes), rather than accepting
    // them and silently doing nothing.
    for warning in config.unwired_warnings() {
        eprintln!("warning: {warning}");
    }

    let mut store = quipu::Store::open(&db_path).unwrap_or_else(|e| {
        eprintln!("error opening store {db_path}: {e}");
        std::process::exit(1);
    });

    // Mint IRIs under the CONFIGURED base namespace, not the hardcoded aegis
    // default (aegis-4h3x) — without this, `[quipu] base_ns = "..."` was inert
    // and every REST/MCP ingest silently minted aegis IRIs. Data identity, so
    // it must be set before the first write; announce it when it is non-default
    // so a non-aegis deployment can see the namespace it is actually writing.
    store.set_base_ns(config.base_ns.clone());
    if config.base_ns != quipu::namespace::DEFAULT_BASE_NS {
        eprintln!("minting IRIs under configured base_ns: {}", config.base_ns);
    }

    // Apply the entity-resolution policy so episode ingest actually dedups
    // (hq-uye) — without this, `[quipu.resolution] enabled = true` is inert.
    store.resolution_config_mut().clone_from(&config.resolution);
    if config.resolution.enabled {
        eprintln!(
            "entity resolution enabled (threshold={}, top_k={}, strict={})",
            config.resolution.threshold, config.resolution.top_k, config.resolution.strict_mode
        );
    }

    // Apply search/limit guardrails so callers can't request unbounded result
    // sets or scan the whole fact log (hq-gkd).
    store.search_config_mut().clone_from(&config.search);

    // Apply the SHACL validation policy so episode writes can be gated against
    // persistently-loaded shapes, not just episode-inline shapes (hq-c6s).
    store.shacl_config_mut().clone_from(&config.shacl);
    if config.shacl.validate_on_write {
        eprintln!("SHACL write-validation enabled (loaded shapes enforced on every write)");
    }

    // Apply the governance enforcement policy: when enabled, `boundary:"action"`
    // policies gate every write (the loom's write-time gate, see
    // docs/design/policy-edit-hooks.md).
    store.governance_config_mut().clone_from(&config.governance);
    if config.governance.enforce_on_write {
        eprintln!(
            "Governance write-enforcement enabled (action-boundary policies gate every write)"
        );
    }

    if let (Some(model_path), Some(tokenizer_path)) = (
        &config.embedding.model_path,
        &config.embedding.tokenizer_path,
    ) {
        match quipu::OnnxEmbeddingProvider::load(
            model_path,
            tokenizer_path,
            config.embedding.dimension,
            config.embedding.max_sequence_length,
        ) {
            Ok(provider) => {
                let dim = provider.dimension();
                store.set_embedding_provider(Arc::new(provider));
                store.embedding_config_mut().auto_embed = config.embedding.auto_embed;
                store.embedding_config_mut().embed_batch_size = config.embedding.embed_batch_size;
                // Server mode defers the write-path ONNX embed OUT of the
                // store lock: an episode write held the global
                // mutex ~10s while embedding under it, so a write flood
                // starved every reader. Write handlers drain + finish via
                // finish_deferred_embed. CLI/library keep the inline path.
                store.set_defer_auto_embed(true);
                eprintln!(
                    "ONNX embedding provider loaded (dim={dim}, auto_embed={}, deferred)",
                    config.embedding.auto_embed
                );
            }
            Err(e) => {
                eprintln!("warning: failed to load ONNX embedder: {e}");
                eprintln!("  model: {}", model_path.display());
                eprintln!("  tokenizer: {}", tokenizer_path.display());
                eprintln!("  vector search will be unavailable");
            }
        }
    }

    // v1 verdict signing (the loom, Phase 0): load-or-generate the host-file
    // ed25519 key. QUIPU_SIGNING_KEY overrides the default path.
    let signing_key_path = std::env::var("QUIPU_SIGNING_KEY").map_or_else(
        |_| std::path::Path::new(".quipu").join("verifier.pk8"),
        std::path::PathBuf::from,
    );
    match quipu::signing::SigningIdentity::load(&signing_key_path, "quipu") {
        Ok(id) => {
            eprintln!(
                "verdict signing enabled (verifier=quipu, key={})",
                signing_key_path.display()
            );
            eprintln!(
                "  register this public key to trust its verdicts: {}",
                id.public_key_hex()
            );
            store.set_signing_identity(Arc::new(id));
        }
        Err(e) => eprintln!("warning: verdict signing disabled -- {e}"),
    }

    let state: SharedStore = Arc::new(FairMutex::new(store));
    let push_store_outer = state.clone();

    // Access-control policy for write endpoints (hq-azs). Decision logic lives
    // in quipu::http_auth (unit-tested); this only wires it into axum.
    let auth_token = config.server.auth_token.clone();
    let read_only = config.server.read_only;
    if read_only {
        eprintln!("server is READ-ONLY — write endpoints will return 403");
    }
    if auth_token.is_some() {
        eprintln!("write endpoints require a bearer token");
    }

    // CORS: an allowlist restricts cross-origin requests when configured; an
    // empty list preserves the prior allow-any behaviour. AUTHORIZATION is
    // allowed so a browser can present the bearer token cross-origin.
    let cors_origins = config.server.cors_allowed_origins.clone();
    let cors_base = if cors_origins.is_empty() {
        tower_http::cors::CorsLayer::new().allow_origin(tower_http::cors::Any)
    } else {
        let origins: Vec<axum::http::HeaderValue> =
            cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        tower_http::cors::CorsLayer::new().allow_origin(origins)
    };
    let cors = cors_base
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    if args.iter().any(|a| a == "--embed-backfill") {
        eprintln!("Running embedding backfill for all entities...");
        let mut s = state.lock();
        match backfill_embeddings(&mut s) {
            Ok(count) => eprintln!("Backfill complete: {count} entities embedded"),
            Err(e) => eprintln!("Backfill error: {e}"),
        }
    }

    let app = Router::new()
        // UI
        .route("/", get(ui))
        .route("/ui", get(ui))
        .route("/quipu-components.js", get(components_js))
        // Core API
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/stats", get(stats))
        .route("/metrics", get(metrics_handler))
        .route("/query", post(query))
        .route("/knot", post(knot))
        .route("/cord", post(cord))
        .route("/unravel", post(unravel))
        .route("/validate", post(validate))
        .route("/episode", post(episode))
        .route("/search", post(search))
        // Read-only resolution dry-run: "what would resolution
        // say about this name?" WITHOUT writing. Before this route, the answer
        // was observable only as a side effect of POST /episode — consumers
        // (the graph-extract minting gate) had to reimplement the name matcher
        // client-side and post-then-retract to see embedding matches: undo,
        // not prevention.
        .route("/resolve", post(resolve_probe))
        .route("/hybrid_search", post(hybrid_search))
        .route("/unified_search", post(unified_search))
        .route("/ask", post(ask))
        .route("/search_nodes", post(search_nodes))
        .route("/search_facts", post(search_facts))
        .route("/search/nodes", post(graphiti_search_nodes))
        .route("/episodes/complete", post(episodes_complete))
        .route("/impact", post(impact_analysis))
        .route("/retract", post(retract))
        .route("/set", post(set_predicate))
        .route("/episode/retract", post(retract_episode))
        .route("/shapes", post(shapes))
        .route("/subscriptions", post(subscriptions))
        .route("/propose", post(propose_schema_change))
        .route("/proposals", post(list_proposals))
        .route("/proposal/accept", post(accept_proposal))
        .route("/proposal/reject", post(reject_proposal))
        .route("/overlay/create", post(overlay_create))
        .route("/overlay/write", post(overlay_write))
        .route("/overlay/compose", post(overlay_compose))
        .route("/cooccurrence", post(cooccurrence))
        .route("/policy/check", post(policy_check))
        .route("/verifier/authorized", post(verifier_authorized))
        .route("/verdict/verify", post(verdict_verify))
        .route("/project", post(project_graph))
        .route("/report", get(report_get).post(report))
        .route("/context", post(context))
        .route("/embed_backfill", post(embed_backfill))
        // Entity + history
        .route("/entity/{iri}", get(entity_conneg))
        .route("/entity/{iri}/json", get(entity_json))
        .route("/entity/{iri}/ttl", get(entity_turtle_suffix))
        .route("/entity/{iri}/html", get(entity_html))
        .route("/entity_history", post(entity_history))
        .route("/transactions", get(transactions))
        // Event log pull API (event-log P1)
        .route("/events", get(events_get))
        .route("/events/commit", post(events_commit))
        // Semantic web APIs (Phase 4)
        .route("/spotlight", post(spotlight_handler))
        .route("/fragments", get(fragments_handler))
        .route("/reconcile", post(reconcile_handler))
        .route("/preview/{iri}", get(preview_handler))
        // Auth / read-only guard on write endpoints (hq-azs). Added before the
        // CORS layer so CORS stays outermost and answers OPTIONS preflight
        // before the guard runs.
        .layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let auth_token = auth_token.clone();
                async move {
                    let is_write = quipu::http_auth::is_write_endpoint(req.uri().path());
                    let auth_header = req
                        .headers()
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    match quipu::http_auth::authorize(
                        is_write,
                        read_only,
                        auth_token.as_deref(),
                        auth_header.as_deref(),
                    ) {
                        quipu::http_auth::AccessDecision::Allow => next.run(req).await,
                        quipu::http_auth::AccessDecision::Unauthorized => {
                            StatusCode::UNAUTHORIZED.into_response()
                        }
                        quipu::http_auth::AccessDecision::ReadOnly => {
                            StatusCode::FORBIDDEN.into_response()
                        }
                    }
                }
            },
        ))
        // CORS: allow the Bobbin Knowledge tab (and other browser origins) to
        // call /query, /search, /episode, etc. cross-origin, incl. OPTIONS
        // preflight (GH#5). Built above from the configured allowlist.
        .layer(cors)
        // Body limit: axum's 2MB default silently caps /knot at small graphs —
        // a code-graph promotion of one real repository is ~9MB of Turtle and
        // was refused 413. 64MB bounds a whole-repo promotion with headroom
        // while still refusing runaway bodies.
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        // Request log, outermost so every request (including CORS preflights
        // and auth rejections) leaves a trace. Until this existed, server.log
        // was startup banners only, and a wedged instance left NOTHING to RCA
        // from — the whole incident was reconstructed from gdb.
        // stderr on purpose: stdout is block-buffered under systemd, stderr is
        // not, and the journal timestamps every line itself.
        .layer(axum::middleware::from_fn(
            |req: axum::extract::Request, next: axum::middleware::Next| async move {
                let method = req.method().clone();
                let path = req.uri().path().to_string();
                // The ROUTE TEMPLATE (`/entity/{iri}`), not the raw path, so
                // metric label cardinality stays bounded. Absent for 404s.
                let endpoint = req
                    .extensions()
                    .get::<axum::extract::MatchedPath>()
                    .map_or_else(|| "unmatched".to_string(), |m| m.as_str().to_string());
                // Log at START with an id, then again at completion. The
                // request that never completes is exactly the one an RCA
                // needs, and completion-only logging guarantees it is the one
                // missing from the log (the mfg0 wedge left NO trace of the
                // killer query). The id ties the two lines together across
                // interleaved requests; the inline timestamp survives
                // plain-file stderr redirects where no journald stamps lines.
                static REQ_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let id = REQ_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                eprintln!("{} req#{id} {method} {path} ...", quipu::time::now_iso());
                let started = std::time::Instant::now();
                let resp = next.run(req).await;
                let status = resp.status().as_u16();
                quipu::metrics::metrics().observe_request(
                    &endpoint,
                    status,
                    started.elapsed().as_secs_f64(),
                );
                eprintln!(
                    "{} req#{id} {method} {path} -> {status} in {}ms",
                    quipu::time::now_iso(),
                    started.elapsed().as_millis()
                );
                resp
            },
        ))
        .with_state(state);

    // Event push P2: the delivery worker. A 2s tick over deliver_tick with the
    // real poster; each tick runs under spawn_blocking (SQLite + ureq are
    // sync). Cursor semantics make every tick idempotent, so the loop needs no
    // state of its own and a missed tick delays, never loses.
    {
        let push_store = push_store_outer;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let store = push_store.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let store = store.lock();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
                    let mut post = |url: &str, body: &serde_json::Value| -> bool {
                        matches!(
                            ureq::post(url)
                                .set("Content-Type", "application/json")
                                .timeout(std::time::Duration::from_secs(10))
                                .send_string(&body.to_string()),
                            Ok(r) if (200..300).contains(&r.status())
                        )
                    };
                    match quipu::store::push::deliver_tick(&store, now, &mut post) {
                        Ok(outcomes) => {
                            for (sub, o) in outcomes {
                                if let quipu::store::push::Delivery::Failed = o {
                                    eprintln!(
                                        "push: delivery to '{sub}' failed; will retry (cursor held)"
                                    );
                                }
                            }
                        }
                        Err(e) => eprintln!("push: tick error: {e}"),
                    }
                })
                .await;
            }
        });
    }

    eprintln!("quipu-server listening on {bind_addr} (db: {db_path})");

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("error binding {bind_addr}: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap();
}

/// Pure read of compiled-in text — must never touch the store (aegis-j0nq).
fn print_usage() {
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

async fn ui() -> Html<&'static str> {
    Html(UI_HTML)
}

async fn components_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        COMPONENTS_JS,
    )
}

async fn health() -> impl IntoResponse {
    axum::Json(json!({"status": "ok"}))
}

/// Prometheus scrape endpoint (usage measurement). Counters come from the
/// request middleware and the policy handler; graph-size gauges are computed
/// here with one cheap SQL aggregate — deliberately NOT /stats' full scan,
/// which must never run on every scrape while holding the store mutex.
async fn metrics_handler(State(store): State<SharedStore>) -> Result<impl IntoResponse, AppError> {
    let (entities, facts, predicates) = blocking(move || {
        let store = store.lock();
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
async fn version() -> impl IntoResponse {
    axum::Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "git_sha": env!("QUIPU_GIT_SHA"),
        "git_dirty": env!("QUIPU_GIT_DIRTY") == "true",
        "features": {
            "shacl": cfg!(feature = "shacl"),
            "onnx": cfg!(feature = "onnx"),
        }
    }))
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
async fn blocking<T, F>(f: F) -> Result<T, AppError>
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
/// keyed on `Store::latest_tx_id()`, the same pattern as SpotlightCache: under
/// polling only the first call after a write pays the scan; the rest hold the
/// store lock for one indexed MAX. Any write moves the generation and
/// invalidates naturally, so the counts are exact, never stale.
struct StatsCache {
    generation: i64,
    value: Arc<JsonValue>,
}

static STATS_CACHE: Mutex<Option<StatsCache>> = Mutex::new(None);

async fn stats(State(store): State<SharedStore>) -> Result<axum::Json<JsonValue>, AppError> {
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

async fn query(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::response::Response, AppError> {
    // Content negotiation (aegis-u7ag): an explicit standard Accept header opts
    // into the W3C SPARQL 1.1 results/RDF shape; absent / */* / application/json
    // keeps the default bespoke rows shape byte-for-byte, so existing callers are
    // unaffected.
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    blocking(move || {
        // Query TEXT at START, before taking the store lock: the query that
        // never completes — or never gets the mutex — must still be on the
        // record. Completion-only text logging is how the mfg0 killer query
        // stayed invisible for its entire ~4h burn.
        {
            let text: String = input
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("<no query field>")
                .chars()
                .take(300)
                .collect();
            eprintln!("{} query start: {text}", quipu::time::now_iso());
        }
        let store = store.lock();
        let started = std::time::Instant::now();

        let run = |store: &quipu::Store| -> Result<axum::response::Response, AppError> {
            if let Some(fmt) = quipu::w3c::negotiate(&accept) {
                let (result, _truncated) = quipu::query_result(store, &input)?;
                if let Some((content_type, body)) = quipu::w3c::serialize(store, &result, fmt)? {
                    return Ok(
                        ([(axum::http::header::CONTENT_TYPE, content_type)], body).into_response()
                    );
                }
                // Format did not fit the result variant (e.g. text/turtle for a SELECT);
                // fall through to the default shape rather than erroring.
            }

            let result = quipu::tool_query(store, &input)?;
            Ok(axum::Json(result).into_response())
        };

        let result = run(&store);
        // The request line above has method+status+duration; only a slow or
        // failed query earns its TEXT in the log — that is the one thing the
        // next wedge RCA needs and the one thing the middleware cannot
        // see (the body is consumed by then).
        let elapsed_ms = started.elapsed().as_millis();
        if elapsed_ms > 1_000 || result.is_err() {
            let text: String = input
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("<no query field>")
                .chars()
                .take(300)
                .collect();
            eprintln!("query {elapsed_ms}ms: {text}");
        }
        result
    })
    .await
}

async fn knot(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        // Hand-written write handler: same drain discipline as rw_handler!.
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
macro_rules! ro_handler {
    ($name:ident, $tool:path) => {
        async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            blocking(move || Ok(axum::Json($tool(&s.lock(), &i)?))).await
        }
    };
}

/// Finish deferred auto-embed work drained from a write handler: run the
/// multi-second ONNX `embed_batch` OUTSIDE the store lock, then
/// relock briefly to write the vectors. Entities whose text changed in the
/// unlocked window are skipped by `apply_deferred_embed` — the later writer's
/// own embed pass owns them — so interleaving readers/writers between the two
/// lock windows (the whole point of deferring) cannot regress an embedding.
fn finish_deferred_embed(s: &SharedStore, work: &quipu::DeferredEmbed) -> Result<(), AppError> {
    if work.is_empty() {
        return Ok(());
    }
    // Brief lock: clone the Arc provider, then DROP the guard.
    let provider = { s.lock().embedding_provider() };
    let Some(provider) = provider else {
        return Ok(());
    };
    let embeddings = provider.embed_batch(&work.texts())?; // CPU work, LOCK-FREE
    s.lock().apply_deferred_embed(work, &embeddings)?;
    Ok(())
}

macro_rules! rw_handler {
    ($name:ident, $tool:path) => {
        async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            blocking(move || {
                // Phase 1 under the lock: the transaction itself (plus cheap
                // embed-work collection). Phase 2 (ONNX embed) runs after the
                // guard drops; phase 3 relocks to write vectors. See
                // finish_deferred_embed.
                let (out, work) = {
                    let mut st = s.lock();
                    let out = $tool(&mut st, &i)?;
                    (out, st.take_deferred_embed())
                };
                if let Some(work) = work {
                    finish_deferred_embed(&s, &work)?;
                }
                Ok(axum::Json(out))
            })
            .await
        }
    };
}

/// A vector-search handler that embeds the query text OUTSIDE the store lock.
///
/// The ONNX query-embed is CPU-bound (tens of ms) and was run *inside* the global
/// `Store` mutex — every concurrent /search serialized across it, so a burst drained
/// in sum-of-embeds wall time, and (before store work moved to the blocking pool)
/// it parked the async workers and stalled even lock-free /health. `blocking()`
/// already keeps the work off the reactor; this additionally shrinks the lock
/// hold-time to the vector/SPARQL step so concurrent embeds run in PARALLEL.
///
/// Mechanism: take a brief lock only to clone the cheap `Arc<dyn EmbeddingProvider>`,
/// release it, run `embed_text` lock-free, then inject the vector as the
/// pre-computed `embedding` the tool already accepts — so the tool skips its own
/// in-lock embed and the second lock covers only the search. A caller-supplied
/// `embedding`, or a missing provider, is passed straight through unchanged (the
/// tool then embeds or errors exactly as before), so behaviour is identical.
macro_rules! embed_handler {
    ($name:ident, $tool:path) => {
        async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            blocking(move || {
                let mut i = i;
                if i.get("embedding").is_none() {
                    if let Some(text) = i.get("query").and_then(|v| v.as_str()).map(str::to_owned) {
                        // Brief lock: clone the Arc provider, then DROP the guard.
                        let provider = { s.lock().embedding_provider() };
                        if let Some(provider) = provider {
                            let vec = provider.embed_text(&text)?; // CPU work, LOCK-FREE
                            if let Some(obj) = i.as_object_mut() {
                                obj.insert("embedding".to_string(), serde_json::json!(vec));
                            }
                        }
                    }
                }
                Ok(axum::Json($tool(&s.lock(), &i)?))
            })
            .await
        }
    };
}

ro_handler!(cord, quipu::tool_cord);
ro_handler!(unravel, quipu::tool_unravel);
embed_handler!(search, quipu::tool_search);
embed_handler!(hybrid_search, quipu::tool_hybrid_search);
ro_handler!(unified_search, quipu::tool_unified_search);
ro_handler!(ask, quipu::tool_ask);
ro_handler!(search_nodes, quipu::tool_search_nodes);
ro_handler!(search_facts, quipu::tool_search_facts);
// /resolve: genuine read — resolve_entity scans labels and runs a
// vector search; its only compute is embedding the QUERY text (one short name,
// tens of ms — not the episode-embed class), and it commits nothing. The
// read-only claim is asserted by test, not just this comment.
ro_handler!(resolve_probe, quipu::tool_resolve_entity);
ro_handler!(
    graphiti_search_nodes,
    quipu::mcp::graphiti::tool_search_nodes
);
// aegis-e163: /shapes WRITES (tool_shapes -> store.load_shapes / remove_shapes),
// so it is rw_handler!, not the ro_handler! it was mis-registered as.
rw_handler!(shapes, quipu::tool_shapes);

// Event push P2: subscription registry (create/list/delete via action, the
// /shapes pattern — one POST route, one WRITE_ENDPOINTS entry).
rw_handler!(subscriptions, quipu::tool_subscriptions);
// Named-graph overlays (aegis-g1al / #36). aegis-e163: overlay_create WRITES the
// graphs registry (through a `&self` method — hence it was wrongly ro_handler!),
// so it is rw_handler! now. compose is a genuine read (no mutating call). write
// is a hand-written &mut handler defined below like `knot`.
rw_handler!(overlay_create, quipu::tool_overlay_create);
ro_handler!(overlay_compose, quipu::tool_overlay_compose);
ro_handler!(cooccurrence, quipu::tool_cooccurrence);
// policy_check is the one read handler with its own counter: the metric uses
// /policy/check's OWN outcome vocabulary (satisfied|unsatisfied|unknown) —
// never a block/warn tiering, which is the caller's downstream mapping.
async fn policy_check(
    State(s): State<SharedStore>,
    axum::Json(i): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let result = quipu::tool_policy_check(&s.lock(), &i)?;
    if let Some(outcome) = result.get("outcome").and_then(|v| v.as_str()) {
        quipu::metrics::metrics().observe_policy_outcome(outcome);
    }
    Ok(axum::Json(result))
}
ro_handler!(verifier_authorized, quipu::tool_verifier_authorized);
ro_handler!(verdict_verify, quipu::tool_verdict_verify);

async fn overlay_write(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        // Hand-written write handler: same drain discipline as rw_handler!.
        let (result, work) = {
            let mut st = store.lock();
            let result = quipu::tool_overlay_write(&mut st, &input)?;
            (result, st.take_deferred_embed())
        };
        if let Some(work) = work {
            finish_deferred_embed(&store, &work)?;
        }
        Ok(axum::Json(result))
    })
    .await
}
// `tool_project` is read-only by default but the `louvain` algorithm can write
// `quipu:memberOfCommunity` facts when `persist:true`, so it needs a mutable store.
rw_handler!(project_graph, quipu::tool_project);
// `/report` is read-only (god-nodes + surprising connections + suggested
// questions; hq-ct27). POST takes a JSON body of options; GET returns the
// report with defaults so a browser/skill can fetch it with no payload.
ro_handler!(report, quipu::tool_report);
async fn report_get(State(s): State<SharedStore>) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        Ok(axum::Json(quipu::tool_report(
            &s.lock(),
            &serde_json::json!({}),
        )?))
    })
    .await
}
ro_handler!(context, quipu::tool_context);

// aegis-e163: propose/accept/reject all WRITE (insert_proposal / accept_proposal
// / reject_proposal persist proposal state through `&self`), so they are
// rw_handler!, not the ro_handler! they were mis-registered as. list_proposals is
// a genuine read.
rw_handler!(propose_schema_change, quipu::tool_propose_schema_change);
ro_handler!(list_proposals, quipu::tool_list_proposals);
rw_handler!(accept_proposal, quipu::tool_accept_proposal);
rw_handler!(reject_proposal, quipu::tool_reject_proposal);

rw_handler!(episode, quipu::tool_episode);
rw_handler!(episodes_complete, quipu::tool_episodes_complete);
rw_handler!(impact_analysis, quipu::tool_impact);
rw_handler!(retract, quipu::tool_retract);
rw_handler!(set_predicate, quipu::tool_set);
rw_handler!(retract_episode, quipu::tool_retract_episode);

async fn validate(
    State(_store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    // No store lock, but SHACL validation is CPU-bound and unbounded in the size
    // of the payload, so it belongs off the reactor too.
    blocking(move || Ok(axum::Json(quipu::tool_validate(&input)?))).await
}

fn backfill_embeddings(store: &mut quipu::Store) -> std::result::Result<usize, String> {
    let provider = store
        .embedding_provider()
        .ok_or("No embedding provider configured")?;
    let result = quipu::sparql_query(store, "SELECT DISTINCT ?s WHERE { ?s ?p ?o }")
        .map_err(|e| format!("{e}"))?;
    let entity_ids: Vec<i64> = result
        .rows()
        .iter()
        .filter_map(|row| match row.get("s") {
            Some(quipu::Value::Ref(id)) => Some(*id),
            _ => None,
        })
        .collect();
    if entity_ids.is_empty() {
        return Ok(0);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();
    let mut embedded = 0;
    for chunk in entity_ids.chunks(32) {
        let pairs: Vec<(i64, String)> = chunk
            .iter()
            .filter_map(|&eid| {
                quipu::build_entity_text(store, eid)
                    .ok()
                    .filter(|t| !t.is_empty())
                    .map(|t| (eid, t))
            })
            .collect();
        if pairs.is_empty() {
            continue;
        }
        let texts: Vec<&str> = pairs.iter().map(|(_, t)| t.as_str()).collect();
        let embs = provider.embed_batch(&texts).map_err(|e| e.to_string())?;
        let vs = store.vector_store();
        for ((eid, text), emb) in pairs.iter().zip(embs.iter()) {
            vs.embed_entity(*eid, text, emb, &ts)
                .map_err(|e| e.to_string())?;
            embedded += 1;
        }
    }
    Ok(embedded)
}

async fn embed_backfill(
    State(store): State<SharedStore>,
) -> std::result::Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let mut s = store.lock();
        match backfill_embeddings(&mut s) {
            Ok(n) => Ok(axum::Json(json!({"status": "ok", "entities_embedded": n}))),
            Err(e) => Ok(axum::Json(json!({"status": "error", "error": e}))),
        }
    })
    .await
}

async fn entity_history(
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

#[derive(serde::Deserialize)]
struct TransactionParams {
    since: Option<i64>,
    limit: Option<i64>,
}

/// Query parameters for the event-log pull API (event-log P1).
#[derive(serde::Deserialize)]
struct EventParams {
    /// Return events with offset STRICTLY AFTER this. Explicit `since` wins
    /// over `consumer` so a caller can inspect any window without moving (or
    /// consulting) its durable cursor.
    since: Option<i64>,
    limit: Option<i64>,
    /// Comma-separated event types (e.g. `edge.added,type.new`).
    types: Option<String>,
    /// Filter to a single `group_id` (episode grouping, e.g. `aegis-ontology`).
    group: Option<String>,
    /// Resume from this consumer's durable committed offset.
    consumer: Option<String>,
}

/// GET /events — pull a batch of graph-change events in offset order.
/// Response: `{events, next_offset, lag, committed_offset?}`; pass
/// `next_offset` back as `since` (or POST /events/commit it) to page forward.
async fn events_get(
    State(store): State<SharedStore>,
    Query(p): Query<EventParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let store = store.lock();
        let committed: Option<i64> = match (&p.since, &p.consumer) {
            (None, Some(c)) => Some(store.consumer_committed(c)?),
            _ => None,
        };
        let since = p.since.unwrap_or_else(|| committed.unwrap_or(0));
        let limit = usize::try_from(p.limit.unwrap_or(100).clamp(1, 10_000)).unwrap_or(100);
        let types: Option<Vec<String>> = p.types.as_deref().map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        });
        let rows = store.events_after(since, limit, types.as_deref(), p.group.as_deref())?;
        // next_offset is the cursor for the NEXT call; when the batch is empty
        // it stays at `since` so polling is a fixpoint, not a rewind.
        let next_offset = rows.last().map_or(since, |r| r.offset);
        // Lag counts ALL events beyond the cursor, unfiltered — it answers
        // "how far behind the log am I", not "how many match my filter".
        let latest = store.latest_event_offset()?;
        let lag = (latest - next_offset).max(0);
        let events: Vec<JsonValue> = rows
            .iter()
            .map(quipu::store::events::EventRow::to_json)
            .collect();
        let mut body = json!({
            "events": events,
            "next_offset": next_offset,
            "lag": lag,
        });
        if let Some(c) = committed {
            body["committed_offset"] = json!(c);
        }
        Ok(axum::Json(body))
    })
    .await
}

/// POST /events/commit `{consumer_id, offset}` — durably record a consumer's
/// cursor. Any offset >= 0 is accepted, including a LOWER one (the explicit
/// replay knob; delivery is at-least-once and consumers dedup by offset).
async fn events_commit(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let consumer_id = input
            .get("consumer_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| quipu::Error::InvalidValue("consumer_id is required".into()))?
            .to_string();
        let offset = input
            .get("offset")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| quipu::Error::InvalidValue("offset (integer) is required".into()))?;
        if offset < 0 {
            return Err(quipu::Error::InvalidValue("offset must be >= 0".into()).into());
        }
        let store = store.lock();
        let now = quipu::time::now_iso();
        store.commit_consumer(&consumer_id, offset, &now)?;
        Ok(axum::Json(json!({
            "consumer_id": consumer_id,
            "committed_offset": offset,
        })))
    })
    .await
}

async fn transactions(
    State(store): State<SharedStore>,
    Query(p): Query<TransactionParams>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let store = store.lock();
        // Cursor for pollers (Shantytown's event subscription): `?since=<tx>`
        // returns only newer transactions so a watermarked poll is O(new), not
        // O(whole log). No params -> the full log, preserving prior behaviour.
        let txns = if p.since.is_none() && p.limit.is_none() {
            store.list_transactions()?
        } else {
            store.list_transactions_since(
                p.since.unwrap_or(0),
                p.limit.unwrap_or(1000).clamp(1, 10_000),
            )?
        };
        let entries: Vec<JsonValue> = txns
            .iter()
            .map(|t| {
                json!({ "id": t.id, "timestamp": t.timestamp, "actor": t.actor, "source": t.source })
            })
            .collect();
        Ok(axum::Json(
            json!({ "transactions": entries, "count": entries.len() }),
        ))
    })
    .await
}

async fn entity_conneg(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html");
    let json_ld = accept.contains("application/ld+json") || accept.contains("application/json");
    let turtle = accept.contains("text/turtle") || accept.contains("application/x-turtle");
    if !json_ld && !turtle {
        return Ok(Html(UI_HTML).into_response());
    }
    blocking(move || {
        let decoded = semweb::decode_iri(&iri);
        let store = store.lock();
        if json_ld {
            Ok(json_ld_response(semweb::entity_json_ld(&store, &decoded)?))
        } else {
            Ok(turtle_response(semweb::entity_turtle(&store, &decoded)?))
        }
    })
    .await
}

async fn entity_json(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
        let j = semweb::entity_json_ld(&store.lock(), &semweb::decode_iri(&iri))?;
        Ok(json_ld_response(j))
    })
    .await
}

async fn entity_turtle_suffix(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    blocking(move || {
        let t = semweb::entity_turtle(&store.lock(), &semweb::decode_iri(&iri))?;
        Ok(turtle_response(t))
    })
    .await
}

async fn entity_html(State(_s): State<SharedStore>, Path(_i): Path<String>) -> Html<&'static str> {
    Html(UI_HTML)
}

fn json_ld_response(j: JsonValue) -> axum::response::Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/ld+json")],
        axum::Json(j),
    )
        .into_response()
}

fn turtle_response(t: Vec<u8>) -> axum::response::Response {
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
/// the fetch; the rest hold the store lock for one indexed MAX. Any write
/// moves the generation and invalidates naturally.
struct SpotlightCache {
    generation: i64,
    entities: Arc<Vec<semweb::LabeledEntity>>,
}

static SPOTLIGHT_CACHE: Mutex<Option<SpotlightCache>> = Mutex::new(None);

async fn spotlight_handler(
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
        // Reader-starvation fix, both halves: the store lock is held only for
        // a generation check (indexed MAX) and — when the graph changed — one
        // entity fetch that refills the cache; the O(entities × text) scan
        // runs outside every lock.
        let entities = {
            let store = store.lock();
            let generation = store.latest_tx_id()?;
            let mut cache = SPOTLIGHT_CACHE.lock().unwrap();
            match cache.as_ref() {
                Some(c) if c.generation == generation => c.entities.clone(),
                _ => {
                    let fresh = Arc::new(semweb::fetch_labeled_entities(&store)?);
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
struct FragmentParams {
    subject: Option<String>,
    predicate: Option<String>,
    object: Option<String>,
    page: Option<usize>,
    #[serde(rename = "pageSize")]
    page_size: Option<usize>,
}

async fn fragments_handler(
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

async fn reconcile_handler(
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

async fn preview_handler(
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

struct AppError(quipu::Error);

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

#[cfg(test)]
mod tests {
    use super::*;
    use quipu::Store;

    /// An embedding provider whose embed is deliberately SLOW, so that whether the
    /// store lock is held across it is observable in wall-clock time.
    struct SleepyProvider;
    impl EmbeddingProvider for SleepyProvider {
        fn embed_text(&self, _text: &str) -> quipu::Result<Vec<f32>> {
            std::thread::sleep(std::time::Duration::from_millis(200));
            Ok(vec![0.1f32; 384])
        }
        fn dimension(&self) -> usize {
            384
        }
    }

    /// m4s2: the query-embed must run OUTSIDE the global `Store` mutex, so
    /// concurrent /search embeds OVERLAP instead of serializing on the lock.
    ///
    /// Deterministic stand-in for the ONNX 30/50-concurrency probe: with a
    /// provider that sleeps 200ms per embed, four concurrent searches finish in
    /// ~one embed's time when the lock is released across the embed (the fix), and
    /// in ~four embeds' time when it is not. The 600ms threshold sits between the
    /// parallel (~200-300ms) and serial (~800ms) outcomes, so this test PASSES on
    /// the embed-outside-lock handler and FAILS on the pre-fix in-lock one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_search_embeds_do_not_serialize_on_the_store_lock() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_embedding_provider(Arc::new(SleepyProvider));
        let shared: SharedStore = Arc::new(FairMutex::new(store));

        let call = || {
            let s = shared.clone();
            async move { search(State(s), axum::Json(json!({ "query": "x", "limit": 1 }))).await }
        };

        let start = std::time::Instant::now();
        let (a, b, c, d) = tokio::join!(call(), call(), call(), call());
        let elapsed = start.elapsed();

        for r in [a, b, c, d] {
            assert!(
                r.is_ok(),
                "search over an empty store should succeed with a scoped:false empty result"
            );
        }
        assert!(
            elapsed < std::time::Duration::from_millis(600),
            "four concurrent embeds serialized on the store lock ({elapsed:?}) — the embed is \
             being held inside the mutex again (m4s2 regressed)"
        );
    }

    /// Guard the fix's invariant: a caller-supplied `embedding` must pass straight
    /// through, never touching the provider — so pre-computed callers keep working
    /// and the injection path only fires for text queries.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn precomputed_embedding_bypasses_the_provider() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_embedding_provider(Arc::new(SleepyProvider)); // would sleep 200ms if consulted
        let shared: SharedStore = Arc::new(FairMutex::new(store));

        let start = std::time::Instant::now();
        let r = search(
            State(shared),
            axum::Json(json!({ "embedding": vec![0.2f32; 384], "limit": 1 })),
        )
        .await;
        assert!(r.is_ok(), "a pre-computed embedding search should succeed");
        assert!(
            start.elapsed() < std::time::Duration::from_millis(150),
            "the sleepy provider was consulted despite a pre-computed embedding"
        );
    }

    /// /stats returns the generation-keyed cache when Store::latest_tx_id()
    /// is unchanged, and recomputes when it moves (a write). Both paths are proven by
    /// poisoning STATS_CACHE at a matching vs a stale generation — matching returns the
    /// poison (so the scan was skipped), stale is ignored and the true stats recomputed —
    /// which needs no triple-insert plumbing and is fully deterministic.
    #[tokio::test]
    async fn stats_uses_generation_cache_and_invalidates_on_write() {
        let store = Store::open_in_memory().unwrap();
        let shared: SharedStore = Arc::new(FairMutex::new(store));

        // Baseline on the empty store (facts == 0), at the current generation.
        *STATS_CACHE.lock().unwrap() = None;
        let base = stats(State(shared.clone()))
            .await
            .ok()
            .expect("stats handler should succeed")
            .0;
        assert_eq!(
            base["facts"].as_u64().unwrap(),
            0,
            "empty store has 0 facts"
        );
        let cur_gen = shared.lock().latest_tx_id().unwrap();

        // Cache HIT: poison at the CURRENT generation. The handler must return the
        // poison — proving it read the cache and did NOT re-scan (a scan would give 0).
        *STATS_CACHE.lock().unwrap() = Some(StatsCache {
            generation: cur_gen,
            value: Arc::new(json!({ "facts": 42, "entities": 7, "predicates": 3 })),
        });
        let hit = stats(State(shared.clone()))
            .await
            .ok()
            .expect("stats handler should succeed")
            .0;
        assert_eq!(
            hit["facts"].as_u64().unwrap(),
            42,
            "matching generation must return the cached aggregate, not re-scan"
        );

        // INVALIDATION: poison at a STALE generation, exactly as a write leaves the
        // cache. The handler must ignore it and recompute the true (empty) stats.
        *STATS_CACHE.lock().unwrap() = Some(StatsCache {
            generation: cur_gen - 1,
            value: Arc::new(json!({ "facts": 42, "entities": 7, "predicates": 3 })),
        });
        let fresh = stats(State(shared.clone()))
            .await
            .ok()
            .expect("stats handler should succeed")
            .0;
        assert_eq!(
            fresh["facts"].as_u64().unwrap(),
            0,
            "a stale generation must be ignored and the stats recomputed"
        );
    }
}
