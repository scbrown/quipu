# Design: In-Memory Read Model — query in memory, write to SQLite

> **Implementation status (2026-08-07):** 🚧 **Phase 1 landed; Phases 2–5
> designed.** Every number below was measured on this branch, native x86-64,
> `--release`, `--no-default-features`. The prototype read model is
> `examples/mem_read_model.rs`; the baseline it is compared against is
> `examples/scale_bench.rs`.
>
> `Store` now carries a memoizing `TermCache` (§8 Phase 1, `quipu-yzf`) — worth
> ~4× on the 2-hop join on its own. `eval_bgp` is otherwise unchanged and still
> does a nested-loop join; the §1 and §4 tables are pre-cache figures, kept as
> the baseline the remaining phases are measured against.

**Thesis:** SQLite should be Quipu's durable write log and archive. Queries
should be answered from an in-memory read model built over it. Measured, that
converts the 2-hop join from quadratic to linear and from 133 seconds to 0.15
milliseconds, at a cost of ~385 bytes per fact of resident memory.

---

## 1. The problem

`eval_bgp` (`src/sparql/triple.rs:35-45`) is a nested-loop join. For each triple
pattern, for each row accumulated so far, it issues a fresh SQL statement. The
binding code then calls `store.resolve()` and `store.lookup()` per result row per
pattern — two more round-trips each. A 2-pattern BGP whose first pattern yields
N rows costs N+1 statements plus ~2N dictionary lookups.

Measured (`examples/scale_bench.rs`, 3-node/2-edge episodes, 20 triples each):

| Episodes | Point lookup | Type scan | **2-hop join** |
|---:|---:|---:|---:|
| 500 | 0.03 ms | 2.8 ms | **1,187 ms** |
| 1,000 | 0.10 ms | 5.7 ms | **4,709 ms** |
| 2,000 | 0.11 ms | 13.3 ms | **20,630 ms** |
| 4,000 | 0.11 ms | 24.4 ms | **84,444 ms** |
| 10,000 | 0.12 ms | 58.2 ms | — |
| 20,000 | 0.22 ms | 145.9 ms | — |

Doubling episodes multiplies join time by ~4.1. Fitting `t = k·n²` predicts the
default 30s `query_timeout_ms` blows at ~2,510 episodes; observed, the join
completes at 2,000 and times out at 2,500.

Point lookups stay flat, which is why this has not bitten yet — bound-subject
reads are effectively free at every size tested. It is specifically **joining on
a shared variable** that degrades, which is the core operation of a graph query.

## 2. Quipu already has three read paths, and only one is broken

This is easy to miss, and worth stating plainly.

| Path | Mechanism | Complexity |
|---|---|---|
| `graph::project()` (`src/graph.rs:50`) | `current_facts()` → one SQL statement pulling **every** current fact into a `Vec`, then builds a petgraph `DiGraph` | O(store), materializes everything |
| `impact()` (`src/impact.rs:79`) | BFS using one indexed `entity_facts()` lookup per frontier node | **O(nodes reached)** |
| SPARQL `eval_bgp` | Per-pattern SQL, nested-loop join | **O(n²)** |

Measured on the same stores:

| Episodes | `project()` | `impact()` 3 hops | SPARQL 2-hop |
|---:|---:|---:|---:|
| 1,000 | 118 ms (3,054n / 8,050e) | **1.5 ms** | 6,779 ms |
| 4,000 | 745 ms (12,054n / 32,050e) | **3.2 ms** | 133,037 ms |
| 10,000 | 2,768 ms (30,054n / 80,050e) | **8.3 ms** | timeout |

So the codebase already contains a full in-memory materialization
(`graph::project`) and an anchored walk that scales (`impact`). What it lacks is
those being available to the query language. An agent writing SPARQL gets the
slow path; an agent calling `tool_impact` gets the fast one, for what is often
the same question.

## 3. The read model

Three permutation indexes over current facts plus a two-way term dictionary —
the same access patterns `facts`' SQL indexes serve, held in memory:

```rust
struct ReadModel {
    spo: HashMap<i64, Vec<(i64, Value)>>,        // <s> ?p ?o
    pso: HashMap<i64, Vec<(i64, Value)>>,        // ?s <p> ?o
    pos: HashMap<(i64, Vec<u8>), Vec<i64>>,      // ?s <p> <o>  /  ?s a <T>
    id_to_iri: HashMap<i64, String>,
    iri_to_id: HashMap<String, i64>,
}
```

`eval_bgp` then becomes: resolve each pattern against the matching index, and
join on shared variables with a **hash join** — build a set from the smaller
side, probe with the larger — instead of re-querying per row. Join **ordering**
also becomes available for free, because index cardinalities are now known
without a round-trip.

The dictionary matters as much as the indexes. Two thirds of the prototype's
build cost was per-term `store.resolve()` calls; in the query path those same
round-trips are what make row binding expensive. Resident, they are hash lookups.

