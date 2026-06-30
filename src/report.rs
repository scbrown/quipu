//! Graph report — a live, queryable equivalent of graphify's static
//! `GRAPH_REPORT.md`. Synthesizes three views of the current knowledge graph in
//! one read-only pass (hq-ct27, backend for the graph-report skill / aegis-1p0
//! Gap 4):
//!
//! 1. **Hubs** ("god-nodes") — the most structurally central entities, by
//!    `PageRank` with in-degree as a secondary signal.
//! 2. **Surprising connections** — low-prior cross-community edges: relationships
//!    that bridge two otherwise-separate Louvain clusters. The rarer the bridge
//!    (the fewer edges that cross between the same two communities), the more
//!    surprising it is.
//! 3. **Suggested questions** — deterministic, template-generated prompts seeded
//!    by the hubs and bridges above, to give an agent or human a starting point.
//!
//! Everything is derived from graph structure in a single immutable read; nothing
//! is persisted. Community membership here is *emergent clustering for surfacing*,
//! **not** an access boundary (see `graph` module note and hq-2u3 / hq-zlph).

use std::collections::HashMap;

use petgraph::visit::EdgeRef;
use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::graph::{PageRankConfig, in_degree, louvain, page_rank, project};
use crate::namespace;
use crate::store::Store;

/// Shorten an IRI to a human-readable local name (the part after the last `#`,
/// `/`, or `:`). Falls back to the whole string when no separator is present.
fn short(iri: &str) -> &str {
    iri.rsplit(['#', '/', ':']).next().unwrap_or(iri)
}

