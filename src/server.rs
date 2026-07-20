//! Quipu REST API server — HTTP interface to the knowledge graph.
//! Usage: `quipu-server [--db <path>] [--bind <addr>]`

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use quipu::{EmbeddingProvider, semweb};
use serde_json::{Value as JsonValue, json};

type SharedStore = Arc<Mutex<quipu::Store>>;

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
                eprintln!(
                    "ONNX embedding provider loaded (dim={dim}, auto_embed={})",
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
    let signing_key_path = std::env::var("QUIPU_SIGNING_KEY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new(".quipu").join("verifier.pk8"));
    match quipu::signing::SigningIdentity::load(&signing_key_path, "quipu") {
        Ok(id) => {
            eprintln!(
                "verdict signing enabled (verifier=quipu, key={})",
                signing_key_path.display()
            );
            eprintln!("  register this public key to trust its verdicts: {}", id.public_key_hex());
            store.set_signing_identity(Arc::new(id));
        }
        Err(e) => eprintln!("warning: verdict signing disabled -- {e}"),
    }

    let state: SharedStore = Arc::new(Mutex::new(store));

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
        let mut s = state.lock().unwrap();
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
        .route("/query", post(query))
        .route("/knot", post(knot))
        .route("/cord", post(cord))
        .route("/unravel", post(unravel))
        .route("/validate", post(validate))
        .route("/episode", post(episode))
        .route("/search", post(search))
        .route("/hybrid_search", post(hybrid_search))
        .route("/unified_search", post(unified_search))
        .route("/ask", post(ask))
        .route("/search_nodes", post(search_nodes))
        .route("/search_facts", post(search_facts))
        .route("/search/nodes", post(graphiti_search_nodes))
        .route("/episodes/complete", post(episodes_complete))
        .route("/impact", post(impact_analysis))
        .route("/retract", post(retract))
        .route("/episode/retract", post(retract_episode))
        .route("/shapes", post(shapes))
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
        .with_state(state);

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

async fn stats(State(store): State<SharedStore>) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
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

    Ok(axum::Json(json!({
        "facts": result.rows().len(),
        "entities": entities.len(),
        "predicates": predicates.len()
    })))
}

async fn query(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::response::Response, AppError> {
    let store = store.lock().unwrap();

    // Content negotiation (aegis-u7ag): an explicit standard Accept header opts
    // into the W3C SPARQL 1.1 results/RDF shape; absent / */* / application/json
    // keeps the default bespoke rows shape byte-for-byte, so existing callers are
    // unaffected.
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(fmt) = quipu::w3c::negotiate(accept) {
        let (result, _truncated) = quipu::query_result(&store, &input)?;
        if let Some((content_type, body)) = quipu::w3c::serialize(&store, &result, fmt)? {
            return Ok((
                [(axum::http::header::CONTENT_TYPE, content_type)],
                body,
            )
                .into_response());
        }
        // Format did not fit the result variant (e.g. text/turtle for a SELECT);
        // fall through to the default shape rather than erroring.
    }

    let result = quipu::tool_query(&store, &input)?;
    Ok(axum::Json(result).into_response())
}

async fn knot(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let mut store = store.lock().unwrap();
    let result = quipu::tool_knot(&mut store, &input)?;
    Ok(axum::Json(result))
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
            Ok(axum::Json($tool(&s.lock().unwrap(), &i)?))
        }
    };
}

macro_rules! rw_handler {
    ($name:ident, $tool:path) => {
        async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            Ok(axum::Json($tool(&mut s.lock().unwrap(), &i)?))
        }
    };
}

ro_handler!(cord, quipu::tool_cord);
ro_handler!(unravel, quipu::tool_unravel);
ro_handler!(search, quipu::tool_search);
ro_handler!(hybrid_search, quipu::tool_hybrid_search);
ro_handler!(unified_search, quipu::tool_unified_search);
ro_handler!(ask, quipu::tool_ask);
ro_handler!(search_nodes, quipu::tool_search_nodes);
ro_handler!(search_facts, quipu::tool_search_facts);
ro_handler!(
    graphiti_search_nodes,
    quipu::mcp::graphiti::tool_search_nodes
);
// aegis-e163: /shapes WRITES (tool_shapes -> store.load_shapes / remove_shapes),
// so it is rw_handler!, not the ro_handler! it was mis-registered as.
rw_handler!(shapes, quipu::tool_shapes);
// Named-graph overlays (aegis-g1al / #36). aegis-e163: overlay_create WRITES the
// graphs registry (through a `&self` method — hence it was wrongly ro_handler!),
// so it is rw_handler! now. compose is a genuine read (no mutating call). write
// is a hand-written &mut handler defined below like `knot`.
rw_handler!(overlay_create, quipu::tool_overlay_create);
ro_handler!(overlay_compose, quipu::tool_overlay_compose);
ro_handler!(cooccurrence, quipu::tool_cooccurrence);
ro_handler!(policy_check, quipu::tool_policy_check);
ro_handler!(verifier_authorized, quipu::tool_verifier_authorized);
ro_handler!(verdict_verify, quipu::tool_verdict_verify);

