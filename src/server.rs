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
//!   POST /retract    — Retract entity facts
//!   POST /shapes     — Manage persistent SHACL shapes
//!   GET  /health     — Health check
//!   GET  /stats      — Store statistics
//!
//! Usage:
//!   quipu-server [--db <path>] [--bind <addr>]
//!   Defaults: db=quipu.db, bind=0.0.0.0:3030

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value as JsonValue, json};

type SharedStore = Arc<Mutex<quipu::Store>>;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let db_path = args
        .windows(2)
        .find(|w| w[0] == "--db")
        .map(|w| w[1].as_str())
        .unwrap_or("quipu.db");

    let bind_addr = args
        .windows(2)
        .find(|w| w[0] == "--bind")
        .map(|w| w[1].as_str())
        .unwrap_or("0.0.0.0:3030");

    let store = quipu::Store::open(db_path).unwrap_or_else(|e| {
        eprintln!("error opening store {db_path}: {e}");
        std::process::exit(1);
    });

    let state: SharedStore = Arc::new(Mutex::new(store));

    let app = Router::new()
        .route("/health", get(health))
        .route("/stats", get(stats))
        .route("/query", post(query))
        .route("/knot", post(knot))
        .route("/cord", post(cord))
        .route("/unravel", post(unravel))
        .route("/validate", post(validate))
        .route("/episode", post(episode))
        .route("/search", post(search))
        .route("/retract", post(retract))
        .route("/shapes", post(shapes))
        .with_state(state);

    eprintln!("quipu-server listening on {bind_addr} (db: {db_path})");

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("error binding {bind_addr}: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap();
}

// ── Handlers ───────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    axum::Json(json!({"status": "ok"}))
}

async fn stats(State(store): State<SharedStore>) -> Result<axum::Json<JsonValue>, AppError> {
    let store = store.lock().unwrap();
    let result = quipu::sparql_query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }")?;

    let mut entities = std::collections::HashSet::new();
    let mut predicates = std::collections::HashSet::new();
    for row in &result.rows {
        if let Some(quipu::Value::Ref(id)) = row.get("s") {
            entities.insert(*id);
        }
        if let Some(quipu::Value::Ref(id)) = row.get("p") {
            predicates.insert(*id);
        }
    }

    Ok(axum::Json(json!({
        "facts": result.rows.len(),
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
