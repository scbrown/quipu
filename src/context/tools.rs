//! MCP tool handlers for the context pipeline.

use crate::error::Result;
use crate::store::Store;

use super::{ContextPipeline, ContextPipelineConfig};

/// MCP tool handler: `quipu_context` — query for knowledge context.
///
/// Input: `{ "query": "...", "max_entities": N, "expand_links": bool }`
/// Output: `KnowledgeContext` as JSON
pub fn tool_context(store: &Store, input: &serde_json::Value) -> Result<serde_json::Value> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| crate::error::Error::InvalidValue("missing 'query' parameter".into()))?;

    let mut config = ContextPipelineConfig::default();

    if let Some(max) = input
        .get("max_entities")
        .and_then(serde_json::Value::as_u64)
    {
        config.max_entities = max as usize;
    }
    if let Some(expand) = input
        .get("expand_links")
        .and_then(serde_json::Value::as_bool)
    {
        config.expand_links = expand;
    }

    let pipeline = ContextPipeline::new(store, config);
    let result = pipeline.query(query)?;

    Ok(serde_json::to_value(result).unwrap())
}

/// MCP tool: `quipu_unified_search` — Single endpoint for Bobbin to fetch knowledge
/// results ready for merging with code search results.
///
/// Combines text search (SPARQL FILTER) with optional semantic vector search,
/// returning results tagged with `"source": "knowledge"` and normalized scores
/// (0.0–1.0) compatible with Bobbin's code search scoring.
///
/// Input: `{ "query": "search text", "embedding": [f32...], "limit": N,
///           "expand_links": bool, "max_facts_per_entity": N }`
/// Output: `{ "results": [{ "entity", "text", "score", "source", "relevance", "types", "facts" }],
///            "count": N, "summary": { ... } }`
pub fn tool_unified_search(store: &Store, input: &serde_json::Value) -> Result<serde_json::Value> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| crate::error::Error::InvalidValue("missing 'query' parameter".into()))?;

    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10) as usize;

    let expand_links = input
        .get("expand_links")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    let max_facts = input
        .get("max_facts_per_entity")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10) as usize;

    let config = ContextPipelineConfig {
        max_entities: limit,
        max_facts_per_entity: max_facts,
        expand_links,
        link_depth: 1,
    };

    let pipeline = ContextPipeline::new(store, config);

    // If embedding provided, use hybrid search (text + vector).
    // When no explicit embedding but an EmbeddingProvider is attached,
    // auto-embed the query text for seamless natural-language search.
    let explicit_embedding: Option<Vec<f32>> = input
        .get("embedding")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect()
        });

    let embedding = match explicit_embedding {
        Some(emb) => Some(emb),
        None => store.embed_query(query)?,
    };

    let ctx = if let Some(ref emb) = embedding {
        pipeline.query_hybrid(query, emb, None)?
    } else {
        pipeline.query(query)?
    };

    let results: Vec<serde_json::Value> = ctx
        .entities
        .iter()
        .map(|e| {
            let facts: Vec<serde_json::Value> = e
                .facts
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "predicate": f.predicate,
                        "value": f.value,
                        "value_type": f.value_type
                    })
                })
                .collect();

            serde_json::json!({
                "entity": e.iri,
                "text": e.label.as_deref().unwrap_or(&e.iri),
                "score": e.score,
                "source": "knowledge",
                "relevance": e.relevance,
                "types": e.types,
                "facts": facts
            })
        })
        .collect();

    Ok(serde_json::json!({
        "results": results,
        "count": results.len(),
        "summary": {
            "total_entities": ctx.summary.total_entities,
            "total_facts": ctx.summary.total_facts,
            "direct_hits": ctx.summary.direct_hits,
            "linked_additions": ctx.summary.linked_additions
        }
    }))
}