/// MCP tool: `quipu_report` — god-nodes, surprising connections, and suggested
/// questions for the current graph (read-only).
///
/// Input (all optional):
/// - `type`: restrict the projection to this rdf:type IRI
/// - `predicate`: restrict the projection to edges with this predicate IRI
/// - `hubs`: number of top hubs to return (default 10)
/// - `surprises`: number of surprising connections to return (default 10)
/// - `questions`: number of suggested questions to return (default 8)
pub fn tool_report(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let type_filter = input.get("type").and_then(|v| v.as_str());
    let pred_filter = input.get("predicate").and_then(|v| v.as_str());
    let n_hubs = input
        .get("hubs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10) as usize;
    let n_surprises = input
        .get("surprises")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10) as usize;
    let n_questions = input
        .get("questions")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(8) as usize;

    let pg = project(store, type_filter, pred_filter)?;

    // ── Hubs ("god-nodes"): PageRank, in-degree as secondary signal ──────────
    let degrees: HashMap<i64, usize> = in_degree(&pg).into_iter().collect();
    let mut ranked = page_rank(&pg, &PageRankConfig::default())?;
    // page_rank sorts by score desc but leaves equal scores in projection order;
    // break ties by ascending entity id so the report is deterministic.
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let hubs: Vec<JsonValue> = ranked
        .iter()
        .take(n_hubs)
        .map(|&(entity_id, score)| {
            let iri = store
                .resolve(entity_id)
                .unwrap_or_else(|_| format!("ref:{entity_id}"));
            serde_json::json!({
                "entity": iri,
                "pagerank": score,
                "in_degree": degrees.get(&entity_id).copied().unwrap_or(0),
            })
        })
        .collect();
    // Quick lookup of an entity's PageRank for bridge tie-breaking below.
    let pr: HashMap<i64, f32> = ranked.iter().copied().collect();

    // ── Surprising connections: low-prior cross-community edges ──────────────
    let communities = louvain(&pg);
    // entity id -> community index.
    let mut community_of: HashMap<i64, usize> = HashMap::new();
    for (k, group) in communities.groups.iter().enumerate() {
        for &entity in group {
            community_of.insert(entity, k);
        }
    }

    // Collect cross-community edges in a deterministic order, and count how many
    // edges cross between each unordered community pair (the bridge's rarity).
    let mut cross: Vec<(i64, i64, i64, usize, usize)> = Vec::new(); // (src, tgt, pred, ca, cb)
    let mut pair_count: HashMap<(usize, usize), usize> = HashMap::new();
    for edge in pg.graph.edge_references() {
        let src = pg.node_to_entity[&edge.source()];
        let tgt = pg.node_to_entity[&edge.target()];
        let (Some(&ca), Some(&cb)) = (community_of.get(&src), community_of.get(&tgt)) else {
            continue;
        };
        if ca == cb {
            continue;
        }
        let pair = if ca <= cb { (ca, cb) } else { (cb, ca) };
        *pair_count.entry(pair).or_insert(0) += 1;
        cross.push((src, tgt, *edge.weight(), ca, cb));
    }

    // Surprise ranking: rarer bridges first (low pair_count = low-prior), then
    // bridges touching more important nodes (higher combined PageRank), then a
    // stable id-based tiebreak.
    cross.sort_by(|a, b| {
        let pa = pair_count[&if a.3 <= a.4 { (a.3, a.4) } else { (a.4, a.3) }];
        let pb = pair_count[&if b.3 <= b.4 { (b.3, b.4) } else { (b.4, b.3) }];
        let impa = pr.get(&a.0).copied().unwrap_or(0.0) + pr.get(&a.1).copied().unwrap_or(0.0);
        let impb = pr.get(&b.0).copied().unwrap_or(0.0) + pr.get(&b.1).copied().unwrap_or(0.0);
        pa.cmp(&pb)
            .then(impb.partial_cmp(&impa).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });

    let surprising: Vec<JsonValue> = cross
        .iter()
        .take(n_surprises)
        .map(|&(src, tgt, pred, ca, cb)| {
            let pair = if ca <= cb { (ca, cb) } else { (cb, ca) };
            serde_json::json!({
                "from": store.resolve(src).unwrap_or_else(|_| format!("ref:{src}")),
                "to": store.resolve(tgt).unwrap_or_else(|_| format!("ref:{tgt}")),
                "predicate": store.resolve(pred).unwrap_or_else(|_| format!("ref:{pred}")),
                "community_from": format!("{}community_{ca}", namespace::QUIPU),
                "community_to": format!("{}community_{cb}", namespace::QUIPU),
                "bridge_rarity": pair_count[&pair],
            })
        })
        .collect();

    // ── Suggested questions: deterministic templates over hubs + bridges ─────
    let mut questions: Vec<String> = Vec::new();
    for h in &hubs {
        if let Some(iri) = h["entity"].as_str() {
            questions.push(format!("What is {}, and why is it so central?", short(iri)));
        }
    }
    for s in &surprising {
        if let (Some(a), Some(b)) = (s["from"].as_str(), s["to"].as_str()) {
            questions.push(format!(
                "Why is {} connected to {} across otherwise-separate clusters?",
                short(a),
                short(b)
            ));
        }
    }
    if let Some(largest) = communities.groups.iter().max_by_key(|g| g.len())
        && let Some(&example) = largest.first()
    {
        let name = store
            .resolve(example)
            .unwrap_or_else(|_| format!("ref:{example}"));
        questions.push(format!(
            "What ties together the largest cluster ({} entities, e.g. {})?",
            largest.len(),
            short(&name)
        ));
    }
    questions.truncate(n_questions);

    Ok(serde_json::json!({
        "graph": {
            "nodes": pg.node_count(),
            "edges": pg.edge_count(),
            "communities": communities.groups.len(),
            "modularity": communities.modularity,
        },
        "hubs": hubs,
        "surprising_connections": surprising,
        "suggested_questions": questions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;
    use crate::store::Store;

    /// Build a two-cluster graph with a single rare bridge between them, plus a
    /// clear hub (`hubA`, referenced by a1/a2/a3) inside cluster A.
    fn seeded_store() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let turtle = r#"
@prefix ex: <http://example.org/> .
ex:a1 ex:rel ex:hubA , ex:a2 .
ex:a2 ex:rel ex:hubA , ex:a3 .
ex:a3 ex:rel ex:hubA .
ex:b1 ex:rel ex:b2 .
ex:b2 ex:rel ex:b3 .
ex:b3 ex:rel ex:b1 .
ex:a1 ex:bridge ex:b1 .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    #[test]
    fn report_has_three_sections() {
        let store = seeded_store();
        let out = tool_report(&store, &serde_json::json!({})).unwrap();
        assert!(out["hubs"].is_array());
        assert!(out["surprising_connections"].is_array());
        assert!(out["suggested_questions"].is_array());
        assert!(out["graph"]["nodes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn hubs_ranked_by_pagerank_with_indegree_reported() {
        let store = seeded_store();
        let out = tool_report(&store, &serde_json::json!({ "hubs": 10 })).unwrap();
        let hubs = out["hubs"].as_array().unwrap();
        assert!(!hubs.is_empty());
        // Hubs are ordered by PageRank descending (the primary "god-node" signal).
        let scores: Vec<f64> = hubs
            .iter()
            .map(|h| h["pagerank"].as_f64().unwrap())
            .collect();
        assert!(
            scores.windows(2).all(|w| w[0] >= w[1]),
            "hubs must be sorted by pagerank desc, got {scores:?}"
        );
        // in_degree is reported as a secondary signal: hubA is referenced by
        // a1/a2/a3, the highest in-degree in the graph.
        let huba = hubs
            .iter()
            .find(|h| h["entity"].as_str().unwrap().ends_with("hubA"))
            .expect("hubA should be present in the hub list");
        assert_eq!(huba["in_degree"].as_u64().unwrap(), 3);
    }

    #[test]
    fn surprising_connection_is_the_bridge() {
        let store = seeded_store();
        let out = tool_report(&store, &serde_json::json!({})).unwrap();
        let surprises = out["surprising_connections"].as_array().unwrap();
        assert!(
            !surprises.is_empty(),
            "expected at least one cross-community edge"
        );
        let top = &surprises[0];
        assert!(top["predicate"].as_str().unwrap().ends_with("bridge"));
        assert_eq!(top["bridge_rarity"].as_u64().unwrap(), 1);
        assert_ne!(top["community_from"], top["community_to"]);
    }

    #[test]
    fn deterministic_across_runs() {
        let store = seeded_store();
        let a = tool_report(&store, &serde_json::json!({})).unwrap();
        let b = tool_report(&store, &serde_json::json!({})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn respects_limits() {
        let store = seeded_store();
        let out = tool_report(
            &store,
            &serde_json::json!({ "hubs": 2, "surprises": 1, "questions": 3 }),
        )
        .unwrap();
        assert!(out["hubs"].as_array().unwrap().len() <= 2);
        assert!(out["surprising_connections"].as_array().unwrap().len() <= 1);
        assert!(out["suggested_questions"].as_array().unwrap().len() <= 3);
    }
}
