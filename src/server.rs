//! Quipu REST API server — HTTP interface to the knowledge graph.
//! Endpoints mirror the MCP tool surface. Usage: `quipu-server [--db <path>] [--bind <addr>]`

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use quipu::EmbeddingProvider;
use serde_json::{Value as JsonValue, json};

type SharedStore = Arc<Mutex<quipu::Store>>;

const UI_HTML: &str = include_str!("../ui/index.html");

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

    // Load config from .bobbin/config.toml, then apply CLI overrides.
    let config = quipu::QuipuConfig::load(std::path::Path::new("."))
        .with_db_override(db_flag)
        .with_bind_override(bind_flag);

    let db_path = config.store_path.to_string_lossy().to_string();
    let bind_addr = config.server.bind.clone();

    let mut store = quipu::Store::open(&db_path).unwrap_or_else(|e| {
        eprintln!("error opening store {db_path}: {e}");
        std::process::exit(1);
    });

    // Initialize ONNX embedding provider if configured.
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

    // Run one-shot embedding backfill if requested (before serving).
    if args.iter().any(|a| a == "--embed-backfill") {
        eprintln!("Running embedding backfill for all entities...");
        let mut s = state.lock().unwrap();
        match backfill_embeddings(&mut s) {
            Ok(count) => eprintln!("Backfill complete: {count} entities embedded"),
            Err(e) => eprintln!("Backfill error: {e}"),
        }
    }

    let app = Router::new()
        .route("/", get(ui))
        .route("/ui", get(ui))
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
        .route("/entity/{iri}", get(entity_conneg))
        .route("/entity_history", post(entity_history))
        .route("/transactions", get(transactions))
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

// ── Handlers ───────────────────────────────────────────────────────────

async fn ui() -> Html<&'static str> {
    Html(UI_HTML)
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

// Read-only tool handlers (shared store, JSON in/out)
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

// Mutable tool handlers
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

// ── Embedding backfill ────────────────────────────────────────────

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

// ── Entity history + transaction listing ──────────────────────────

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
    let entries: Vec<JsonValue> = store.list_transactions()?.iter().map(|t| {
        json!({ "id": t.id, "timestamp": t.timestamp, "actor": t.actor, "source": t.source })
    }).collect();
    Ok(axum::Json(
        json!({ "transactions": entries, "count": entries.len() }),
    ))
}

// ── Content negotiation for entity URLs ───────────────────────────

async fn entity_conneg(
    State(store): State<SharedStore>,
    Path(iri): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html");

    if accept.contains("application/ld+json") || accept.contains("application/json") {
        // Return JSON-LD
        let store = store.lock().unwrap();
        let decoded_iri = iri.replace("%23", "#").replace("%20", " ");
        let sparql = format!("SELECT ?p ?o WHERE {{ <{decoded_iri}> ?p ?o }}",);
        let result = quipu::sparql_query(&store, &sparql)?;

        let mut props = serde_json::Map::new();
        props.insert("@context".to_string(), json!("https://schema.org"));
        props.insert("@id".to_string(), json!(decoded_iri));

        for row in result.rows() {
            if let (Some(quipu::Value::Ref(p_id)), Some(val)) = (row.get("p"), row.get("o")) {
                let p_name = store.resolve(*p_id).unwrap_or_else(|_| format!("{p_id}"));
                let short_p = short_name_server(&p_name);
                let json_val = match val {
                    quipu::Value::Ref(id) => {
                        let n = store.resolve(*id).unwrap_or_else(|_| format!("{id}"));
                        json!({"@id": n})
                    }
                    quipu::Value::Str(s) => json!(s),
                    quipu::Value::Int(i) => json!(i),
                    quipu::Value::Float(f) => json!(f),
                    quipu::Value::Bool(b) => json!(b),
                    quipu::Value::Bytes(_) => json!("[binary]"),
                };
                match props.get_mut(&short_p) {
                    Some(serde_json::Value::Array(arr)) => arr.push(json_val),
                    Some(existing) => {
                        let prev = existing.clone();
                        *existing = json!(vec![prev, json_val]);
                    }
                    None => {
                        props.insert(short_p, json_val);
                    }
                }
            }
        }

        Ok((
            [(axum::http::header::CONTENT_TYPE, "application/ld+json")],
            axum::Json(serde_json::Value::Object(props)),
        )
            .into_response())
    } else {
        // Return HTML (the SPA handles client-side routing)
        Ok(Html(UI_HTML).into_response())
    }
}

fn short_name_server(iri: &str) -> String {
    let iri = iri.trim_start_matches('<').trim_end_matches('>');
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}

// ── Error handling ─────────────────────────────────────────────────

struct AppError(quipu::Error);

impl From<quipu::Error> for AppError {
    fn from(e: quipu::Error) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = json!({
            "error": self.0.to_string()
        });
        (StatusCode::BAD_REQUEST, axum::Json(body)).into_response()
    }
}
