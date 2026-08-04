//! Quipu REST API server — HTTP interface to the knowledge graph.
//! Usage: `quipu-server [--db <path>] [--bind <addr>]`

use std::sync::Arc;

use parking_lot::FairMutex;

use axum::{
    Router,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use quipu::EmbeddingProvider;

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
const GRAPH_CANVAS_JS: &str = include_str!("../ui/graph-canvas.js");
const DATALINKS_JS: &str = include_str!("../ui/datalinks.js");
// Vendored, not fetched: the UI must render on an air-gapped deploy, so the
// only 3D dependency ships in the binary like every other UI asset.
const THREE_JS: &str = include_str!("../ui/vendor/three.module.min.js");

// A bin crate root resolves `mod x;` to `src/x.rs`, which would collide with
// the library's modules — so each submodule names its path under `src/server/`
// explicitly.
#[path = "server/base.rs"]
mod base;
#[path = "server/entity.rs"]
mod entity;
#[cfg(test)]
#[path = "server/tests.rs"]
mod tests;
#[path = "server/tools.rs"]
mod tools;

use base::{
    components_js, datalinks_js, export, graph_canvas_js, health, knot, metrics_handler, query,
    stats, three_js, ui, version,
};
use entity::{
    entity_conneg, entity_history, entity_html, entity_json, entity_turtle_suffix, events_commit,
    events_get, fragments_handler, preview_handler, reconcile_handler, spotlight_handler,
    transactions,
};
use tools::ontology;
use tools::{
    accept_proposal, ask, context, cooccurrence, cord, embed_backfill, episode, episodes_complete,
    graph_view, graphiti_search_nodes, hybrid_search, impact_analysis, list_proposals,
    overlay_compose, overlay_create, overlay_write, policy_check, project_graph,
    propose_schema_change, reject_proposal, report, report_get, resolve_probe, retract,
    retract_episode, search, search_facts, search_nodes, set_predicate, shapes, subscriptions,
    unified_search, unravel, validate, verdict_verify, verifier_authorized,
};

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

    // Apply the OWL write-time constraint policy (aegis-bmqup). Without this
    // line the flag is UNREACHABLE: the config field, the store field and the
    // write gate can all exist and `owl.validate_on_write = true` in
    // config.toml still does nothing, because nothing carries it across. That is
    // not hypothetical — it shipped that way for the length of one test cycle,
    // and the unit tests did not catch it because they set the store's config
    // directly and never traversed this path. An end-to-end write did.
    #[cfg(feature = "owl")]
    {
        store.owl_config_mut().clone_from(&config.owl);
        if config.owl.validate_on_write {
            eprintln!(
                "OWL write-validation enabled (disjointWith + FunctionalProperty enforced on every write)"
            );
        }
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
        let outcome = {
            let mut s = state.lock();
            tools::backfill_embeddings(&mut s)
        };
        match outcome {
            Ok(count) => eprintln!("Backfill complete: {count} entities embedded"),
            // The flag is an EXPLICIT request for a capability. Serving on
            // without it produced a healthy-looking process whose semantic
            // search silently returned nothing (quipu #53) — the operator
            // asked for embeddings, so failing to provide them is fatal, not
            // a line of log noise scrolled past at boot.
            Err(e) => {
                eprintln!("error: --embed-backfill failed: {e}");
                eprintln!(
                    "refusing to start without the capability that was explicitly requested; \
                     drop --embed-backfill to start anyway"
                );
                std::process::exit(1);
            }
        }
    }

    let app = Router::new()
        // UI
        .route("/", get(ui))
        .route("/ui", get(ui))
        .route("/quipu-components.js", get(components_js))
        .route("/graph-canvas.js", get(graph_canvas_js))
        .route("/datalinks.js", get(datalinks_js))
        .route("/vendor/three.module.min.js", get(three_js))
        // Core API
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/stats", get(stats))
        .route("/metrics", get(metrics_handler))
        .route("/query", post(query))
        .route("/export", post(export))
        .route("/knot", post(knot))
        .route("/cord", post(cord))
        .route("/graph", post(graph_view))
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
        .route("/ontology", post(ontology))
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
