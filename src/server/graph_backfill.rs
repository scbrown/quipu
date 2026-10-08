//! Bounded named-only vector preparation without a bulk producer drain.

use super::SharedStore;
use super::base::{AppError, blocking};
use axum::extract::State;
use serde_json::{Value as JsonValue, json};

/// Outcome of one bounded named-graph backfill call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphBackfillResult {
    pub(crate) embedded: usize,
    pub(crate) remaining: usize,
    pub(crate) stale_skipped: usize,
}

/// Embed up to `max_entities` entities of one registered named graph that have
/// no current vector (aegis-rcz5ib.10). Deliberately BOUNDED per call: the
/// caller paces calls and watches memory between them, because one unbounded
/// ONNX drain over thousands of entities is the aegis-tvlxr4 failure. Batches of
/// 32 are snapshotted under the lock, embedded outside it, and applied only if
/// the entity text is unchanged; a changed entity is left for the next call.
pub(crate) fn backfill_graph_embeddings(
    store: &SharedStore,
    graph_iri: &str,
    max_entities: usize,
) -> std::result::Result<GraphBackfillResult, String> {
    const BATCH_SIZE: usize = 32;
    let (provider, todo, missing_total) = {
        let s = store.lock();
        let provider = s.embedding_provider().ok_or(quipu::NO_PROVIDER_HELP)?;
        let g = s
            .registered_graph_id(graph_iri)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown graph: {graph_iri}"))?;
        let vs = s.vector_store();
        let mut missing = Vec::new();
        for eid in s.entities_in_graph(g).map_err(|e| e.to_string())? {
            // Preserve the observed ROOT corpus, including entities with ROOT
            // facts but no current vector. Filling those holes is separate
            // maintenance, not a side effect of named-graph backfill.
            if !s
                .entity_is_named_graph_only(eid)
                .map_err(|e| e.to_string())?
            {
                continue;
            }
            if vs
                .current_embedding_text(eid)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                missing.push(eid);
            }
        }
        let total = missing.len();
        missing.truncate(max_entities);
        (provider, missing, total)
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();
    let mut embedded = 0usize;
    let mut stale_skipped = 0usize;
    for ids in todo.chunks(BATCH_SIZE) {
        let snapshots: Vec<(i64, String)> = {
            let s = store.lock();
            let mut out = Vec::with_capacity(ids.len());
            for &eid in ids {
                let text = quipu::build_entity_text_for_graph_backfill(&s, eid)
                    .map_err(|e| e.to_string())?;
                if !text.is_empty() {
                    out.push((eid, text));
                }
            }
            out
        };
        if snapshots.is_empty() {
            continue;
        }
        let texts: Vec<&str> = snapshots.iter().map(|(_, text)| text.as_str()).collect();
        let embs = provider.embed_batch(&texts).map_err(|e| e.to_string())?;
        let s = store.lock();
        let vs = s.vector_store();
        for ((eid, text), emb) in snapshots.iter().zip(embs.iter()) {
            let current =
                quipu::build_entity_text_for_graph_backfill(&s, *eid).map_err(|e| e.to_string())?;
            if current != *text
                || vs
                    .current_embedding_text(*eid)
                    .map_err(|e| e.to_string())?
                    .is_some()
            {
                stale_skipped += 1;
                continue;
            }
            if !s
                .entity_is_named_graph_only(*eid)
                .map_err(|e| e.to_string())?
            {
                stale_skipped += 1;
                continue;
            }
            s.mark_named_search_embedding(*eid)
                .map_err(|e| e.to_string())?;
            vs.embed_entity(*eid, text, emb, &ts)
                .map_err(|e| e.to_string())?;
            embedded += 1;
        }
    }
    Ok(GraphBackfillResult {
        embedded,
        remaining: missing_total.saturating_sub(embedded),
        stale_skipped,
    })
}

pub(crate) async fn embed_backfill_graph(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> std::result::Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let graph = input
            .get("graph")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                quipu::Error::InvalidValue("graph (named-graph IRI) is required".into())
            })?
            .to_string();
        let max_entities = input
            .get("max_entities")
            .and_then(JsonValue::as_u64)
            .map_or(256, |n| usize::try_from(n).unwrap_or(usize::MAX))
            .clamp(1, 2000);
        match backfill_graph_embeddings(&store, &graph, max_entities) {
            Ok(outcome) => Ok(axum::Json(json!({
                "status": "ok",
                "graph": graph,
                "max_entities": max_entities,
                "entities_embedded": outcome.embedded,
                "remaining": outcome.remaining,
                "stale_skipped": outcome.stale_skipped,
            }))),
            Err(e) => Err(quipu::Error::InvalidValue(e).into()),
        }
    })
    .await
}
