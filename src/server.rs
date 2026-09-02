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

// A bin crate root resolves `mod x;` to `src/x.rs`, which would collide with
// the library's modules — so each submodule names its path under `src/server/`
// explicitly.
#[path = "server/admission.rs"]
mod admission;
#[path = "server/assets.rs"]
mod assets;
#[path = "server/base.rs"]
mod base;
#[path = "server/entity.rs"]
mod entity;
#[path = "server/graph_store.rs"]
mod graph_store;
#[path = "server/handle.rs"]
mod handle;
#[path = "server/query_usage.rs"]
mod query_usage;
#[path = "server/reason.rs"]
mod reason;
#[path = "server/request_middleware.rs"]
mod request_middleware;
#[path = "server/service_description.rs"]
mod service_description;
#[path = "server/snapshot_upload.rs"]
mod snapshot_upload;
#[cfg(test)]
#[path = "server/tests.rs"]
mod tests;
#[path = "server/tools.rs"]
mod tools;

use assets::{components_js, datalinks_js, graph_canvas_js, three_js, ui};
use base::{export, health, metrics_handler, print_usage, query, share_payload, stats, version};
use entity::{
    changes_get, entity_conneg, entity_history, entity_html, entity_json, entity_query_conneg,
    entity_turtle_suffix, events_commit, events_get, fragments_handler, preview_handler,
    reconcile_handler, spotlight_handler, transactions,
};
pub(crate) use handle::{ReadPool, SharedStore, StoreHandle};
use reason::{explain, reason, shapes};
use tools::*;

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

    let mut store =
        quipu::open_with_configured_attachments(&db_path, &config).unwrap_or_else(|e| {
            eprintln!("error opening store {db_path}: {e}");
            std::process::exit(1);
        });

    // quipu-lv7: `vector.backend` selects the store's vector backend in-binary.
    // It refuses when this build cannot construct the configured one rather
    // than falling back to the SQLite table a migrated deployment has left.
    base::apply_vector_backend(&mut store, &config);

    // quipu-at2: announce what the `[[quipu.attachments]]` declarations became.
    // A composed layer that nobody can see is how "it returned no rows" becomes
    // a mystery; a missing file already refused the open above.
    base::report_attachments(&store);

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
    // quipu #68: the floors and their consumer land together — a settable knob
    // that nothing reads is the bug config.rs guards against.
    store.labels_config_mut().clone_from(&config.label_floors);

    // quipu #47: report the configured federation remotes at startup, and prove
    // they are REACHED rather than merely parsed. `federation.remotes` was
    // parsed, exported, and consumed by nothing for months; announcing what was
    // configured — and whether each one answers — is what makes the difference
    // visible without waiting for a federated query to be issued.
    if !config.federation.remotes.is_empty() {
        base::report_federation(&store, &config.federation);
    }

    // Apply the SHACL validation policy so episode writes can be gated against
    // persistently-loaded shapes, not just episode-inline shapes (hq-c6s).
    store.shacl_config_mut().clone_from(&config.shacl);
    if config.shacl.validate_on_write {
        eprintln!("SHACL write-validation enabled (loaded shapes enforced on every write)");
    }

    // Apply the OWL write-time constraint policy and opt-in reactive
    // materialization (aegis-bmqup, quipu-923) — see base::apply_owl for why
    // the clone_from is load-bearing.
    base::apply_owl(&mut store, &config);

    // Register the reactive reasoner so DERIVED facts stay fresh on write
    // (aegis-nnf0h). It was registered at src/cli.rs:582 and NOWHERE else, so the
    // server — the only writer that matters, since every agent ingests through it
    // — ran with no incremental derivation at all.
    //
    // Rules come from the SHAPES table: `shapes/aegis-rules.ttl` documents that
    // rules may be stored alongside SHACL shapes, and the parser only picks up
    // `a rule:Rule` subjects, so the two vocabularies coexist and no new storage
    // or route is needed. A deployment with no rules loaded is the normal case and
    // stays silent.
    //
    // SCOPE, stated because the bead's original framing over-promised: this makes
    // DATALOG derivation incremental. `ReactiveReasoner::new` takes a `RuleSet` and
    // there is no OWL path in it, so OWL materialization is re-run on ontology
    // load only. Entailments that must stay live are therefore better
    // expressed as rules — an `owl:inverseOf` is exactly a one-atom projection,
    // `hosts(?y, ?x) :- runs_on(?x, ?y)`.
    //
    // Registered UNCONDITIONALLY (quipu-923, gap G6), rules or none: the
    // startup ruleset used to be a snapshot, so rules loaded through /shapes
    // needed a restart. Now /shapes hot-swaps the ruleset through the Arc kept
    // on StoreHandle, and an empty reasoner is a per-write no-op.
    #[cfg(feature = "reactive-reasoner")]
    let reactive_reasoner = {
        let empty = || quipu::reasoner::RuleSet::empty(quipu::namespace::DEFAULT_BASE_NS);
        // A malformed ruleset must not take the server down, but it must not
        // pass unremarked either: the failure mode being avoided is a reasoner
        // that silently does nothing.
        let ruleset = match store.get_combined_shapes() {
            Ok(Some(ttl)) => match quipu::reasoner::parse_rules(&ttl, None) {
                Ok(rs) => rs,
                Err(e) => {
                    eprintln!("reactive reasoner starts EMPTY — rules failed to parse: {e}");
                    empty()
                }
            },
            Ok(None) => empty(),
            Err(e) => {
                eprintln!("reactive reasoner starts EMPTY — could not read shapes: {e}");
                empty()
            }
        };
        let n = ruleset.len();
        let reasoner = Arc::new(quipu::ReactiveReasoner::new(ruleset));
        store.add_observer(reasoner.clone());
        if n > 0 {
            eprintln!(
                "reactive reasoner registered — {n} Datalog rule(s) re-derive on every write"
            );
        } else {
            eprintln!("reactive reasoner registered (0 rules) — /shapes loads take effect live");
        }
        Some(reasoner)
    };

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

    // Build the read pool LAST, from the fully-configured writer.
    // Order is load-bearing: `adopt_read_config_from` copies the writer's
    // read-relevant policy, so every setter above must already have run. A pool
    // built earlier would serve reads under default search guardrails and a
    // default base namespace, and answer the same question differently
    // depending on which connection happened to take it.
    //
    // An in-memory store is deliberately NOT pooled: each `:memory:` connection
    // is a separate empty database, so a pool there would serve reads from
    // nothing. Same for a store we cannot open read-only — announce it and run
    // serialised rather than fail to boot, since a slow server is recoverable
    // and a dead one is not.
    let read_pool = if db_path == ":memory:" {
        ReadPool::empty()
    } else if store.has_vector_delegate() || store.has_local_vector_backend() {
        // FAIL SAFE, not fail silent. The vector backends are boxed trait
        // objects and are not `Clone`, so `adopt_read_config_from` cannot carry
        // them onto a pooled reader — and `unified_search`, `ask`,
        // `search_nodes` and `search_facts` are all pooled AND vector-backed.
        // A pool built here would answer those from the built-in SQLite vectors
        // table while the writer answered from the configured backend: same
        // question, two answers, no error, decided by which connection happened
        // to take the request.
        //
        // Not reachable in this deployment — the REST server configures neither
        // backend today (`lancedb: false`) — which is exactly why it needs to be
        // a guard rather than a note. Whoever turns LanceDB on will not be
        // thinking about the read pool.
        eprintln!(
            "read pool: DISABLED — a vector delegate/local backend is configured and \
             cannot be shared with read-only connections. Serving reads from the writer \
             rather than answering vector search two different ways."
        );
        ReadPool::empty()
    } else {
        let want = config.server.read_pool_size;
        let mut conns = Vec::with_capacity(want);
        for i in 0..want {
            match quipu::Store::open_read_only(&db_path) {
                Ok(mut r) => {
                    r.adopt_read_config_from(&store);
                    conns.push(FairMutex::new(r));
                }
                Err(e) => {
                    eprintln!(
                        "warning: read connection {i} of {want} failed to open ({e}) — \
                         continuing with {i}; reads fall back to the writer when the pool is empty"
                    );
                    break;
                }
            }
        }
        ReadPool {
            conns,
            next: std::sync::atomic::AtomicUsize::new(0),
        }
    };
    if read_pool.len() > 0 {
        eprintln!("read pool: {} read-only connections", read_pool.len());
    } else {
        eprintln!("read pool: DISABLED — every read serialises behind the writer lock");
    }

    let vector_reads_pooled = store.has_sqlite_vector_backend();
    let state: SharedStore = Arc::new(StoreHandle {
        writer: FairMutex::new(store),
        readers: read_pool,
        vector_reads_pooled,
        federation: config.federation.clone(),
        #[cfg(feature = "reactive-reasoner")]
        reasoner: reactive_reasoner,
    });
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
        let outcome = tools::backfill_embeddings(&state);
        match outcome {
            Ok(outcome) => eprintln!(
                "Backfill complete: {} entities embedded, {} stale snapshots retried",
                outcome.embedded, outcome.stale_retries
            ),
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
        .route("/.well-known/void", get(service_description::service_description))
        .route("/metrics", get(metrics_handler))
        .route("/query", post(query))
        .route("/export", post(export))
        .merge(graph_store::routes())
        .route("/share", post(share_payload))
        .merge(snapshot_upload::routes())
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
        .route("/path/cone", post(path_cone))
        .route("/path/backtest", post(path_backtest))
        .route("/retract", post(retract))
        .route("/set", post(set_predicate))
        .route("/episode/retract", post(retract_episode))
        .route("/shapes", post(shapes))
        .route("/reason", post(reason))
        .route("/explain", post(explain))
        .route("/ontology", post(ontology))
        .route("/subscriptions", post(subscriptions))
        .route("/datasets", post(datasets))
        .route("/queries", post(queries))
        .route("/propose", post(propose_schema_change))
        .route("/proposals", post(list_proposals))
        .route("/proposal/accept", post(accept_proposal))
        .route("/proposal/reject", post(reject_proposal))
        // camayoc-s0h: the registration and labelling primitives existed in
        // the store with no way in from outside. Routing into a named graph
        // without being able to LABEL it yields separate graphs every query
        // still reads at equal trust, so these ship together.
        .route("/graphs", get(graphs_list))
        .route("/graph/create", post(graph_create))
        .route("/graph/label", post(graph_label))
        .route("/graph/freeze", post(graph_freeze))
        .route("/graph/thaw", post(graph_thaw))
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
        .route("/entity", get(entity_query_conneg))
        .route("/entity/{iri}", get(entity_conneg))
        .route("/entity/{iri}/json", get(entity_json))
        .route("/entity/{iri}/ttl", get(entity_turtle_suffix))
        .route("/entity/{iri}/html", get(entity_html))
        .route("/entity_history", post(entity_history))
        .route("/transactions", get(transactions))
        // Event log pull API (event-log P1)
        .route("/changes", get(changes_get))
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
                    let path = req.uri().path().to_string();
                    let is_write = quipu::http_auth::is_write_request(&path, req.method().as_str());
                    let auth_header = req
                        .headers()
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    let decision = quipu::http_auth::authorize(
                        is_write,
                        read_only,
                        auth_token.as_deref(),
                        auth_header.as_deref(),
                    );
                    match decision {
                        quipu::http_auth::AccessDecision::Allow => {
                            let mut req = req;
                            if is_write && auth_token.is_some() {
                                req.extensions_mut().insert(
                                    quipu::http_auth::AuthenticatedPrincipal::LEGACY_SHARED_BEARER,
                                );
                            }
                            let mut response = next.run(req).await;
                            let outcome = if !is_write {
                                quipu::request_usage::AuthOutcome::NotRequired
                            } else if auth_token.is_some() {
                                quipu::request_usage::AuthOutcome::Authenticated
                            } else {
                                quipu::request_usage::AuthOutcome::OpenWrite
                            };
                            response.extensions_mut().insert(outcome);
                            response
                        }
                        // BOTH refusals carry a JSON body, and that is the whole
                        // point of aegis-zodg0. `StatusCode::into_response()`
                        // yields a bare status with a ZERO-LENGTH body, so
                        // `curl -s` prints NOTHING and exits 0 — a refusal that
                        // is indistinguishable from "the algorithm returned no
                        // results" or "the graph is empty". Measured on /project:
                        // it took two round trips to discover it was auth at all.
                        //
                        // Same failure SHAPE this repo already records for the
                        // bobbin /search 405 (CLAUDE.md): a documented recipe that
                        // silently returns nothing, so the reader concludes the
                        // service is empty rather than that they called it wrong.
                        // A refusal MUST say it refused; an empty body cannot.
                        //
                        // Fixed HERE, in the middleware, rather than per-route:
                        // every bearer-gated endpoint shared the defect. /shapes
                        // returned 0 bytes on 401 too, so there was no correct
                        // per-route body to copy — the bug was never /project's.
                        quipu::http_auth::AccessDecision::Unauthorized => {
                            // The /project note is PATH-SCOPED. Emitting it on
                            // every gated route put an explanation of `louvain`
                            // on a /shapes 401 — a message that is accurate about
                            // some other endpoint is its own small version of the
                            // defect this whole change is about, so it is
                            // conditional rather than convenient.
                            let why = if path == "/project" {
                                " /project looks read-only and is not: `louvain` with persist:true \
                                 WRITES quipu:memberOfCommunity, so the whole route is gated even \
                                 though its other algorithms only read."
                            } else {
                                ""
                            };
                            let mut response = (
                                StatusCode::UNAUTHORIZED,
                                axum::Json(serde_json::json!({
                                    "error": format!(
                                        "unauthorized: {path} is a WRITE endpoint and requires a bearer \
                                         token. Send `Authorization: Bearer <token>`. Read endpoints \
                                         (/query, /search, entity reads, /health) are open and need no \
                                         credential.{why}"
                                    ),
                                    "endpoint": path,
                                    "reason": "missing_or_invalid_bearer_token",
                                })),
                            )
                                .into_response();
                            response.extensions_mut().insert(
                                quipu::request_usage::AuthOutcome::Unauthorized,
                            );
                            response
                        }
                        quipu::http_auth::AccessDecision::ReadOnly => {
                            let mut response = (
                                StatusCode::FORBIDDEN,
                                axum::Json(serde_json::json!({
                                    "error": format!(
                                        "read-only mode: {path} is a WRITE endpoint and this server was \
                                         started read-only, so no credential will authorize it. Restart \
                                         without read-only to permit writes."
                                    ),
                                    "endpoint": path,
                                    "reason": "server_is_read_only",
                                })),
                            )
                                .into_response();
                            response
                                .extensions_mut()
                                .insert(quipu::request_usage::AuthOutcome::ReadOnly);
                            response
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
        .layer(axum::middleware::from_fn(request_middleware::log_request))
        .with_state(state);

    // Event push P2: the delivery worker. A 2s tick over deliver_tick with the
    // real poster; each tick runs under spawn_blocking (SQLite + ureq are
    // sync). Cursor semantics make every tick idempotent, so the loop needs no
    // state of its own and a missed tick delays, never loses.
    {
        let push_store = push_store_outer.clone();
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

    // Event-log retention (quipu-9z9). Opt-in via `[quipu.events]
    // retention_days`; unset keeps today's keep-forever behaviour and spawns
    // nothing. An hourly cadence (first tick immediate) is plenty: the policy
    // is measured in days, and `prune_events` never touches an offset a
    // registered consumer has not committed past, so an aggressive cadence
    // could not break replay anyway — it would only burn cycles.
    if let Some(days) = config.events.retention_days {
        let prune_store = push_store_outer;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                tick.tick().await;
                let store = prune_store.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let cutoff = quipu::time::iso_days_ago(u64::from(days));
                    let store = store.lock();
                    match store.prune_events(&cutoff) {
                        Ok(0) => {}
                        Ok(n) => eprintln!(
                            "events: retention pruned {n} event(s) older than {cutoff} ({days}d)"
                        ),
                        Err(e) => eprintln!("events: retention prune failed: {e}"),
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

    // AFTER the bind succeeds, so the recorded start time is when this process
    // began SERVING, not when it began trying. A failed bind exits above; a
    // start time recorded before it would describe a process that never served.
    quipu::metrics::init_start_time();

    axum::serve(listener, app).await.unwrap();
}
