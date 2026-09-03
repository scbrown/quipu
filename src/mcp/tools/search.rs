//! Vector and hybrid search: `quipu_search`, `quipu_hybrid_search`, and the
//! group/type scoping that feeds them.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::sparql;
use crate::store::Store;
use crate::types::Value;
use crate::vector::KnowledgeVectorStore;

/// MCP tool: `quipu_search` -- Semantic vector search over entity embeddings.
///
/// Accepts either a pre-computed `embedding` vector or a natural-language
/// `query` string. When `query` is provided and no `embedding`, the store's
/// `EmbeddingProvider` is used to embed the text automatically.
pub fn tool_search(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let explicit_embedding: Option<Vec<f32>> = input
        .get("embedding")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect()
        });

    let query_text = input.get("query").and_then(|v| v.as_str());

    let embedding = match (explicit_embedding, query_text) {
        (Some(emb), _) => emb,
        (None, Some(text)) => store.embed_query(text)?.ok_or_else(|| {
            Error::InvalidValue(
                "no embedding provider configured; supply a pre-computed 'embedding' or \
                     attach an EmbeddingProvider to the store"
                    .into(),
            )
        })?,
        (None, None) => {
            return Err(Error::InvalidValue(
                "missing 'embedding' array or 'query' text parameter".into(),
            ));
        }
    };

    let limit = store
        .search_config()
        .clamp_limit(input.get("limit").and_then(serde_json::Value::as_u64));

    let valid_at = input.get("valid_at").and_then(|v| v.as_str());
    let verbose = input
        .get("verbose")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let prefixes = (!verbose)
        .then(|| crate::compact::PrefixMap::from_store(store))
        .transpose()?;

    // Provenance scoping, NOT isolation (hq-93d; design: docs/design/group-isolation.md).
    // A plain vector search returns matches from every group/type. When the caller
    // scopes with `group_ids` and/or `entity_type`, narrow to entities in that scope
    // — a best-effort PROVENANCE filter, not a tenant boundary: it is optional and
    // caller-supplied (nothing forces a scope), and `/knot` facts (no episode) are
    // DROPPED from a group scope because they trace back to no activity. Do not build
    // an access decision on it; true isolation is deferred (keeper gate hq-2u3).
    let entity_type = input.get("entity_type").and_then(|v| v.as_str());
    let group_ids: Option<Vec<&str>> = input
        .get("group_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    let scope = scoped_entity_iris(store, entity_type, group_ids.as_deref())?;

    // Oversample in both paths: an entity has one embedding row per fact/text,
    // so the raw top-N can be several rows of the same entity (aegis-a1s5).
    // Fetching extra candidates leaves room to dedupe down to `limit` entities.
    let oversampled = store.search_config().oversample(limit);

    let matches = if let Some(ref allowed) = scope {
        // Keep only in-scope entities (works for both the SQLite backend and as
        // a safety net over LanceDB pushdown). entity_type is also pushed down
        // to LanceDB for efficiency.
        let pushdown = entity_type.map(|t| format!("entity_type = '{t}'"));
        store
            .vector_store()
            .vector_search_filtered(&embedding, oversampled, pushdown.as_deref(), valid_at)?
            .into_iter()
            .filter(|m| {
                store
                    .resolve(m.entity_id)
                    .is_ok_and(|iri| allowed.contains(&iri))
            })
            .collect::<Vec<_>>()
    } else {
        store.vector_search(&embedding, oversampled, valid_at)?
    };

    // Dedupe by entity, keeping the highest-scoring occurrence. Matches arrive
    // score-descending, so the first row seen for an entity is its best one.
    let mut seen = std::collections::HashSet::new();
    let matches: Vec<_> = matches
        .into_iter()
        .filter(|m| seen.insert(m.entity_id))
        .take(limit)
        .collect();

    let results: Vec<JsonValue> = matches
        .iter()
        .map(|m| {
            let iri = store
                .resolve(m.entity_id)
                .unwrap_or_else(|_| format!("ref:{}", m.entity_id));
            let iri = prefixes
                .as_ref()
                .map_or(iri.clone(), |map| map.compact(&iri));
            serde_json::json!({
                "entity": iri,
                "text": m.text,
                "score": m.score,
                "source": "knowledge",
                "valid_from": m.valid_from,
                "valid_to": m.valid_to
            })
        })
        .collect();

    Ok(serde_json::json!({
        "results": results,
        "count": results.len(),
        "scoped": scope.is_some()
    }))
}

