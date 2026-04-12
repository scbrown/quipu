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

    let mut store = quipu::Store::open(&db_path).unwrap_or_else(|e| {
        eprintln!("error opening store {db_path}: {e}");
        std::process::exit(1);
    });

    if let (Some(model_path), Some(tokenizer_path)) = (
        &config.embedding.model_path,
        &config.embedding.tokenizer_path,
    ) {
        match quipu::OnnxEmbeddingProvider::load(
            model_path,
            tokenizer_path,
            config.embedding.dimension,
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

    let state: SharedStore = Arc::new(Mutex::new(store));

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
        .route("/search_nodes", post(search_nodes))
        .route("/search_facts", post(search_facts))
        .route("/search/nodes", post(graphiti_search_nodes))
        .route("/episodes/complete", post(episodes_complete))
        .route("/impact", post(impact_analysis))
        .route("/retract", post(retract))
        .route("/shapes", post(shapes))
        .route("/project", post(project_graph))
        .route("/context", post(context))
        .route("/embed_backfill", post(embed_backfill))
        // Entity + history
        .route("/entity/{iri}", get(entity_conneg))
        .route("/entity_history", post(entity_history))
        .route("/transactions", get(transactions))
        // Semantic web APIs (Phase 4)
        .route("/spotlight", post(spotlight_handler))
        .route("/fragments", get(fragments_handler))
        .route("/reconcile", post(reconcile_handler))
        .route("/preview/{iri}", get(preview_handler))
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
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_query(&store, &input)?;
    Ok(axum::Json(result))
}

async fn knot(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let mut store = store.lock().unwrap();
    let result = quipu::tool_knot(&mut store, &input)?;
    Ok(axum::Json(result))
}

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
ro_handler!(search_nodes, quipu::tool_search_nodes);
ro_handler!(search_facts, quipu::tool_search_facts);
ro_handler!(
    graphiti_search_nodes,
    quipu::mcp::graphiti::tool_search_nodes
);
ro_handler!(shapes, quipu::tool_shapes);
ro_handler!(project_graph, quipu::tool_project);
ro_handler!(context, quipu::tool_context);

rw_handler!(episode, quipu::tool_episode);
rw_handler!(episodes_complete, quipu::tool_episodes_complete);
rw_handler!(impact_analysis, quipu::tool_impact);
rw_handler!(retract, quipu::tool_retract);

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
        .and_then(|v| v.as_f64())
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
