# PageRank & Personalized PageRank — Specification

> Status: **Specified, unbuilt** · Origin: agent (strider), prompted by Stiwi ·
> Date: 2026-06-21
>
> Fulfils the graph-algorithm intent in [`vision.md`](./vision.md) §9 and the
> `Projection` trait sketch. Companion to Bobbin's
> `docs/plans/ppr-ranking-signal.md`, which consumes this primitive for code
> retrieval. This document is the Quipu-side spec.

## Problem: the centrality gap

Quipu advertises "centrality" (README, CHANGELOG, book, `src/graph.rs:2`) but
ships only **`in_degree()`** (`src/graph.rs:135-149`) — a flat, local metric.
Every higher-order query surface treats connectivity as binary:

- **Impact analysis** (`src/impact.rs:79-153`) orders reached entities by *hops*,
  not importance.
- **Context pipeline** (`src/context/mod.rs:189-252`) keeps expanded neighbors by
  a crude link score, then truncates to `max_entities`.
- **Hybrid / unified search** ranks by vector similarity alone; graph structure
  contributes nothing to the ranking.

Meanwhile the **episode-accretion model** means link structure *does* encode
importance: an entity referenced by many episodes and sitting on many paths is
structurally central, even though no single episode declares it so. Nothing in
Quipu reads that signal today.

## Already-designed intent (alignment)

This spec does not invent a feature — it builds one already on the books:

- `vision.md` §9 "Graph Projection for Algorithms": lists **PageRank** as a target
  algorithm and states **"Results write back as triples."**
- `vision.md` `Projection` trait sketch:
  `fn page_rank(&self, config: PageRankConfig) -> HashMap<NodeId, f64>;`
- `vision.md` "Stolen From" table: *Graph algorithms (PageRank, Louvain) — Neo4j
  GDS — **Pure functions over Projection API**.*
- `src/graph.rs:2` module doc names PageRank as intended.

## Goals

- A **pure** PageRank / Personalized PageRank function over the projected graph,
  matching the vision's "pure functions over the Projection API" shape.
- Expose via `tool_project` (MCP), CLI, and REST, consistent with existing graph
  algorithms.
- Optionally **persist scores as bitemporal triples** so PageRank becomes
  queryable, time-travelable knowledge — not an ephemeral computation.
- Improve ranking on existing surfaces (impact ordering, context relevance).

## Non-goals (v1)

- Louvain / community detection (future; same Projection API).
- Approximate / streaming PageRank at write time.
- Distributed computation (single-process, "SQLite energy").

## API design

A pure function in `src/graph.rs`, no `Store` coupling in the math:

```rust
/// Configuration for (personalized) PageRank.
pub struct PageRankConfig {
    /// Restart probability (typically 0.85).
    pub damping: f32,
    /// Seed distribution for personalization. Empty = uniform = global PageRank.
    pub seeds: Vec<i64>,        // entity term IDs
    pub max_iters: u32,         // e.g. 100
    pub tolerance: f32,         // L1 convergence, e.g. 1e-6
}

/// Power-iteration PageRank over a projected graph.
/// Returns entity term IDs paired with normalized scores, descending.
pub fn page_rank(pg: &ProjectedGraph, cfg: &PageRankConfig)
    -> Result<Vec<(i64, f32)>>;
```

- **Global PageRank**: `seeds` empty → uniform restart vector.
- **Personalized PageRank**: `seeds` non-empty → restart mass concentrated on
  seeds; scores reflect connectivity *to the seed set*.
- Operates on the existing `ProjectedGraph` (`src/graph.rs:42-132`), so type and
  predicate filters from `project()` apply for free (e.g. PageRank over only
  `WebApplication` entities, or only `depends_on` edges).

### Algorithm

Standard power iteration on the row-normalized adjacency of the projected
`DiGraph<i64, i64>`:

1. Initialize rank vector to the restart distribution `r0` (uniform or seeded).
2. Iterate `r' = (1 - d) · r_restart + d · Mᵀ r`, where `M` is row-stochastic.
3. Handle dangling nodes (no out-edges) by redistributing their mass to
   `r_restart`.
