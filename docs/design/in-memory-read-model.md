# Design: In-Memory Read Model — query in memory, write to SQLite

> **Implementation status (2026-08-07):** 🚧 **Phases 1–3 landed; 4–5
> designed.** The read model is **on by default** and SPARQL joins run through
> it. Every number below was measured on this branch, native x86-64,
> `--release`, `--no-default-features`. The model is exercised by
> `examples/mem_read_model.rs`; the baseline it is compared against is
> `examples/scale_bench.rs`.
>
> `Store` carries a memoizing `TermCache` (`quipu-yzf`),
> `src/store/read_model.rs` holds the `ReadModel` (`quipu-d6x`), and `eval_bgp`
> hash-joins through it for multi-pattern BGPs (`quipu-syt`). The §1 table is
> the original SQL baseline the phases are measured against.

**Thesis:** SQLite should be Quipu's durable write log and archive. Queries
should be answered from an in-memory read model built over it. Measured, that
converts the 2-hop join from quadratic to linear — measured at 0.26 ms against
a SQL path that times out on the same store — at a cost of ~320 bytes per triple
of resident memory.

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

Measured on the same stores, **warm page cache**:

| Episodes | `project()` | `impact()` 3 hops | SPARQL 2-hop |
|---:|---:|---:|---:|
| 1,000 | 13.0 ms (3,054n / 8,050e) | **1.1 ms** | 4,709 ms |
| 2,000 | 24.2 ms (6,054n / 16,050e) | — | 20,630 ms |
| 4,000 | 48.5 ms (12,054n / 32,050e) | **1.8 ms** | 84,444 ms |
| 10,000 | 134.5 ms (30,054n / 80,050e) | **4.3 ms** | timeout |

**Correction, worth stating plainly:** an earlier revision of this document
reported `project()` at 118 / 745 / 2,768 ms and called it "roughly O(n^1.3)".
Those were **cold** measurements — first touch of an 83 MB file — and the
apparent superlinearity was page-cache cost, not an algorithmic property. Warm,
`project()` is linear (1.86×, 2.00× and 2.77× for 2×, 2× and 2.5× the data) and
about 20× faster than the figure that was published. It does not resolve per
fact; it builds the petgraph from term ids alone.

The SPARQL column is `examples/scale_bench.rs` throughout, so it is comparable
with §1 rather than being a second measurement of the same query in a different
context.

What remains true of `project()` is that it is **uncached** and pays a full scan
per call. Fine against a small derived graph, not against a large ROOT — scoping
it to a named graph composes with §7 and is tracked as `quipu-tz5`.

So the codebase already contains a full in-memory materialization
(`graph::project`) and an anchored walk that scales (`impact`). What it lacks is
those being available to the query language. An agent writing SPARQL gets the
slow path; an agent calling `tool_impact` gets the fast one, for what is often
the same question.

## 3. The read model

Three permutation indexes over current facts plus a two-way term dictionary —
the same access patterns `facts`' SQL indexes serve, held in memory:

```rust
pub struct ReadModel {
    graph: i64,                                  // the graph this covers; 0 is ROOT
    spo: HashMap<i64, Vec<(i64, Value)>>,        // <s> ?p ?o
    pso: HashMap<i64, Vec<(i64, Value)>>,        // ?s <p> ?o
    pos: HashMap<(i64, Vec<u8>), Vec<i64>>,      // ?s <p> <o>  /  ?s a <T>
    osp: HashMap<Vec<u8>, Vec<(i64, i64)>>,      // ?s ?p <o>
}
```

`osp` exists because SQL serves `?s ?p <o>` from `idx_vaet`; without it the
model would have to scan for that shape, making it a regression rather than a
speedup.

Keyed by term id throughout, holding no strings: the `id <-> IRI` dictionary
lives on `Store` (Phase 1) and is shared by every read path rather than
duplicated per model.

`eval_bgp` then becomes: resolve each pattern against the matching index, and
join on shared variables with a **hash join** — build a set from the smaller
side, probe with the larger — instead of re-querying per row. Join **ordering**
also becomes available for free, because index cardinalities are now known
without a round-trip.

The dictionary matters as much as the indexes. Two thirds of the prototype's
build cost was per-term `store.resolve()` calls, and in the query path those same
round-trips are what make row binding expensive. Resident, they are hash lookups
— which is why Phase 1 shipped first and why Phase 2 came in faster than the
prototype it replaced.