## 4. Measured — the prototype

`examples/mem_read_model.rs`, warm page cache, same stores as §1:

| Episodes | Facts | Build | Resident | **2-hop hash join** | SPARQL equivalent |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 17,150 | 34.7 ms | 9.2 MB | **0.034 ms** (1,000 rows) | 6,779 ms |
| 2,000 | 34,150 | 63.6 ms | 15.1 MB | **0.086 ms** (2,000 rows) | 20,630 ms |
| 4,000 | 68,150 | 135.7 ms | 26.0 MB | **0.115 ms** (4,000 rows) | 133,037 ms |
| 10,000 | 170,150 | 337.3 ms | 59.7 MB | **0.294 ms** (10,000 rows) | timeout |

Everything is linear. Build is ~2.0 µs/fact and the join is ~30 ns per candidate
edge. Resident growth is **~350 bytes/fact** at the margin — the per-fact figure
the example prints is higher at small sizes because RSS includes fixed process
overhead — which is ~6 KB/episode.

The comparison understates the gap: the hash join returns **every** row (10,000
at 10k episodes) while the SPARQL query had `LIMIT 100` and still timed out. At
4,000 episodes the speedup on like-for-like work is roughly **900,000×**.

Two caveats on these numbers:

- **Build time above is warm.** Cold, the 10k build was ~2,650 ms, dominated by
  reading 83 MB off disk — page-cache cost, not indexing cost. The breakdown at
  10k warm: ~118 ms for `current_facts()`, ~82 ms to build the three indexes,
  ~180 ms for dictionary `resolve()` round-trips.
- **That 180 ms is an artifact of the prototype**, which calls `resolve()` per
  new term. A single `SELECT id, iri FROM terms` sweep replaces 30,063
  statements with one and should remove most of it.

## 5. Correctness — what the read model may and may not answer

This is the section that decides whether the idea is safe, and it is where the
work actually is. `current_facts()` returns **ROOT-scoped, currently-valid,
asserted** facts. That is a strict subset of what SPARQL can ask for. A read
model that silently answered outside its scope would be a correctness disaster
in exactly the dimensions Quipu exists to get right.

| Query dimension | In scope? | Why |
|---|---|---|
| Current facts, ROOT graph | ✅ | Exactly what `current_facts()` returns |
| `valid_at` / `as_of_tx` time travel | ❌ | The model holds no history; `valid_to IS NULL` was filtered at load |
| `GRAPH <iri>` / named graphs | ⚠️ | Needs a per-graph model, or `current_facts_in_graph(g)` |
| Overlays + tombstones | ❌ | Composition is a resolution rule, not a row filter |
| Attached DBs (`facts_source()` UNION) | ❌ | The model is built from one database |
| Alias / `canonical_id` rewriting | ⚠️ | Must be applied at build time or preserved at probe time |

**The rule: the read model is a fast path, never a substitute.** `eval_bgp`
should consult it only when the `TemporalContext` is current-time, the dataset
resolves to a single graph the model covers, and the store has no attachments —
and fall back to today's SQL path otherwise. Anything else keeps the existing
behaviour, unchanged and still correct.

That discipline is the same one the codebase already applies elsewhere: never
present an approximation as the precise thing. A query that time-travels is not
slower because of a missing optimization, it is slower because it is a different
question.

## 6. Invalidation — "write to SQLite" stays literally true

The requested mode is **writes go to SQLite; reads come from memory**. SQLite
remains the single source of truth, the durability boundary, and the thing that
`git`-like tooling, `quipu attach` and knowledge packs operate on. Nothing about
the write path changes.

Keeping the model current does not require rebuilding it. `Store::transact`
already threads committed datums through `after_commit_hooks`
(`src/store/ops.rs:154,345`), and there is precedent for exactly this pattern one
line below: `invalidate_policy_registry_if_governance` invalidates a cached
registry on the writes that affect it.

So:

- **Incremental apply.** A commit yields its datums; applying them to three
  HashMaps is O(datums) — microseconds, against an ingest rate measured at
  315–390 episodes/s. Full rebuild is the fallback, not the mechanism.
- **Retraction and tombstones** must remove from the indexes, not just append.
  This is where an incremental path earns its tests.
- **Cold start** pays one build: ~380 ms at 10k episodes warm.

## 7. Memory budget, and why this argues for distillation

At ~350 bytes/fact (~6 KB/episode resident):

| Scale | Facts | Resident |
|---|---:|---:|
| 10k episodes | 170k | 60 MB (measured) |
| 100k episodes | 1.7M | ~600 MB |
| 1M episodes | 17M | **~6 GB** |

A server can hold 100k episodes comfortably. **1M raw episodes cannot be an
always-resident read model**, and on `wasm32` — a 4 GB address space — it is out
of reach entirely.