4. Stop on `‖r' - r‖₁ < tolerance` or `max_iters`.

Edge weights (predicate IDs today) are uniform in v1; **per-predicate weighting**
is a tuning hook (see Open Questions).

## Write-back as triples (the Quipu-native twist)

Per `vision.md` §9, a PageRank run may persist results:

```turtle
<entity> quipu:pageRank "0.0421"^^xsd:double .
```

Written via `Store::transact()` with the run's transaction timestamp, making
scores:

- **SPARQL-queryable** — `ORDER BY DESC(?pr)` for "top entities".
- **Bitemporal** — score history per entity; "importance trajectory" via
  `as_of` / `valid_at` time-travel.
- **Reasoner-visible** — datalog rules can fire on importance thresholds
  (e.g. `quipu:pageRank > X ∧ fewFacts → quipu:underDocumented`).

This is something HippoRAG's ephemeral, LLM-built graph cannot offer.

## Interfaces

| Surface | Change |
|---------|--------|
| `tool_project` (`src/graph.rs:206-286`) | Add `"algorithm": "pagerank"` / `"ppr"`; inputs `seeds[]`, `damping`, `max_iters`, `tolerance`, optional `persist: bool` |
| CLI | `quipu project --algorithm pagerank [--seed IRI ...] [--persist]` |
| REST | `POST /project` `{ "algorithm": "pagerank", ... }` |
| `tool_impact` (`src/mcp/impact.rs`) | Optional `rank_by_ppr: bool` → re-rank `reached[]` by PPR seeded at the root |
| Context pipeline (`src/context/mod.rs`) | Optional PPR re-rank of expanded neighbors before `max_entities` truncation |

## Episode-driven use cases

- **Emergent importance** — global PageRank surfaces `kota` as central because
  many deploy/rebuild episodes reference it; powers a "top entities" panel, web
  UI node sizing, per-type centrality in the schema inspector.
- **Query-relevant multi-hop** — PPR seeded by vector/text hits ("traefik
  issues") surfaces `kota`, the cert service, the bead that last touched it —
  even when not textually similar.
- **Episode-impact delta** — run PPR before/after an episode's nodes to answer
  "which entities did this episode make more central?", tying provenance to
  importance.
- **Temporal PageRank** — `as_of` past transactions: "`kota`'s importance rose
  after the rebuild episodes."
- **Counterfactual PageRank** — via `Store::speculate()`: "if we retire `koror`,
  how does importance redistribute?"

## Performance

- Global PageRank: run on demand or amortized; persist scores to avoid recompute.
- Personalized PageRank for interactive queries: compute over the **induced
  k-hop subgraph** around seeds, cap `max_iters` (~20) with early stop. Avoid
  whole-graph iteration on the query path.

## Testing

- Reuse fixtures in `src/graph.rs` tests (`graph.rs:289-413`).
- Convergence on cyclic and acyclic subgraphs; dangling-node handling.
- Ordering sanity vs `in_degree()` baseline (PageRank should rank hubs higher but
  diverge on multi-hop structure).
- Personalization: seeded run ranks seed-adjacent nodes above distant ones.
- Persistence round-trip: `--persist` writes `quipu:pageRank`, SPARQL reads back.

## Phasing

1. **Primitive** — `page_rank()` + `PageRankConfig` + tests. Standalone.
2. **Exposure** — `tool_project` algorithm branch + CLI + REST.
3. **Write-back** — optional `persist` → bitemporal `quipu:pageRank` triples.
4. **Re-ranking** — wire into `tool_impact` and the context pipeline.
5. **Temporal / counterfactual** — `as_of` and `speculate()` variants.

## Open questions

1. **Per-predicate edge weights** — uniform v1, or weight by predicate from the
   start? (Affects walk semantics materially.)
2. **Persist by default?** Always-on `quipu:pageRank` facts grow the log; or
   persist only on explicit `--persist`.
3. **Dangling-mass policy** — redistribute to restart vector (chosen above) vs
   uniform; document the choice as it changes scores.
4. **Subgraph induction depth** for interactive PPR — fixed k, or budget-driven?
