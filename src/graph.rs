//! Graph projection — materialize the fact store into a petgraph `DiGraph`
//! for running graph algorithms (`PageRank`, shortest path, connected
//! components, Louvain community detection).
//!
//! **Communities are not an access boundary.** `louvain` (with `persist:true`)
//! writes `quipu:memberOfCommunity` facts, but community membership is *emergent
//! clustering* derived from graph structure — it is not a tenancy or
//! authorization primitive. Like `group_id` (hq-2u3: provenance, not isolation),
//! it must never be used to gate access; consumers must not build access control
//! on community membership.

use std::collections::HashMap;
use std::sync::Arc;

use petgraph::algo;
use petgraph::graph::{DiGraph, NodeIndex};
use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::namespace;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

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

/// Project the current fact store (ROOT graph) into a directed graph.
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
    project_in_graph(store, type_filter, predicate_filter, None)
}

/// Project ONE graph's current facts into a directed graph (quipu-tz5).
///
/// `graph = None` is ROOT — byte-for-byte the projection [`project`] always
/// built. `Some(iri)` scopes the scan to that named graph's OWN facts (the
/// same scope a `GRAPH <iri> { … }` read sees, not a composed overlay view),
/// which is what makes projecting a small derived layer cheap against a large
/// episode log. An unknown graph IRI is an error rather than an empty graph:
/// a typo that silently ranks nothing is worse than a refusal.
pub fn project_in_graph(
    store: &Store,
    type_filter: Option<&str>,
    predicate_filter: Option<&str>,
    graph: Option<&str>,
) -> Result<ProjectedGraph> {
    let g = resolve_graph(store, graph)?;
    let facts = store.current_facts_in_graph(g)?;

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

/// Resolve an optional graph IRI to its `g` id (`None` = ROOT).
fn resolve_graph(store: &Store, graph: Option<&str>) -> Result<i64> {
    match graph {
        None => Ok(crate::schema::ROOT_GRAPH),
        Some(iri) => store.lookup(iri)?.ok_or_else(|| {
            crate::error::Error::Store(format!("unknown graph: {iri} (never written to)"))
        }),
    }
}

/// One memoized projection, resident on [`Store`] (quipu-tz5).
///
/// Single-entry by design: the observed access pattern is repeated calls with
/// the same shape (an agent iterating pagerank/communities over one layer),
/// not a working set of distinct projections. The tx stamp does the
/// invalidation — [`Store::latest_tx_id`] is monotonic and cheap, and its docs
/// exist for exactly this rebuild-when-it-moves pattern — so no write-path
/// hook is needed. Valid-time passage cannot stale the entry because the scan
/// only reads facts with `valid_to IS NULL`.
pub(crate) struct ProjectionCacheEntry {
    key: (i64, Option<String>, Option<String>),
    tx: i64,
    pg: Arc<ProjectedGraph>,
}

/// [`project_in_graph`], memoized on the store (quipu-tz5).
///
/// A hit returns the resident [`Arc`] without touching SQL; anything else
/// (different graph or filters, or any committed transaction since the entry
/// was built) rebuilds and replaces the entry. The projection holds only term
/// ids and petgraph indices — a fraction of the fact rows it was scanned
/// from — so keeping one resident is far cheaper than the read model this
/// pattern is borrowed from.
pub fn project_cached(
    store: &Store,
    type_filter: Option<&str>,
    predicate_filter: Option<&str>,
    graph: Option<&str>,
) -> Result<Arc<ProjectedGraph>> {
    let g = resolve_graph(store, graph)?;
    let key = (
        g,
        type_filter.map(str::to_string),
        predicate_filter.map(str::to_string),
    );
    let tx = store.latest_tx_id()?;
    if let Some(entry) = store.projected_graph.borrow().as_ref()
        && entry.key == key
        && entry.tx == tx
    {
        return Ok(Arc::clone(&entry.pg));
    }
    let pg = Arc::new(project_in_graph(
        store,
        type_filter,
        predicate_filter,
        graph,
    )?);
    *store.projected_graph.borrow_mut() = Some(ProjectionCacheEntry {
        key,
        tx,
        pg: Arc::clone(&pg),
    });
    Ok(pg)
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

/// A community partition: groups of entity ids plus the modularity score.
pub struct Communities {
    /// One group of entity ids per community. Each group is sorted ascending,
    /// and the outer vector is ordered by each group's minimum entity id, so a
    /// given partition has exactly one canonical representation.
    pub groups: Vec<Vec<i64>>,
    /// Newman–Girvan modularity of the partition (higher = stronger community
    /// structure). `0.0` for an edgeless graph.
    pub modularity: f64,
}

/// Deterministic Louvain community detection (modularity local-moving phase).
///
/// The projected directed multigraph is read as an **undirected weighted**
/// graph: parallel edges sum into one weight, self-loops are ignored, and edge
/// direction is dropped — the standard input shape for modularity-based
/// community detection.
///
/// **Determinism is a hard requirement** (the same graph must always yield the
/// same partition, regardless of `HashMap` iteration order): nodes are visited
/// in ascending entity-id order, candidate communities are evaluated in
/// ascending-label order, and ties in modularity gain are broken toward the
/// lowest community label. No randomness, no hash-order dependence.
///
/// This is Louvain's first (local-moving) level run to convergence — sufficient
/// to separate well-defined communities; the multi-level aggregation step is
/// intentionally omitted to keep the implementation auditable and dependency-free.
pub fn louvain(pg: &ProjectedGraph) -> Communities {
    let n = pg.graph.node_count();
    if n == 0 {
        return Communities {
            groups: Vec::new(),
            modularity: 0.0,
        };
    }

    // Stable index space: entity ids sorted ascending → contiguous 0..n. Visiting
    // nodes in this order (rather than NodeIndex / hash order) is what makes the
    // partition deterministic.
    let mut ids: Vec<i64> = pg.node_to_entity.values().copied().collect();
    ids.sort_unstable();
    let index_of: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, &e)| (e, i)).collect();

    // Undirected weighted adjacency: each directed edge contributes 1.0 to the
    // weight between its endpoints (stored symmetrically); self-loops dropped.
    let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
    for idx in pg.graph.node_indices() {
        let u = index_of[&pg.node_to_entity[&idx]];
        for edge in pg.graph.edges_directed(idx, petgraph::Direction::Outgoing) {
            let v = index_of[&pg.node_to_entity[&petgraph::visit::EdgeRef::target(&edge)]];
            if u == v {
                continue;
            }
            *adj[u].entry(v).or_insert(0.0) += 1.0;
            *adj[v].entry(u).or_insert(0.0) += 1.0;
        }
    }

    // Weighted degree per node and 2m (twice the total edge weight).
    let k: Vec<f64> = adj.iter().map(|a| a.values().sum()).collect();
    let two_m: f64 = k.iter().sum();

    // Edgeless graph: every node is its own community, modularity 0.
    if two_m <= f64::EPSILON {
        return Communities {
            groups: ids.iter().map(|&e| vec![e]).collect(),
            modularity: 0.0,
        };
    }

    // comm[i] = community label of node i. Start with every node isolated.
    let mut comm: Vec<usize> = (0..n).collect();
    // sigma_tot[c] = sum of weighted degrees of nodes currently in community c.
    let mut sigma_tot: HashMap<usize, f64> = (0..n).map(|i| (i, k[i])).collect();
    let mut next_label = n; // fresh labels for re-isolated nodes

    const EPS: f64 = 1e-12;
    const MAX_PASSES: usize = 100; // convergence guard (deterministic upper bound)

    for _ in 0..MAX_PASSES {
        let mut moved = false;
        for i in 0..n {
            let ci = comm[i];
            let ci_total = *sigma_tot.get(&ci).unwrap_or(&0.0);
            let was_alone = (ci_total - k[i]).abs() <= EPS; // i was the only member

            // Remove i from its community.
            let st = sigma_tot.entry(ci).or_insert(0.0);
            *st -= k[i];
            if *st <= EPS {
                sigma_tot.remove(&ci);
            }

            // Sum edge weight from i into each neighbouring community.
            let mut k_i_to: HashMap<usize, f64> = HashMap::new();
            for (&j, &w) in &adj[i] {
                if j == i {
                    continue;
                }
                *k_i_to.entry(comm[j]).or_insert(0.0) += w;
            }

            // Best gain vs. staying isolated (baseline gain 0). Evaluate candidate
            // communities in ascending-label order; strict-greater wins, exact ties
            // resolve to the lowest label — both deterministic.
            let mut cands: Vec<usize> = k_i_to.keys().copied().collect();
            cands.sort_unstable();
            let mut best_gain = 0.0_f64;
            let mut best_c: Option<usize> = None;
            for &c in &cands {
                let gain = k_i_to[&c] - k[i] * sigma_tot.get(&c).copied().unwrap_or(0.0) / two_m;
                if gain > best_gain + EPS {
                    best_gain = gain;
                    best_c = Some(c);
                }
            }

            let target = match best_c {
                Some(c) => c,
                // No positive-gain community: stay isolated. Reuse ci if i was
                // already alone (no real move), else take a fresh singleton label.
                None if was_alone => ci,
                None => {
                    let l = next_label;
                    next_label += 1;
                    l
                }
            };

            *sigma_tot.entry(target).or_insert(0.0) += k[i];
            comm[i] = target;
            if target != ci {
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    // Group nodes by community label, then canonicalise ordering.
    let mut by_label: HashMap<usize, Vec<i64>> = HashMap::new();
    for (i, &c) in comm.iter().enumerate() {
        by_label.entry(c).or_default().push(ids[i]);
    }
    let mut groups: Vec<Vec<i64>> = by_label.into_values().collect();
    for g in &mut groups {
        g.sort_unstable();
    }
    groups.sort_by_key(|g| g[0]); // order by each group's minimum entity id

    let modularity = modularity_of(&comm, &adj, &k, two_m);
    Communities { groups, modularity }
}

/// Newman–Girvan modularity of a partition over the undirected weighted graph.
fn modularity_of(comm: &[usize], adj: &[HashMap<usize, f64>], k: &[f64], two_m: f64) -> f64 {
    if two_m <= f64::EPSILON {
        return 0.0;
    }
    // Σ_in: edge weight inside each community (each undirected edge counted once).
    let mut sigma_in: HashMap<usize, f64> = HashMap::new();
    let mut sigma_tot: HashMap<usize, f64> = HashMap::new();
    for (i, &ci) in comm.iter().enumerate() {
        *sigma_tot.entry(ci).or_insert(0.0) += k[i];
        for (&j, &w) in &adj[i] {
            if comm[j] == ci {
                *sigma_in.entry(ci).or_insert(0.0) += w; // counts each edge twice → /2 below
            }
        }
    }
    sigma_tot
        .iter()
        .map(|(c, &tot)| {
            let in_w = sigma_in.get(c).copied().unwrap_or(0.0) / 2.0;
            in_w / (two_m / 2.0) - (tot / two_m).powi(2)
        })
        .sum()
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
///           "graph": "<optional named-graph IRI, default ROOT>",
///           "algorithm": "stats|in_degree|pagerank|components|louvain|shortest_path",
///           "from": "<IRI>", "to": "<IRI>", "persist": <bool> }`
///
/// Read-only by default. The `louvain` algorithm additionally accepts
/// `persist: true`, which writes `quipu:memberOfCommunity` facts through the
/// store (see [`persist_communities`]); all other algorithms ignore it.
pub fn tool_project(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let type_filter = input.get("type").and_then(|v| v.as_str());
    let pred_filter = input.get("predicate").and_then(|v| v.as_str());
    let graph = input.get("graph").and_then(|v| v.as_str());
    let algorithm = input
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("stats");

    let pg = project_cached(store, type_filter, pred_filter, graph)?;

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
        "louvain" | "community" => {
            let communities = louvain(&pg);

            // Resolve entity ids to IRIs (immutable reads) into owned JSON before
            // any write, so nothing borrows the store across the persist call.
            let results: Vec<JsonValue> = communities
                .groups
                .iter()
                .enumerate()
                .map(|(k, group)| {
                    let entities: Vec<String> = group
                        .iter()
                        .map(|&id| store.resolve(id).unwrap_or_else(|_| format!("ref:{id}")))
                        .collect();
                    serde_json::json!({
                        "community": format!("{}community_{k}", namespace::QUIPU),
                        "entities": entities,
                        "size": group.len(),
                    })
                })
                .collect();

            // Opt-in persistence (read-only by default). Writes
            // quipu:memberOfCommunity facts, superseding any prior derivation.
            let persist = input
                .get("persist")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let persisted = if persist {
                let now = crate::time::now_iso();
                let timestamp = input
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&now);
                Some(persist_communities(store, &communities.groups, timestamp)?)
            } else {
                None
            };

            Ok(serde_json::json!({
                "algorithm": "louvain",
                "communities": results,
                "count": results.len(),
                "modularity": communities.modularity,
                "persisted": persisted,
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
            "unknown algorithm: {other} (try: stats, in_degree, pagerank, components, louvain, shortest_path)"
        ))),
    }
}

/// Persist a community partition as `quipu:memberOfCommunity` facts, bitemporally
/// **superseding** any prior derivation (hq-zlph AC 5).
///
/// Community membership is derived from graph structure, so it goes stale when the
/// graph changes. This reconciles the stored membership against the fresh
/// partition in a single transaction stamped with `source = "algo:louvain"`:
/// memberships that changed are retracted (their `valid_to` closed), genuinely
/// new ones are asserted, and unchanged ones are left untouched (retaining their
/// original assertion time). A default current-facts query (`valid_to IS NULL`)
/// therefore always returns exactly the latest partition — no stale clusters
/// accumulate. Community labels are positional (`community_<k>`) and not stable
/// across runs, so the reconcile compares by `(entity, community-term)` rather
/// than trusting a label.
///
/// (Retract-and-reassert is folded into a diff because the `facts` table is keyed
/// by `(e, a, v, tx)` — retracting *and* re-asserting an identical triple in one
/// transaction would collide on that key.)
///
/// This is emergent clustering, **not** an access boundary — see the module note
/// and hq-zlph AC 6. Returns the total number of memberships in the new partition.
pub fn persist_communities(
    store: &mut Store,
    groups: &[Vec<i64>],
    timestamp: &str,
) -> Result<usize> {
    let pred_id = store.intern(&format!("{}memberOfCommunity", namespace::QUIPU))?;

    // Desired membership set: (entity, community-term-id). Intern labels up front.
    let mut desired: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    for (k, group) in groups.iter().enumerate() {
        let comm_id = store.intern(&format!("{}community_{k}", namespace::QUIPU))?;
        for &entity in group {
            desired.insert((entity, comm_id));
        }
    }
    let total = desired.len();

    let mut datums: Vec<Datum> = Vec::new();

    // Reconcile against current state: drop already-correct memberships from
    // `desired`; retract the rest (stale).
    for fact in store.current_facts()? {
        if fact.attribute != pred_id {
            continue;
        }
        let keep = matches!(fact.value, Value::Ref(cid) if desired.remove(&(fact.entity, cid)));
        if !keep {
            datums.push(Datum {
                entity: fact.entity,
                attribute: pred_id,
                value: fact.value,
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Retract,
            });
        }
    }

    // Assert the genuinely-new memberships (deterministic order).
    let mut to_assert: Vec<(i64, i64)> = desired.into_iter().collect();
    to_assert.sort_unstable();
    for (entity, comm_id) in to_assert {
        datums.push(Datum {
            entity,
            attribute: pred_id,
            value: Value::Ref(comm_id),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }

    store.transact(&datums, timestamp, None, Some("algo:louvain"))?;
    Ok(total)
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
