//! Graph projection — materialize the fact store into a petgraph `DiGraph`
//! for running graph algorithms (`PageRank`, shortest path, connected components).

use std::collections::HashMap;

use petgraph::algo;
use petgraph::graph::{DiGraph, NodeIndex};
use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

/// A projected graph with entity-to-index mappings.
pub struct ProjectedGraph {
    pub graph: DiGraph<i64, i64>,
    pub entity_to_node: HashMap<i64, NodeIndex>,
    pub node_to_entity: HashMap<NodeIndex, i64>,
}

impl ProjectedGraph {
    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

/// Project the current fact store into a directed graph.
///
/// Nodes are entities (term IDs). Edges exist where a fact's value is a Ref
/// (i.e., entity-to-entity relationship). The edge weight is the predicate ID.
///
/// Optional filters:
/// - `type_filter`: only include entities of this rdf:type IRI
/// - `predicate_filter`: only include edges with this predicate IRI
pub fn project(
    store: &Store,
    type_filter: Option<&str>,
    predicate_filter: Option<&str>,
) -> Result<ProjectedGraph> {
    let facts = store.current_facts()?;

    // If type filter is set, find matching entity IDs.
    let type_entity_ids: Option<std::collections::HashSet<i64>> =
        if let Some(type_iri) = type_filter {
            let rdf_type_id = store.lookup(namespace::RDF_TYPE)?;
            let type_val_id = store.lookup(type_iri)?;
            match (rdf_type_id, type_val_id) {
                (Some(rdf_type), Some(type_val)) => {
                    let ids: std::collections::HashSet<i64> = facts
                        .iter()
                        .filter(|f| f.attribute == rdf_type && f.value == Value::Ref(type_val))
                        .map(|f| f.entity)
                        .collect();
                    Some(ids)
                }
                _ => Some(std::collections::HashSet::new()),
            }
        } else {
            None
        };

    let pred_id_filter: Option<i64> = if let Some(pred_iri) = predicate_filter {
        store.lookup(pred_iri)?.or(Some(-1)) // -1 means "not found, match nothing"
    } else {
        None
    };

    let mut graph = DiGraph::new();
    let mut entity_to_node: HashMap<i64, NodeIndex> = HashMap::new();
    let mut node_to_entity: HashMap<NodeIndex, i64> = HashMap::new();

    let ensure_node = |graph: &mut DiGraph<i64, i64>,
                       e2n: &mut HashMap<i64, NodeIndex>,
                       n2e: &mut HashMap<NodeIndex, i64>,
                       entity_id: i64|
     -> NodeIndex {
        *e2n.entry(entity_id).or_insert_with(|| {
            let idx = graph.add_node(entity_id);
            n2e.insert(idx, entity_id);
            idx
        })
    };

    for fact in &facts {
        // Only create edges for Ref values (entity-to-entity relationships).
        if let Value::Ref(target_id) = &fact.value {
            let source_id = fact.entity;
            let pred_id = fact.attribute;

            // Apply predicate filter.
            if let Some(filter_id) = pred_id_filter
                && pred_id != filter_id
            {
                continue;
            }

            // Apply type filter.
            if let Some(ref type_ids) = type_entity_ids
                && !type_ids.contains(&source_id)
            {
                continue;
            }

            let src = ensure_node(
                &mut graph,
                &mut entity_to_node,
                &mut node_to_entity,
                source_id,
            );
            let tgt = ensure_node(
                &mut graph,
                &mut entity_to_node,
                &mut node_to_entity,
                *target_id,
            );
            graph.add_edge(src, tgt, pred_id);
        }
    }

    Ok(ProjectedGraph {
        graph,
        entity_to_node,
        node_to_entity,
    })
}

/// Compute in-degree for each node (simple influence metric).
pub fn in_degree(pg: &ProjectedGraph) -> Vec<(i64, usize)> {
    let mut degrees: Vec<(i64, usize)> = pg
        .node_to_entity
        .iter()
        .map(|(idx, &entity_id)| {
            let deg = pg
                .graph
                .neighbors_directed(*idx, petgraph::Direction::Incoming)
                .count();
            (entity_id, deg)
        })
        .collect();
    degrees.sort_by_key(|&(_, deg)| std::cmp::Reverse(deg));
    degrees
}

/// Configuration for (personalized) `PageRank`.
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping / restart probability (typically 0.85).
    pub damping: f32,
    /// Seed distribution for personalization (entity term IDs). Empty = uniform
    /// restart = global `PageRank`.
    pub seeds: Vec<i64>,
    /// Maximum power-iteration steps.
    pub max_iters: u32,
    /// L1 convergence tolerance.
    pub tolerance: f32,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            seeds: Vec::new(),
            max_iters: 100,
            tolerance: 1e-6,
        }
    }
}