async fn overlay_write(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let mut store = store.lock().unwrap();
    let result = quipu::tool_overlay_write(&mut store, &input)?;
    Ok(axum::Json(result))
}
// `tool_project` is read-only by default but the `louvain` algorithm can write
// `quipu:memberOfCommunity` facts when `persist:true`, so it needs a mutable store.
rw_handler!(project_graph, quipu::tool_project);
// `/report` is read-only (god-nodes + surprising connections + suggested
// questions; hq-ct27). POST takes a JSON body of options; GET returns the
// report with defaults so a browser/skill can fetch it with no payload.
ro_handler!(report, quipu::tool_report);
async fn report_get(State(s): State<SharedStore>) -> Result<axum::Json<JsonValue>, AppError> {
    Ok(axum::Json(quipu::tool_report(
        &s.lock().unwrap(),
        &serde_json::json!({}),
    )?))
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
rw_handler!(retract_episode, quipu::tool_retract_episode);

async fn validate(
    State(_store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    Ok(axum::Json(quipu::tool_validate(&input)?))
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
    let mut s = store.lock().unwrap();
    match backfill_embeddings(&mut s) {
        Ok(n) => Ok(axum::Json(json!({"status": "ok", "entities_embedded": n}))),
        Err(e) => Ok(axum::Json(json!({"status": "error", "error": e}))),
    }
}

async fn entity_history(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let iri = input
        .get("iri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| quipu::Error::InvalidValue("missing 'iri' parameter".into()))?;
    let store = store.lock().unwrap();
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
}

async fn transactions(State(store): State<SharedStore>) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let entries: Vec<JsonValue> = store
        .list_transactions()?
        .iter()
        .map(|t| {
            json!({ "id": t.id, "timestamp": t.timestamp, "actor": t.actor, "source": t.source })
        })
        .collect();
    Ok(axum::Json(
        json!({ "transactions": entries, "count": entries.len() }),
    ))
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
    let decoded = semweb::decode_iri(&iri);
    if accept.contains("application/ld+json") || accept.contains("application/json") {
        let j = semweb::entity_json_ld(&store.lock().unwrap(), &decoded)?;
        Ok(json_ld_response(j))
    } else if accept.contains("text/turtle") || accept.contains("application/x-turtle") {
        let t = semweb::entity_turtle(&store.lock().unwrap(), &decoded)?;
        Ok(turtle_response(t))
    } else {
        Ok(Html(UI_HTML).into_response())
    }
}

async fn entity_json(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let j = semweb::entity_json_ld(&store.lock().unwrap(), &semweb::decode_iri(&iri))?;
    Ok(json_ld_response(j))
}

async fn entity_turtle_suffix(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let t = semweb::entity_turtle(&store.lock().unwrap(), &semweb::decode_iri(&iri))?;
    Ok(turtle_response(t))
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

async fn spotlight_handler(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let text = input
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| quipu::Error::InvalidValue("missing 'text' parameter".into()))?;
    let confidence = input
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.5);
    let store = store.lock().unwrap();
    Ok(axum::Json(semweb::spotlight(&store, text, confidence)?))
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
    let result = semweb::fragments(&store.lock().unwrap(), &q)?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=60"),
        ],
        axum::Json(result),
    )
        .into_response())
}

async fn reconcile_handler(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    if input.get("queries").is_none() {
        return Ok(axum::Json(semweb::reconcile_manifest()));
    }
    let queries = input
        .get("queries")
        .and_then(|v| v.as_object())
        .ok_or_else(|| quipu::Error::InvalidValue("'queries' must be an object".into()))?;
    let store = store.lock().unwrap();
    Ok(axum::Json(semweb::reconcile(&store, queries)?))
}

async fn preview_handler(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let html = semweb::preview_card(&store.lock().unwrap(), &semweb::decode_iri(&iri))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response())
}

struct AppError(quipu::Error);

impl From<quipu::Error> for AppError {
    fn from(e: quipu::Error) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = json!({ "error": self.0.to_string() });
        (StatusCode::BAD_REQUEST, axum::Json(body)).into_response()
    }
}
