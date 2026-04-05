//! Quipu REST API server — HTTP interface to the knowledge graph.
//!
//! Endpoints mirror the MCP tool surface:
//!   POST /query      — SPARQL SELECT
//!   POST /knot       — Assert facts (Turtle, optional SHACL)
//!   POST /cord       — List entities
//!   POST /unravel    — Time-travel query
//!   POST /validate   — SHACL validation (dry run)
//!   POST /episode    — Structured episode ingestion
//!   POST /search     — Vector similarity search
//!   POST /`hybrid_search` — Combined SPARQL + vector search
//!   POST /`unified_search` — Unified search for Bobbin integration (code + knowledge)
//!   POST /`search_nodes` — Search entities by natural language query
//!   POST /`search_facts` — Search relationships/edges by natural language
//!   POST /search/nodes — Graphiti-compatible semantic entity search
//!   POST /episodes/complete — Graphiti-compatible flat episode ingestion
//!   POST /retract    — Retract entity facts
//!   POST /shapes     — Manage persistent SHACL shapes
//!   GET  /health     — Health check
//!   GET  /stats      — Store statistics
//!   GET  /entity/:iri — Content-negotiated entity (HTML or JSON-LD)
//!
//! Usage:
//!   quipu-server [--db <path>] [--bind <addr>]
//!   Defaults: db=quipu.db, bind=0.0.0.0:3030

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
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

    let store = quipu::Store::open(&db_path).unwrap_or_else(|e| {
        eprintln!("error opening store {db_path}: {e}");
        std::process::exit(1);
    });

    let state: SharedStore = Arc::new(Mutex::new(store));

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
        .route("/retract", post(retract))
        .route("/shapes", post(shapes))
        .route("/project", post(project_graph))
        .route("/context", post(context))
        .route("/entity/{iri}", get(entity_conneg))
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

async fn cord(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_cord(&store, &input)?;
    Ok(axum::Json(result))
}

async fn unravel(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_unravel(&store, &input)?;
    Ok(axum::Json(result))
}

async fn validate(
    State(_store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let result = quipu::tool_validate(&input)?;
    Ok(axum::Json(result))
}

async fn episode(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let mut store = store.lock().unwrap();
    let result = quipu::tool_episode(&mut store, &input)?;
    Ok(axum::Json(result))
}

async fn search(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_search(&store, &input)?;
    Ok(axum::Json(result))
}

async fn hybrid_search(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_hybrid_search(&store, &input)?;
    Ok(axum::Json(result))
}

async fn unified_search(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_unified_search(&store, &input)?;
    Ok(axum::Json(result))
}

async fn search_nodes(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_search_nodes(&store, &input)?;
    Ok(axum::Json(result))
}

async fn search_facts(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_search_facts(&store, &input)?;
    Ok(axum::Json(result))
}

async fn graphiti_search_nodes(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::mcp::graphiti::tool_search_nodes(&store, &input)?;
    Ok(axum::Json(result))
}

async fn episodes_complete(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let mut store = store.lock().unwrap();
    let result = quipu::tool_episodes_complete(&mut store, &input)?;
    Ok(axum::Json(result))
}

async fn retract(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let mut store = store.lock().unwrap();
    let result = quipu::tool_retract(&mut store, &input)?;
    Ok(axum::Json(result))
}

async fn shapes(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_shapes(&store, &input)?;
    Ok(axum::Json(result))
}

async fn project_graph(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_project(&store, &input)?;
    Ok(axum::Json(result))
}

async fn context(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::tool_context(&store, &input)?;
    Ok(axum::Json(result))
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
                        let name = store.resolve(*id).unwrap_or_else(|_| format!("{id}"));
                        json!({"@id": name})
                    }
                    quipu::Value::Str(s) => json!(s),
                    quipu::Value::Int(i) => json!(i),
                    quipu::Value::Float(f) => json!(f),
                    quipu::Value::Bool(b) => json!(b),
                    quipu::Value::Bytes(_) => json!("[binary]"),
                };

                if let Some(existing) = props.get_mut(&short_p) {
                    if let serde_json::Value::Array(arr) = existing {
                        arr.push(json_val);
                    } else {
                        let prev = existing.clone();
                        *existing = json!(vec![prev, json_val]);
                    }
                } else {
                    props.insert(short_p, json_val);
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