/// Resolve the set of entity IRIs permitted by an optional `entity_type` and/or
/// `group_ids` scope — best-effort PROVENANCE scoping, NOT tenant isolation
/// (hq-93d; design: docs/design/group-isolation.md). Returns `Ok(None)` when no
/// scope is requested (caller wants the full graph). Group membership is resolved
/// via a REQUIRED `prov:wasGeneratedBy → episode → groupId` join, matching the
/// `search_nodes` path — so `/knot` facts, which have no episode, are DROPPED from
/// any group scope rather than returned. This is a narrowing filter, not a
/// boundary a caller cannot widen.
///
/// Pattern order matters (perf). The BGP evaluator joins triple patterns left to
/// right, running one SQL query per pattern *per surviving row*, so the leading
/// pattern decides the whole cost. This query used to open with an unbound
/// `?s ?p ?o .`, which enumerated every fact in the graph and then probed the
/// store once per fact: 53.6s on a 4967-fact graph, during which the server's CPU
/// quota was saturated and Traefik dropped the backend, 503-ing the public
/// endpoint for everyone else. The `?s ?p ?o` triple also constrained nothing —
/// every ?s the remaining patterns can bind necessarily has a fact.
///
/// So: drop it, and lead with the most selective pattern (episode→groupId, one
/// row per episode) rather than the fan-out one (?s→episode). Same 4967-fact
/// graph, same results: 53.6s → 0.016s.
///
/// There is deliberately no LIMIT. This scope set is an *allow-list*; truncating
/// it silently drops legitimate in-scope entities from every scoped search. The
/// old `LIMIT oversample(limit)` capped one populous group at 100 of its
/// 457 entities, so ~78% of that group's graph was unsearchable.
fn scoped_entity_iris(
    store: &Store,
    entity_type: Option<&str>,
    group_ids: Option<&[&str]>,
) -> Result<Option<std::collections::HashSet<String>>> {
    let has_group = group_ids.is_some_and(|g| !g.is_empty());
    if entity_type.is_none() && !has_group {
        return Ok(None);
    }

    let mut patterns = String::new();
    let mut filters = String::new();
    if has_group {
        patterns.push_str(
            "?_episode <http://aegis.gastown.local/ontology/groupId> ?_gid . \
             ?s <http://www.w3.org/ns/prov#wasGeneratedBy> ?_episode . ",
        );
        let gid_filters: Vec<String> = group_ids
            .unwrap()
            .iter()
            .map(|g| {
                let safe = g.replace('\\', "\\\\").replace('\'', "\\'");
                format!("?_gid = '{safe}'")
            })
            .collect();
        filters.push_str(&format!("FILTER({}) ", gid_filters.join(" || ")));
    }
    if let Some(type_iri) = entity_type {
        let safe_type = type_iri.replace('>', "\\>");
        patterns.push_str(&format!("?s a <{safe_type}> . "));
    }

    let sparql = format!("SELECT DISTINCT ?s WHERE {{ {patterns}{filters}}}");
    let result = sparql::query(store, &sparql)?;

    let mut iris = std::collections::HashSet::new();
    for row in result.rows() {
        if let Some(Value::Ref(id)) = row.get("s") {
            iris.insert(store.resolve(*id)?);
        }
    }
    Ok(Some(iris))
}

/// Extract a `LanceDB` predicate-pushdown filter from a simple SPARQL type query.
///
/// Recognises patterns like `SELECT ?s WHERE { ?s a <TypeIRI> }` and converts
/// them to `entity_type = 'TypeIRI'`. Returns `None` for complex queries that
/// cannot be reduced to a single type constraint.
pub(crate) fn extract_type_filter(sparql: &str) -> Option<String> {
    // Normalise whitespace for matching.
    let normalised: String = sparql.split_whitespace().collect::<Vec<_>>().join(" ");

    // Match: ... { ?<var> a <IRI> } or ... { ?<var> a <IRI> . }
    // Also match rdf:type in full IRI form.
    let type_predicates = [" a ", " <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> "];

    for pred in &type_predicates {
        if let Some(pos) = normalised.find(pred) {
            let after = &normalised[pos + pred.len()..];
            // Extract the IRI between < and >
            if let Some(start) = after.find('<')
                && let Some(end) = after[start..].find('>')
            {
                let iri = &after[start + 1..start + end];
                // Verify this looks like a simple single-pattern query
                // (no UNION, OPTIONAL, FILTER, etc.)
                let upper = normalised.to_uppercase();
                if !upper.contains("UNION")
                    && !upper.contains("OPTIONAL")
                    && !upper.contains("FILTER")
                    && !upper.contains("MINUS")
                {
                    return Some(format!("entity_type = '{iri}'"));
                }
            }
        }
    }

    None
}