## 4. Measured

`examples/mem_read_model.rs`, warm page cache, same stores as §1. This drives
the real `ReadModel` as of Phase 2; the figures below replace the standalone
prototype's.

| Episodes | Triples | Build | Resident | **2-hop hash join** | SPARQL equivalent |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 17,150 | 19.3 ms | 8.7 MB | **0.022 ms** (1,000 rows) | 4,709 ms |
| 2,000 | 34,150 | 40.5 ms | 14.5 MB | **0.082 ms** (2,000 rows) | 20,630 ms |
| 4,000 | 68,150 | 78.8 ms | 24.7 MB | **0.101 ms** (4,000 rows) | 84,444 ms |
| 10,000 | 170,150 | 246.5 ms | 55.5 MB | **0.259 ms** (10,000 rows) | timeout |

Everything is linear. Build is ~1.45 µs/triple; the join is ~25 ns per candidate
edge. Resident growth is **~320 bytes/triple** at the margin — the per-triple
figure the example prints is higher at small sizes because RSS includes fixed
process overhead.

The comparison understates the gap: the hash join returns **every** row (10,000
at 10k episodes) while the SPARQL query had `LIMIT 100` and still timed out.

Two notes on how these moved:

- **Faster and smaller than the prototype**, which built at 337 ms / 59.7 MB for
  the same store. The model no longer carries its own term dictionary — that
  lives on `Store` as of Phase 1 and is shared by every read path, so Phase 1
  paid for part of Phase 2 before it was written.
- **Build time above is warm.** Cold, the 10k build is dominated by reading
  83 MB off disk — page-cache cost, not indexing cost.

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
| Alias / `canonical_id` rewriting | ⚠️ | Covered by the attachment check — `canonical_id` is the identity without them |
| A write holding an open savepoint | ❌ | The policy guard queries staged rows the model has not seen |
| A graph past the size budget | ❌ | Building it would cost more memory than configured |

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

The cache is **bounded** (`quipu-h03`): `DEFAULT_TERM_CACHE_LIMIT` admits
500k terms — roughly 175 MB at the measured ~350 bytes/term, against the ~30k a
10k-episode store holds — adjustable via `Store::set_term_cache_limit`, where
`0` disables memoization outright.

At the cap it stops **admitting**, and does not evict. Every policy is correct
here, because a miss falls through to SQL; the only question is behaviour under
pressure, and eviction under a scan that touches every term degenerates into
thrash — constant work, no hits. Refusing admission keeps whatever warmed first,
which for these read paths is the hot set, and costs O(1) with no bookkeeping.

**Phase 2 — `ReadModel`. ✅ LANDED** (`quipu-d6x`). `src/store/read_model.rs`:
three permutation indexes over one graph's current facts, built via
`Store::build_read_model(graph)`, with `apply`/`apply_all` for incremental
maintenance. 12 tests, the load-bearing ones differential — after every write
shape, an incrementally-updated model must hold exactly what a rebuilt one
holds.

Deliberately **inert**: nothing consults it yet. It is not stored on `Store`
either, because residency and invalidation policy belong with the scope guard
that makes consulting it safe — that is Phase 3.

Three things the tests pin down, each of which would be a silent wrong answer:
retraction and tombstones must *remove* from all three indexes rather than
append; a repeated assert must not double-count, because SQL's `SELECT DISTINCT`
never produced duplicate rows; and a datum arriving with `valid_to` already set
is not current, so indexing it would make the incremental path disagree with the
build.

**Phase 3 — Route `eval_bgp` through it.** ✅ **LANDED, and ON by default**
(`quipu-syt`, `quipu-att`, `quipu-m9h`).

`eval_bgp` hash-joins through the model when the guard admits, falling back to
SQL otherwise. Three things had to be true before the default could flip:

1. **The join is a hash join.** Each pattern is evaluated once and joined on
   shared variables, rather than re-evaluated per accumulated row. That nested
   loop was the O(n²); making each evaluation cheap only shrank its constant.
2. **Writes maintain the model** instead of dropping it. The write path vouches
   for its complete change set when it can — it cannot when OWL inference
   extends the batch, or when functional-property supersede closes prior values
   with a bulk `UPDATE … WHERE v != ?` that never enumerates what it hit — and
   only then is the model rebuilt. A write to a *different* graph leaves a ROOT
   model untouched entirely.
