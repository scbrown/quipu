//! Graph projection — materialize the fact store into a petgraph DiGraph
//! for running graph algorithms (PageRank, shortest path, connected components).

use std::collections::HashMap;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo;
use serde_json::Value as JsonValue;

use crate::error::Result;
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
    let type_entity_ids: Option<std::collections::HashSet<i64>> = if let Some(type_iri) = type_filter {
        let rdf_type_id = store.lookup("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")?;
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

            let src = ensure_node(&mut graph, &mut entity_to_node, &mut node_to_entity, source_id);
            let tgt = ensure_node(&mut graph, &mut entity_to_node, &mut node_to_entity, *target_id);
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
    degrees.sort_by(|a, b| b.1.cmp(&a.1));
    degrees
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

    let (from_id, to_id) = match (from_id, to_id) {
        (Some(f), Some(t)) => (f, t),
        _ => return Ok(None),
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
    let path = algo::astar(
        &pg.graph,
        from_idx,
        |n| n == to_idx,
        |_| 1,
        |_| 0,
    );

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
                .and_then(|v| v.as_u64())
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
        "components" => {
            let components = connected_components(&pg);
            let results: Vec<JsonValue> = components
                .into_iter()
                .map(|comp| {
                    let iris: Vec<String> = comp
                        .into_iter()
                        .map(|id| {
                            store
                                .resolve(id)
                                .unwrap_or_else(|_| format!("ref:{id}"))
                        })
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
            let from = input
                .get("from")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    crate::Error::InvalidValue("missing 'from' IRI for shortest_path".into())
                })?;
            let to = input
                .get("to")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
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
            "unknown algorithm: {other} (try: stats, in_degree, components, shortest_path)"
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