/// Power-iteration `PageRank` / Personalized `PageRank` over a projected graph.
///
/// Returns `(entity_id, normalized_score)` pairs, descending by score. With an
/// empty `seeds` set this is global `PageRank` (uniform restart); with seeds it is
/// Personalized `PageRank`, with restart mass concentrated on the seed entities.
///
/// Dangling nodes (no out-edges) redistribute their mass to the restart vector,
/// which keeps total rank mass conserved at 1.0. Parallel edges are respected
/// (a node that links a target N times sends it `N/out_degree` of its rank).
pub fn page_rank(pg: &ProjectedGraph, cfg: &PageRankConfig) -> Result<Vec<(i64, f32)>> {
    let n = pg.graph.node_count();
    if n == 0 {
        return Ok(Vec::new());
    }

    // project() only ever adds nodes, so NodeIndex values are contiguous 0..n
    // and `idx.index()` is a valid array position.
    let mut out_targets: Vec<Vec<usize>> = vec![Vec::new(); n];
    for idx in pg.graph.node_indices() {
        let i = idx.index();
        for edge in pg.graph.edges_directed(idx, petgraph::Direction::Outgoing) {
            out_targets[i].push(petgraph::visit::EdgeRef::target(&edge).index());
        }
    }

    // Build the restart (personalization) vector, summing to 1.0.
    let mut restart = vec![0.0f32; n];
    let seed_positions: Vec<usize> = cfg
        .seeds
        .iter()
        .filter_map(|sid| pg.entity_to_node.get(sid).map(|idx| idx.index()))
        .collect();
    if seed_positions.is_empty() {
        // Uniform (global PageRank), or seeds given but none present in graph.
        let p = 1.0 / n as f32;
        restart.fill(p);
    } else {
        let p = 1.0 / seed_positions.len() as f32;
        for &pos in &seed_positions {
            restart[pos] += p;
        }
    }

    let d = cfg.damping;
    let mut rank = restart.clone();
    let mut next = vec![0.0f32; n];

    for _ in 0..cfg.max_iters.max(1) {
        // Base: teleport term.
        for i in 0..n {
            next[i] = (1.0 - d) * restart[i];
        }
        // Dangling mass redistributed to the restart vector.
        let mut dangling_mass = 0.0f32;
        for i in 0..n {
            if out_targets[i].is_empty() {
                dangling_mass += rank[i];
            }
        }
        if dangling_mass > 0.0 {
            for i in 0..n {
                next[i] += d * dangling_mass * restart[i];
            }
        }
        // Push rank along out-edges.
        for i in 0..n {
            let deg = out_targets[i].len();
            if deg == 0 {
                continue;
            }
            let share = d * rank[i] / deg as f32;
            for &j in &out_targets[i] {
                next[j] += share;
            }
        }

        // L1 convergence check.
        let mut diff = 0.0f32;
        for i in 0..n {
            diff += (next[i] - rank[i]).abs();
        }
        std::mem::swap(&mut rank, &mut next);
        if diff < cfg.tolerance {
            break;
        }
    }

    // Normalize defensively (mass is conserved, but guard against drift).
    let sum: f32 = rank.iter().sum();
    if sum > 0.0 {
        for r in &mut rank {
            *r /= sum;
        }
    }

    let mut results: Vec<(i64, f32)> = pg
        .graph
        .node_indices()
        .map(|idx| (pg.node_to_entity[&idx], rank[idx.index()]))
        .collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(results)
}