3. **Only multi-pattern BGPs use it.** A single pattern is the shape SQL is
   already fast at, and routing it through the model made it pay to build one —
   a 0.12 ms → 320 ms regression on the most common query shape.

Measured, with no shape regressing:

| Episodes | Point lookup | Type scan | 2-hop join |
|---:|---|---|---|
| 1,000 | 0.11 → 0.14 ms | 4.6 → 5.4 ms | 1,016 → **38 ms** |
| 4,000 | 0.10 → 0.13 ms | 18.8 → 18.5 ms | 26,233 → **225 ms** |
| 10,000 | 0.16 → 0.12 ms | 56.2 → 46.8 ms | 173,803 → **560 ms** |

**27×–310× on the join**, now linear where it was quadratic. Those figures
include the model build, which the first join pays for.

Size is bounded by `DEFAULT_READ_MODEL_MAX_TRIPLES` (1M triples, ~320 MB),
checked with a `COUNT` so an oversized store never pays a build to discover it
is oversized. Past the ceiling queries keep the SQL path — slower on joins, but
the behaviour they already had.

Binding is shared with the SQL path through one extracted `bind_row`, so the two
cannot drift on the subtle part: a subject resolving to a blank node binds as
`Value::Str`, everything else re-looks-up its IRI to choose between `Ref` and
`Str`, and the predicate has no blank-node case at all.

**Two invalidation bugs, and the structural fix that retired both.** The
write-time policy guard queries *inside* the savepoint, against staged rows. A
model built there survived a rollback still holding facts SQL no longer had; and
a model cached *before* staging made the guard deny a write that was compliant.
The first fix dropped the model at both points — correct, and it forced a
rebuild per write. The real fix is to **suspend consultation for the duration of
a write**: the guard uses SQL, the model never observes staged rows, and there
is nothing to invalidate on rollback. That is also what makes maintenance
possible, because the model is still there when the commit lands.

**Phase 4 — Selectivity-ordered join planning** and `LIMIT` pushdown — **built
(quipu-0lr)**. The hash join evaluates every pattern once either way, so the
fold order is planned from MEASURED cardinalities (`join_plan` in
`src/sparql/triple.rs`): smallest first, then connected-smallest, cartesian
only when the query genuinely contains one — a pathological source ordering
folds the same joins as a good one. `LIMIT` is pushed through prefix-safe
subtrees (`Project`/`Reduced` only) into the BGP leaf, which stops the scan
once it has bound enough rows. Measured: the `LIMIT 100` type scan went from
linear (0.51 ms at 1k episodes, 100.2 ms at 10k) to flat (0.63 ms at 10k).

**Phase 5 — Scope to the derived graph. ✅ LANDED** (`quipu-nip`). The
resident slot became a per-graph map: the applicability guard admits any
SINGLE graph scope (`GRAPH <iri>`, a one-graph `FROM`, or the `graph`
request param), each graph builds its own model on first use, writes
maintain only the written graph's model, and the size budget bounds the
COMBINED resident set — so a ROOT past the budget keeps the SQL path while
a small derived graph stays resident (the §7 shape). Unions and `GRAPH ?g`
keep SQL: one model holds one graph and no per-row g. **Measured** on the
scale-bench store: a 10,000-triple derived graph beside a 170k-fact ROOT
costs **4.2 MiB resident (~433 bytes/triple)** — the ROOT model over the
same store would cost 70 MiB, and past the budget is simply never built.

Deliberately **not** in this plan: replacing SQLite, changing the EAVT schema,
or touching the write path. Storage is not the problem — 8.3 KB/episode, linear,
at 315–390 episodes/s is fine for an append-heavy log, and the bitemporal
governance model needs exactly those columns.

## 9. Open questions

1. ~~**Is the read model always-resident or lazily built per query?**~~
   **Settled:** built on first use, maintained across writes, bounded by a
   triple budget, and only for multi-pattern BGPs. A store past the budget keeps
   the SQL path.
2. ~~**Per-graph or whole-store?**~~ **Settled per-graph** (`quipu-nip`):
   §7's argument held, and the `GRAPH <iri>` case now takes the fast path
   rather than falling back.
3. **Does the fast path need to be visible in results?** A query answered from
   memory and one answered from SQL should be indistinguishable in content — but
   if they ever are not, that is a bug we would want surfaced rather than
   averaged away. A differential test mode is cheaper than a wire-level tier tag.