That is not a defect in the design; it is the design telling us what the read
model should contain. Episodes are raw ingest. The valuable artifact is the
knowledge *derived* from them, and Quipu already has the machinery to produce it:

- `src/reasoner/` — Datalog rules deriving `affects` / `dependsOn` from raw
  EAVT, written back with `source = "reasoner:<rule-id>"` provenance.
- `graph.rs` Louvain with `persist: true` — consolidates emergent structure into
  stated `quipu:memberOfCommunity` facts.
- `src/derivation.rs` — `derivedBy`, an executable recipe for recomputing a fact.
- Named graphs / overlays — a derived layer that extends ROOT without mutating
  it.
- `src/context/` — the pipeline that actually serves agents.
- Knowledge packs — export a distilled subgraph as one attachable artifact.

**So the read model should cover the derived graph, not the episode log.** If
1M episodes distil to 10⁴–10⁵ derived entities, the resident model is tens of
megabytes and every join is sub-millisecond. The raw log stays in SQLite, queried
rarely, by anchored walks and time-travel — precisely the shapes that are already
fast or already need SQL.

This is what makes "query in memory, write to SQLite" coherent rather than just
a cache: the two layers hold different things, for different reasons.

## 8. Plan

**Phase 1 — Term dictionary memoization. ✅ LANDED** (`quipu-yzf`).

The dictionary round-trips turned out to be in the `eval_bgp` binding path
(`resolve()` + `lookup()` per result row per pattern), the `graph.rs` output
paths (per result *node*, not per fact — `project()` itself builds from term ids
alone), and `impact.rs` per frontier node. Building a dictionary per query would
have cost more than it saved for small result sets, so this landed as a
**memoizing `TermCache` on `Store`** consulted by `resolve()` and `lookup()`,
plus a `warm_term_cache()` bulk sweep for callers that will touch most of the
store.

The cache needs no invalidation, and that is a property worth stating: `terms`
is **append-only while a store is open** — `INSERT OR IGNORE` is the only write,
and `respace_file` `VACUUM INTO`s a copy and rewrites the *destination*, never
the open database. A mapping already observed cannot become wrong. Only positive
lookups are cached; a miss must stay a miss, because the next `intern` can
create it. The test `terms_table_is_append_only` is the tripwire if that
premise ever changes.

Measured (`examples/scale_bench.rs`, same stores):

| Episodes | 2-hop join before | after | Type scan before | after |
|---:|---:|---:|---:|---:|
| 1,000 | 4,709 ms | **954 ms** | 5.7 ms | 4.5 ms |
| 2,000 | 20,630 ms | **5,354 ms** | 13.3 ms | 8.9 ms |
| 4,000 | 84,444 ms | **21,886 ms** | 24.4 ms | 17.7 ms |

**~3.9–4.9× on the join, ~1.4× on the type scan, ingest unchanged**
(368–373 episodes/s against 315–390 before). Still quadratic — Phase 1 removes a
constant factor, not the nested loop. That is Phase 3's job.

One consequence to watch: the cache is unbounded, growing at roughly the term
count. ~7 MB at 30k terms is nothing; a 1M-episode raw log's ~3M terms would be
several hundred MB in a long-lived server. Tracked as `quipu-h03`, and another
reason §7's derived-layer scoping is the right target.

**Phase 2 — `ReadModel` behind a `Store` accessor.** Build, incremental apply,
invalidate. Tested against the SQL path for equivalence on the in-scope subset.
Not yet wired to the query engine.

**Phase 3 — Route `eval_bgp` through it.** Guarded by the §5 scope check, with
fallback to SQL. Hash joins plus selectivity-ordered patterns. The acceptance
bar is differential: for every query in the existing suite, the read-model answer
must equal the SQL answer.

**Phase 4 — Selectivity-ordered join planning** and `LIMIT` pushdown, now that
cardinalities are free.

**Phase 5 — Scope to the derived graph.** Per-graph read models so the resident
set is the distilled layer, with the episode log left in SQLite.

Deliberately **not** in this plan: replacing SQLite, changing the EAVT schema,
or touching the write path. Storage is not the problem — 8.3 KB/episode, linear,
at 315–390 episodes/s is fine for an append-heavy log, and the bitemporal
governance model needs exactly those columns.

## 9. Open questions

1. **Is the read model always-resident or lazily built per query?** Always-on
   makes the first query fast and costs steady memory; lazy costs ~380 ms on the
   first join after any write that invalidates. Recommendation: always-on for a
   server, gated by a config budget, with lazy as the fallback when the budget is
   exceeded.
2. **Per-graph or whole-store?** §7 argues per-graph, which also makes the
   `GRAPH <iri>` case work rather than fall back.
3. **Does the fast path need to be visible in results?** A query answered from
   memory and one answered from SQL should be indistinguishable in content — but
   if they ever are not, that is a bug we would want surfaced rather than
   averaged away. A differential test mode is cheaper than a wire-level tier tag.