/// Find connected components (weakly connected, ignoring direction).
pub fn connected_components(pg: &ProjectedGraph) -> Vec<Vec<i64>> {
    let components = algo::kosaraju_scc(&pg.graph);
    components
        .into_iter()
        .map(|component| {
            component
                .into_iter()
                .map(|idx| pg.node_to_entity[&idx])
                .collect()
        })
        .collect()
}

/// Shortest path between two entities (by IRI), returns the path as entity IRIs.
pub fn shortest_path(
    store: &Store,
    pg: &ProjectedGraph,
    from_iri: &str,
    to_iri: &str,
) -> Result<Option<Vec<String>>> {
    let from_id = store.lookup(from_iri)?;
    let to_id = store.lookup(to_iri)?;

    let (Some(from_id), Some(to_id)) = (from_id, to_id) else {
        return Ok(None);
    };

    let from_idx = match pg.entity_to_node.get(&from_id) {
        Some(idx) => *idx,
        None => return Ok(None),
    };
    let to_idx = match pg.entity_to_node.get(&to_id) {
        Some(idx) => *idx,
        None => return Ok(None),
    };

    // BFS shortest path (unweighted).
    let path = algo::astar(&pg.graph, from_idx, |n| n == to_idx, |_| 1, |_| 0);

    match path {
        Some((_cost, nodes)) => {
            let iris: Result<Vec<String>> = nodes
                .into_iter()
                .map(|idx| {
                    let entity_id = pg.node_to_entity[&idx];
                    store.resolve(entity_id)
                })
                .collect();
            Ok(Some(iris?))
        }
        None => Ok(None),
    }
}