/// MCP tool: `quipu_hybrid_search` — Combined SPARQL + vector similarity search.
///
/// When the SPARQL query is a simple type constraint (e.g. `?s a <Type>`), the
/// type is pushed down into `LanceDB`'s filtered ANN search for O(log n) instead
/// of the old O(n) scan-then-filter path. Complex SPARQL falls back to the
/// two-phase approach (SPARQL candidates → post-filter).
///
/// Accepts either a pre-computed `embedding` vector or a natural-language
/// `query` string. When `query` is provided and no `embedding`, the store's
/// `EmbeddingProvider` is used to embed the text automatically.
///
/// Input: `{ "embedding": [f32...], "query": "text", "sparql": "SELECT ?s WHERE {...}", "limit": N, "valid_at": "..." }`
/// Output: entities ranked by vector similarity, optionally pre-filtered by SPARQL.
pub fn tool_hybrid_search(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let explicit_embedding: Option<Vec<f32>> = input
        .get("embedding")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect()
        });

    let query_text = input.get("query").and_then(|v| v.as_str());

    let embedding = match (explicit_embedding, query_text) {
        (Some(emb), _) => emb,
        (None, Some(text)) => store.embed_query(text)?.ok_or_else(|| {
            Error::InvalidValue(format!(
                "{}\n\nAlternatively, supply a pre-computed 'embedding' array.",
                crate::embedding::NO_PROVIDER_HELP
            ))
        })?,
        (None, None) => {
            return Err(Error::InvalidValue(
                "missing 'embedding' array or 'query' text parameter".into(),
            ));
        }
    };

    let limit = store
        .search_config()
        .clamp_limit(input.get("limit").and_then(serde_json::Value::as_u64));

    let valid_at = input.get("valid_at").and_then(|v| v.as_str());
    let sparql_filter = input.get("sparql").and_then(|v| v.as_str());

    // Try to extract a pushdown filter from SPARQL type constraints.
    let pushdown = sparql_filter.and_then(extract_type_filter);

    // Step 1: If SPARQL filter provided, get candidate entity IRIs for post-filter.
    // This is always needed as a fallback (SQLite) and safety net (complex SPARQL).
    let candidate_iris: Option<Vec<String>> = if let Some(sparql) = sparql_filter {
        let result = crate::sparql::query(store, sparql)?;
        let mut iris = Vec::new();
        for row in result.rows() {
            if let Some(first_var) = result.variables().first() {
                match row.get(first_var) {
                    Some(crate::types::Value::Ref(id)) => {
                        iris.push(store.resolve(*id)?);
                    }
                    Some(crate::types::Value::Str(s)) => {
                        iris.push(s.clone());
                    }
                    _ => {}
                }
            }
        }
        Some(iris)
    } else {
        None
    };

    // Step 2: Vector search with predicate pushdown (LanceDB) or oversample (SQLite).
    let all_matches = store.vector_store().vector_search_filtered(
        &embedding,
        limit,
        pushdown.as_deref(),
        valid_at,
    )?;

    // Step 3: Post-filter by SPARQL candidates when present.
    // With LanceDB pushdown the filter is redundant but harmless (belt + suspenders).
    // With SQLite fallback (oversampled, no pushdown) this is essential.
    let filtered: Vec<_> = if let Some(ref candidates) = candidate_iris {
        all_matches
            .into_iter()
            .filter(|m| {
                store
                    .resolve(m.entity_id)
                    .is_ok_and(|iri| candidates.contains(&iri))
            })
            .take(limit)
            .collect()
    } else {
        all_matches.into_iter().take(limit).collect()
    };

    let results: Vec<JsonValue> = filtered
        .iter()
        .map(|m| {
            let iri = store
                .resolve(m.entity_id)
                .unwrap_or_else(|_| format!("ref:{}", m.entity_id));
            serde_json::json!({
                "entity": iri,
                "text": m.text,
                "score": m.score,
                "source": "knowledge",
                "valid_from": m.valid_from,
                "valid_to": m.valid_to
            })
        })
        .collect();

    Ok(serde_json::json!({
        "results": results,
        "count": results.len(),
        "sparql_candidates": candidate_iris.as_ref().map(std::vec::Vec::len),
        "pushdown_filter": pushdown,
        // quipu #53: zero results with `embedded_entities: 0` means the store
        // was never embedded (e.g. loaded with `quipu knot`, which does not
        // embed) — not that nothing matched.
        "embeddings": {
            "configured": store.embedding_provider().is_some(),
            "embedded_entities": store.vector_store().vector_count().unwrap_or(0),
        },
    }))
}
