//! (Personalized) `PageRank` over a [`ProjectedGraph`] — the config and the
//! kernel. Split from `graph.rs` for the file-size ratchet; public paths are
//! unchanged (`graph.rs` re-exports both items).

use super::ProjectedGraph;
use crate::error::Result;

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