/// MCP tool: `quipu_project` — Project the knowledge graph and run algorithms.
///
/// Input: `{ "type": "<optional IRI>", "predicate": "<optional IRI>",
///           "algorithm": "stats|in_degree|components|shortest_path",
///           "from": "<IRI>", "to": "<IRI>" }`
pub fn tool_project(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let type_filter = input.get("type").and_then(|v| v.as_str());
    let pred_filter = input.get("predicate").and_then(|v| v.as_str());
    let algorithm = input
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("stats");

    let pg = project(store, type_filter, pred_filter)?;

    match algorithm {
        "stats" => Ok(serde_json::json!({
            "nodes": pg.node_count(),
            "edges": pg.edge_count(),
        })),
        "in_degree" => {
            let limit = input
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(20) as usize;
            let degrees = in_degree(&pg);
            let results: Vec<JsonValue> = degrees
                .into_iter()
                .take(limit)
                .map(|(entity_id, deg)| {
                    let iri = store
                        .resolve(entity_id)
                        .unwrap_or_else(|_| format!("ref:{entity_id}"));
                    serde_json::json!({"entity": iri, "in_degree": deg})
                })
                .collect();
            Ok(serde_json::json!({
                "algorithm": "in_degree",
                "results": results,
                "count": results.len()
            }))
        }
        "pagerank" | "ppr" => {
            let damping = input
                .get("damping")
                .and_then(serde_json::Value::as_f64)
                .map_or(0.85, |v| v as f32);
            let max_iters = input
                .get("max_iters")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(100) as u32;
            let tolerance = input
                .get("tolerance")
                .and_then(serde_json::Value::as_f64)
                .map_or(1e-6, |v| v as f32);
            let limit = input
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(20) as usize;

            // Seeds accepted as IRIs (resolved to term IDs) or raw integer IDs.
            let mut seeds: Vec<i64> = Vec::new();
            if let Some(arr) = input.get("seeds").and_then(|v| v.as_array()) {
                for s in arr {
                    if let Some(iri) = s.as_str() {
                        if let Some(id) = store.lookup(iri)? {
                            seeds.push(id);
                        }
                    } else if let Some(id) = s.as_i64() {
                        seeds.push(id);
                    }
                }
            }

            let personalized = !seeds.is_empty();
            let cfg = PageRankConfig {
                damping,
                seeds,
                max_iters,
                tolerance,
            };
            let ranked = page_rank(&pg, &cfg)?;
            let results: Vec<JsonValue> = ranked
                .into_iter()
                .take(limit)
                .map(|(entity_id, score)| {
                    let iri = store
                        .resolve(entity_id)
                        .unwrap_or_else(|_| format!("ref:{entity_id}"));
                    serde_json::json!({"entity": iri, "score": score})
                })
                .collect();
            Ok(serde_json::json!({
                "algorithm": "pagerank",
                "personalized": personalized,
                "results": results,
                "count": results.len()
            }))
        }
        "components" => {
            let components = connected_components(&pg);
            let results: Vec<JsonValue> = components
                .into_iter()
                .map(|comp| {
                    let iris: Vec<String> = comp
                        .into_iter()
                        .map(|id| store.resolve(id).unwrap_or_else(|_| format!("ref:{id}")))
                        .collect();
                    serde_json::json!({"entities": iris, "size": iris.len()})
                })
                .collect();
            Ok(serde_json::json!({
                "algorithm": "components",
                "components": results,
                "count": results.len()
            }))
        }
        "shortest_path" => {
            let from = input.get("from").and_then(|v| v.as_str()).ok_or_else(|| {
                crate::Error::InvalidValue("missing 'from' IRI for shortest_path".into())
            })?;
            let to = input.get("to").and_then(|v| v.as_str()).ok_or_else(|| {
                crate::Error::InvalidValue("missing 'to' IRI for shortest_path".into())
            })?;
            let path = shortest_path(store, &pg, from, to)?;
            Ok(serde_json::json!({
                "algorithm": "shortest_path",
                "from": from,
                "to": to,
                "path": path,
                "length": path.as_ref().map(|p| p.len().saturating_sub(1))
            }))
        }
        other => Err(crate::Error::InvalidValue(format!(
            "unknown algorithm: {other} (try: stats, in_degree, pagerank, components, shortest_path)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;

    fn test_graph_store() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let turtle = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:knows ex:bob ; ex:knows ex:carol .
ex:bob a ex:Person ; ex:knows ex:carol .
ex:carol a ex:Person ; ex:knows ex:dave .
ex:dave a ex:Person .
ex:server1 a ex:Server ; ex:hosts ex:app1 .
ex:app1 a ex:App ; ex:uses ex:server1 .
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
    fn test_project_all() {
        let store = test_graph_store();
        let pg = project(&store, None, None).unwrap();
        assert!(pg.node_count() >= 6);
        assert!(pg.edge_count() >= 10); // includes rdf:type edges
    }

    #[test]
    fn test_project_type_filter() {
        let store = test_graph_store();
        let pg = project(&store, Some("http://example.org/Person"), None).unwrap();
        // Only Person entities as sources
        assert!(pg.node_count() >= 4);
    }

    #[test]
    fn test_project_predicate_filter() {
        let store = test_graph_store();
        let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
        assert_eq!(pg.edge_count(), 4); // alice->bob, alice->carol, bob->carol, carol->dave
    }

    #[test]
    fn test_in_degree() {
        let store = test_graph_store();
        let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
        let degrees = in_degree(&pg);
        // carol should have highest in-degree (alice + bob know carol)
        let carol_id = store.lookup("http://example.org/carol").unwrap().unwrap();
        let carol_deg = degrees.iter().find(|(id, _)| *id == carol_id).unwrap().1;
        assert_eq!(carol_deg, 2);
    }

    #[test]
    fn test_shortest_path() {
        let store = test_graph_store();
        let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
        let path = shortest_path(
            &store,
            &pg,
            "http://example.org/alice",
            "http://example.org/dave",
        )
        .unwrap();
        assert!(path.is_some());
        let path = path.unwrap();
        // alice -> carol -> dave (length 2)
        assert!(path.len() <= 4); // at most alice->bob->carol->dave
        assert_eq!(path.first().unwrap(), "http://example.org/alice");
        assert_eq!(path.last().unwrap(), "http://example.org/dave");
    }

    #[test]
    fn test_pagerank_converges_and_sums_to_one() {
        let store = test_graph_store();
        let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
        let ranks = page_rank(&pg, &PageRankConfig::default()).unwrap();
        assert!(!ranks.is_empty());
        let sum: f32 = ranks.iter().map(|(_, s)| s).sum();
        assert!(
            (sum - 1.0).abs() < 1e-3,
            "ranks should sum to ~1, got {sum}"
        );
    }

    #[test]
    fn test_pagerank_ranks_hub_highest() {
        let store = test_graph_store();
        let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
        let ranks = page_rank(&pg, &PageRankConfig::default()).unwrap();
        // dave is a sink reached via carol (alice/bob/carol all flow toward it);
        // carol is referenced by both alice and bob. Top-ranked should be carol
        // or dave, never alice (which has no incoming knows edges).
        let alice = store.lookup("http://example.org/alice").unwrap().unwrap();
        let top = ranks[0].0;
        assert_ne!(top, alice, "alice has no in-edges and must not rank first");
        let carol = store.lookup("http://example.org/carol").unwrap().unwrap();
        let dave = store.lookup("http://example.org/dave").unwrap().unwrap();
        assert!(top == carol || top == dave, "expected carol or dave on top");
    }

    #[test]
    fn test_personalized_pagerank_favors_seed_neighborhood() {
        let store = test_graph_store();
        let pg = project(&store, None, Some("http://example.org/knows")).unwrap();
        let alice = store.lookup("http://example.org/alice").unwrap().unwrap();
        let cfg = PageRankConfig {
            seeds: vec![alice],
            ..Default::default()
        };
        let ranks = page_rank(&pg, &cfg).unwrap();
        // Personalized at alice: alice itself should carry significant rank
        // (restart mass) — far more than under global PageRank where it has 0
        // in-edges.
        let alice_score = ranks.iter().find(|(id, _)| *id == alice).unwrap().1;
        assert!(
            alice_score > 0.1,
            "seed should retain restart mass, got {alice_score}"
        );
    }

    #[test]
    fn test_pagerank_empty_graph() {
        let store = Store::open_in_memory().unwrap();
        let pg = project(&store, None, None).unwrap();
        let ranks = page_rank(&pg, &PageRankConfig::default()).unwrap();
        assert!(ranks.is_empty());
    }

    #[test]
    fn test_tool_project_pagerank() {
        let store = test_graph_store();
        let input = serde_json::json!({
            "algorithm": "pagerank",
            "predicate": "http://example.org/knows",
            "limit": 5
        });
        let result = tool_project(&store, &input).unwrap();
        assert_eq!(result["algorithm"], "pagerank");
        assert_eq!(result["personalized"], false);
        assert!(result["count"].as_u64().unwrap() > 0);
        assert!(result["results"][0]["score"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn test_tool_project_ppr_with_seeds() {
        let store = test_graph_store();
        let input = serde_json::json!({
            "algorithm": "ppr",
            "predicate": "http://example.org/knows",
            "seeds": ["http://example.org/alice"]
        });
        let result = tool_project(&store, &input).unwrap();
        assert_eq!(result["algorithm"], "pagerank");
        assert_eq!(result["personalized"], true);
    }

    #[test]
    fn test_connected_components() {
        let store = test_graph_store();
        let pg = project(&store, None, None).unwrap();
        let comps = connected_components(&pg);
        assert!(!comps.is_empty());
    }

    #[test]
    fn test_tool_project_stats() {
        let store = test_graph_store();
        let input = serde_json::json!({"algorithm": "stats"});
        let result = tool_project(&store, &input).unwrap();
        assert!(result["nodes"].as_u64().unwrap() >= 6);
        assert!(result["edges"].as_u64().unwrap() >= 4);
    }

    #[test]
    fn test_tool_project_in_degree() {
        let store = test_graph_store();
        let input = serde_json::json!({
            "algorithm": "in_degree",
            "predicate": "http://example.org/knows",
            "limit": 5
        });
        let result = tool_project(&store, &input).unwrap();
        assert_eq!(result["algorithm"], "in_degree");
        assert!(result["count"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_tool_project_shortest_path() {
        let store = test_graph_store();
        let input = serde_json::json!({
            "algorithm": "shortest_path",
            "predicate": "http://example.org/knows",
            "from": "http://example.org/alice",
            "to": "http://example.org/dave"
        });
        let result = tool_project(&store, &input).unwrap();
        assert!(result["path"].is_array());
        assert!(result["length"].as_u64().unwrap() >= 2);
    }
}
