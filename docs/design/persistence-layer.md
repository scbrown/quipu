# Persistence without whole-graph residency

Status: design for review, 2026-09-14. No backend switch or production setting
change is authorized by this document. Source audit: `f1033d736e63`.

## Recommendation

Keep SQLite as the authoritative temporal store for the next design stage, but
stop treating an in-memory graph model as the prerequisite for efficient joins.
Design a bounded, disk-backed query execution path and a process-wide memory
budget first. Evaluate a RocksDB-backed Oxigraph projection as the principal
alternative, behind the same snapshot and semantic contract. Do not commit to a
replacement until the experiments below show which cost remains in storage.

This is a conditional recommendation, not a claim that SQLite will scale to a
billion facts. Existing evidence establishes expensive application working sets;
it does not establish a winning replacement. SQLite already has disk indexes,
WAL readers and demand-paged mappings. Changing the bytes beneath an evaluator
that collects all intermediate rows can preserve the dominant allocation.
Conversely, retaining SQLite while continuing per-binding SQL calls leaves a
serious join cost that smaller caches cannot solve.

The desired property is **memory bounded by configured working budgets and
output batches, rather than by total historical corpus size**. Some graph
algorithms and inference closures inherently need large state; they must spill,
run as separately budgeted jobs, or refuse explicitly. A small final `LIMIT`
must not be represented as a bound on all intermediate work.

## Evidence and its boundaries

There are three different evidence classes here:

1. **Current executable source**, pinned above: establishes allocation paths and
   scope guards, not their contribution to a running process's RSS.
2. **Controlled million-fact measurements**, with raw receipts: establish that
   query shape and concurrency can produce several GiB without writes. Their
   older binary must not be relabeled as the audited source revision.
3. **Operational observations and benchmark history**: identify workloads and
   failure modes to reproduce. They are not controlled backend comparisons.

The task's approximate 150k-triple description is not a capacity denominator.
A read-only monitoring sample during this review returned 990,183 graph facts
and 951,894,016 bytes of process peak RSS. Those are asynchronously scraped
metrics, not a phase-aligned resident-memory measurement; the peak resets with
process lifetime. Historical vectors, active vectors, ROOT facts, all-graph facts,
entities and transaction-log rows must each be counted separately.

### Controlled sizing input

The retained experiment from `aegis-f7mxxu`, audited in this review, used a
SQLite backup of the same synthetic fixture in every arm:

- 1,008,000 fact rows: 960,000 in one named graph and 48,000 in ROOT;
- 1,008,043 terms; 48,000 active vectors of 384 float32 dimensions;
- vector payload alone: 73,728,000 bytes, not an RSS estimate;
- copied installed server `0.5.1 / 692d700a3c67`, ONNX Runtime 1.29.0;
- embedding-on-write disabled; no write requests; two-core CPU quota and
  6 GiB memory limit; read pools of 1, 2 and 4;
- three serial queries, three bursts of four queries, then the same search
  sequence: 30 requests per arm. This is a read-herd surrogate, not a recording
  of production query-first traffic.

The GRAPH query enumerates the large named graph and returns 200 rows. The
paired FROM query returns the same rows in this fixture. Values below are GiB
(`/proc` kB divided by 1,048,576). Startup means the harness's first sample
associated with readiness, not a filesystem-cache-cold measurement.

| Arm | Startup RSS | After serial query | After query herd | After search herd | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| GRAPH, 1 reader | 0.148 | 1.478 | 1.498 | 1.436 | 1.725 |
| GRAPH, 2 readers | 0.148 | 1.473 | 2.392 | 2.393 | 2.751 |
| GRAPH, 4 readers | 0.150 | 1.438 | 3.894 | 3.931 | 4.478 |
| GRAPH, 4 readers, no vectors | 0.149 | 1.484 | 4.059 | 4.060 | 4.626 |
| GRAPH, 4 readers, PSS repeat | excluded | 1.473 | 4.044 | 4.045 | 4.595 |
| FROM, 4 readers | 0.149 | 0.152 | 0.155 | 0.331 | 0.331 |

The excluded startup cell was 2,084 kB: the sampler captured process startup
before readiness and the harness reused its latest sample. Treating it as a
ready server's baseline would manufacture a growth factor. Other startup
samples remain approximate; future probes must sample synchronously after the
readiness response.

The PSS repeat ended at 3.288 GiB PSS versus 4.045 GiB RSS. Anonymous RSS was
3,160,432 kB; file RSS was 1,081,348 kB but file PSS only 287,582 kB. Multiple
mappings of shared SQLite pages inflate RSS without consuming that much extra
physical memory. Anonymous allocation nevertheless dominates the increase.

All 180 requests returned HTTP 200; all 90 queries returned 200 rows. All 75
populated searches returned five results; 15 empty-vector controls returned zero.
All 30 paired GRAPH/FROM result objects matched. Concurrent query mean was
6.488 seconds for GRAPH versus 0.00156 seconds for FROM in the paired arms.
These are fixture timings under a quota, not production latency promises or
valid WatDiv performance results.

The original raw receipts and harness are recorded under `aegis-f7mxxu`.
A sanitized machine-readable extraction and independent repeat accompany this
review in [the evidence directory](persistence-evidence/README.md). No production database is published.

### Independent repeat during this review

The same fixture was recovered with SQLite backup, including WAL, and two arms
were repeated without writes. Copied binary `/version` again identifies
`692d700a3c67`; runtime is ONNX 1.23 in this repeat, not 1.29. Full build features
are in the receipts; the installed binary's compiler profile was not independently
recovered, so absolute timing is not used to rank builds. Both runs used a
6 GiB scope and 200% CPU quota, four readers and a private loopback listener.

| Repeat | Ready sample RSS | Serial query RSS | Query-herd RSS | Final RSS | Final PSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| GRAPH | 0.143 | 1.430 | 3.934 | 3.935 | 3.180 |
| FROM | 0.137 | 0.140 | 0.142 | 0.339 | 0.333 |

All 60 requests returned HTTP 200; all 30 paired result objects matched exactly.
The repeat supports the same allocation discriminator. It is neither a current
production snapshot nor an exact replay of the query-first workload. Production
RSS attribution, latest-build performance and a 24-hour tail remain unmeasured.
The early 150k estimate, a million-fact synthetic test and a 10M ingest ledger
cannot be silently substituted for those missing measurements.

### Existing scale work: what can be used

The `aegis-tydvlg.5` programme and `aegis-j0yaxj.2` are inputs, not completed
1M–1B results. An ingestion diagnostic measured 200,000 synthetic triples in
3.89 seconds versus a real WatDiv prefix in 217.96 seconds on the same host.
That demonstrates workload sensitivity; it does not justify extrapolating one
rate linearly to a billion triples. `aegis-3sau5a` additionally records an ingest
rate decline between 4.4M and 6.8M facts whose cause was unresolved.

The completed 10M ingest ledger contains two exit-zero runs of server version
0.3.38: 3,156.11 and 2,848.91 seconds, each with 10,916,460 live facts and
3,227,811,840 store bytes. Both use N-Triples SHA-256
`7cfe0341d578a677d3b5d562eaaf94d67aff8587d9e0ef3d83cc82765b77cddd`.
The three-row excess over 10,916,457 declared input triples requires explaining
before treating store count as an input-count identity; it is not an RSS result.
These receipts lack a build-profile field, so retain their version and conditions
and do not use absolute timing to compare against a new build. Two exit-143
partial runs (10M and 100M) are excluded from completed-result claims.

A checked-in generated 1M artifact pin exists at
`c158998c66e11b33bc56cf7fa3cbc9e69c1c36bf9bdd1bab447d8a64e2d8da75`
with 1,091,718 declared triples. This is a pin, not a completed query checkpoint.
The existence of the artifact on disk has not been independently verified here.

The early “1M” query results were taken on a prefix of a 10M dataset. That cuts
join partners out of the graph. Those timings, including zero-result complex
queries, are excluded. Use published archives at 10M/100M/1B, and generate once,
normalize and hash a complete 1M dataset. `benchmark/public/watdiv_ingest.py`
and `watdiv_instantiate.py` implement the fixture work; conformance JSON files
are not memory/latency ledgers. No valid large-scale backend ranking is claimed.

`Cargo.toml:166` explicitly enables `oxigraph/rocksdb` for `oxicompare`.
An Oxigraph `Store::new()` in-memory comparison is inadmissible for persistence.
`src/bin/oxi_compare.rs` alternates arms and requires repeated timings and
verified loads. Both products share RDF/parser dependencies: its comparison is
storage plus evaluation, not independent parser engines. A same-process peak
RSS cannot attribute memory to one engine; memory comparisons need separate
processes even if timing comparisons use the existing interleaved harness.

Historical memory observations distinguish warm-up, plateau and continued
growth. A fixed-corpus lazy-value change reduced RSS from 304.0 to 144.3 MB;
it did not prove a production peak reduction. Other process lifetimes showed
multi-GiB anonymous growth and recycling. The controlled no-write experiment
shows a sufficient allocation mechanism, not the cause of every historical
incident. Neither a short stable tail nor correlation with facts written proves
or disproves a leak. Preserve process identity, uptime and deployment boundaries.

## What actually becomes resident

Line references below refer to the audited revision; comments sometimes describe
older behavior. In particular, the book's ROOT-only model description and the
module's “nothing consults this yet” text do not describe current guards.

| Component and source | Trigger and contents | Lifetime / scaling | Necessity |
| --- | --- | --- | --- |
| SQLite tables/indexes, `src/schema.rs:15`, `src/store/open.rs:185` | Open establishes schema and migrations; B-tree pages are read as needed, not a whole-store Rust load | Disk authoritative; page residency follows access | Persistent identities, temporal log and atomic metadata are required; full heap residency is not |
| WAL readers, `src/store/open.rs:54`, `src/server/handle.rs:186` | Each reader opens its own Store; requests 256 MiB mmap and ~31.25 MiB page cache | Per-connection caches; mapped pages can be physically shared | Separate connections serve concurrent reads; these particular budgets are choices |
| Term memo, `src/store/terms.rs:17` | Resolve/lookup lazily populate two maps, duplicating IRI strings; explicit warm sweep at line 216 | 500,000-entry admission cap per Store; no eviction | Convenience; cache misses read SQL correctly |
| Read model, `src/store/read_model/index.rs:54` | First eligible multi-pattern query scans one graph into an arena and four pointer indexes | Cached per graph, per Store; transaction deltas catch up at `read_model.rs:79` | Join acceleration, not authoritative data |
| Lazy values, `src/store/read_model/index.rs:26` | Scalars/references inline; heap-backed values stored as row IDs and fetched when needed | Pointer arena remains resident; decoded query values are additional | Already implemented; “make attributes lazy” is not a new proposal |
| Graph projection, `src/graph.rs:70`, `:205` | Algorithms scan full graph facts before building ID-only petgraph and two node maps | Last projection cached per Store; transaction-stamped Arc can outlive replacement | Graph algorithm state; full fact decoding before projection is avoidable |
| SQLite vectors, `src/vector.rs:139` | Every search streams current/temporal blobs, decodes one vector, collects all `(id, score)`, sorts, fetches text for survivors | O(V) score array and O(d) vector scratch per active search, plus top-K output | Exact scoring may scan all candidates; full sorting is optional |
| ONNX, `src/onnx_embedder.rs:19` | Model/tokenizer loaded for provider; session behind a mutex, provider cloned into readers | Shared provider, tensor/batch scratch; CPU arena and memory-pattern retention explicitly disabled at line 42 | Embedding capability, not RDF storage; not one full model per reader |
| OWL, `src/owl_materialize.rs:261` | Full materialization loads ROOT plus inferred companion, clones premises and builds a dedup set; delta path uses changed facts | Per pass; closure output persists; 64-pass ceiling is not a byte ceiling | Closure semantics required; full repeated preload is an algorithm choice |
| Datalog, `src/reasoner/reactive.rs:33`, `evaluate.rs:370` | Resident rules/dependency index; affected rules load scoped facts and derivations into evaluation state | Rule catalogue plus request/write working sets | Rule semantics required; rederive-and-diff is replaceable with incremental state |
| SPARQL, `src/sparql/triple.rs:56`, `pattern.rs:199` | Vec bindings at operators; unsafe LIMIT pushdown evaluates inner rows first; grouping clones rows at line 337 | Per request; concurrent allocations multiply; allocator may retain freed pages | Result semantics required, eager full materialization is not |
| Resolution labels, `src/resolution/mod.rs:209` | A batch resolution builds a Vec of every current label in the composition; candidates resolve IRIs lazily | Per resolution batch, O(labels and label bytes) | Fuzzy matching needs candidates; full label preload is a replaceable index strategy |
| Sharing, `src/pack.rs:158`, `share_import.rs:219` | Export bytes/text, sorted dedup set, canonical output; import parses triples into Vec and rewrites them | Per export/import; independent of query cache size | Canonical identity/validation required; whole-input heap copies are optional |

The mmap number is a mapping limit, not an allocation promise. SQLite's
[mmap documentation](https://www.sqlite.org/mmap.html) explains demand access
and its platform constraints. Already enabling mmap cannot eliminate Rust
binding vectors. Measure RSS, PSS and cgroup memory separately.

### Bounds that are not hard memory bounds

The read-model default is 1,000,000 triples across resident graphs **within one
Store** (`read_model/applicability.rs:103`). An already-resident graph returns
true without recounting; delta application can grow it. Arena removals leave
`None` slots while insertion appends (`index.rs:272–306`). Consequently a
constant live-triple count does not imply constant arena capacity under churn.
These are source-derived growth risks, not measured attribution of a live leak.

The historical ~320 bytes/triple estimate predates compact lazy pointers and
must not be reused as current capacity. Similarly, 500k dictionary entries is
not a byte limit: term lengths vary, both maps allocate strings, and each reader
has its own memo. Lowering a nonzero term cap does not evict existing entries
(`terms.rs:109`). `adopt_read_config_from` copies policy/provider configuration,
not term/model cache limits, so library setters on the writer are not proven
server-wide controls.

There are already query limits (`src/config.rs:88`): 30 seconds, 10,000 final
SPARQL rows and 1,000,000 intermediate join rows by default. They matter, but
row counts do not bound bytes per binding or all simultaneous operators. The
million-fact GRAPH fixture fits below the intermediate-row ceiling while still
allocating GiB. A byte budget complements these guards; it does not replace
their timeout/cancellation behavior.

A useful accounting equation is:

```text
physical memory ≈ shared file PSS + provider/base heap + writer state
                + Σ(reader caches + models + projection state)
                + Σ(active request intermediates + response buffers)
                + inference/import jobs + allocator retention
```

Admission must budget bytes and active jobs together. Limiting the read pool
alone trades memory for queue latency; it does not bound one pathological query.

### Concurrency and correctness

The FairMutex serializes the writer; WAL readers do not normally queue on it.
The pool has separate Stores, not one shared read-model instance. Its per-Store
memoization can multiply anonymous memory even when mapped database pages are
shared. The built-in vector path can use readers; external vector backends
currently disable that pool because their boxed providers cannot be copied
safely (`src/server/handle.rs:189`). Moving vectors changes concurrency semantics
as well as storage.

[SQLite WAL](https://www.sqlite.org/wal.html) supports overlapping reads and a
writer, but long read transactions can delay checkpoint completion. Bounded
streaming must therefore also bound snapshot lifetime and expose WAL retention.
An iterator is not automatically a resource fix if a slow client pins its
snapshot indefinitely. Buffer bounded output batches, cap duration, and cancel
upstream work on disconnect.

Any replacement/shared cache must preserve one coherent request snapshot,
valid-time and transaction-time queries, graph/dataset selection, overlays,
tombstones, inferred companions, labels and attached term spaces. The current
transaction-stamped model solves a known stale-reader problem; a process-global
cache must be keyed by snapshot and graph semantics, not just “latest graph”.
Audit snapshot lifetime explicitly; the presence of multiple SQL statements and
a freshness stamp alone is not proof of repeatable-read semantics for a complete
request. SQL fallback must remain authoritative when a cache cannot prove scope.

## Alternatives

| Option | Concrete benefit | Concrete cost / risk | Decision |
| --- | --- | --- | --- |
| Disk-resident SQLite execution | Reuse temporal tables, B-tree indexes, transactions, tooling and existing packs | Query planner/executor work; current SQL path performs per-binding calls; index additions increase disk/write cost | Preferred first candidate; benchmark as a changed execution plan, not pragma tuning |
| Raw RocksDB temporal KV | Ordered ID-key scans, atomic batches, tunable disk/heap tradeoff | Must implement all permutations, history, visibility, dictionary and graph metadata invariants; compaction and write amplification | Reject as first migration; strongest rewrite cost without query-engine reuse |
| RocksDB-backed Oxigraph | Reuse mature disk quad indexing and SPARQL execution; independently benchmark current-state projection | Temporal log, governance, share identity and provenance remain Quipu responsibilities; two stores need snapshot coherence | Preferred challenger as a rebuildable projection, not immediate authority |
| LMDB-backed ordered indexes | Demand-paged read access and cheap read transactions suit read-heavy local stores | One writer remains; custom RDF/temporal index layer, map growth and long-reader page retention; portability work | Retain as second challenger if bounded SQLite misses targets and write profile fits |
| Sled-class pure Rust KV | Attractive integration and build footprint | New schema/query engine plus durability and format qualification; upstream still calls sled beta | Reject for authoritative persistence at this stage |
| Hot/cold hybrid over SQLite | Keep frequently used small graph/index subset; cold history/packs stay on disk | Cache admission, eviction, snapshot invalidation and cold latency must be explicit; arbitrary graph union cannot substitute for ROOT | Include in preferred design as optional acceleration |
| Separate remote database/service | Independent scaling and resource isolation | Gives up simple embedded/offline operation; distributed failure and backup coordination | Reject for default embedded product; optional deployment mode only |

Upstream facts, checked for this review: Oxigraph's
[architecture](https://github.com/oxigraph/oxigraph/wiki/Architecture) uses
RocksDB-backed quad indexes and repeatable-read operations. It does not supply
Quipu's complete temporal/governance model. RocksDB has
[memory consumers beyond block cache](https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB):
memtables, filters/indexes and pinned iterator blocks. Budget them together;
“on disk” is not “constant RSS”. LMDB's
[documentation](https://lmdb.readthedocs.io/en/latest/) describes mapped access,
reader transactions, map resizing and the need to release long readers. Sled's
[own project page](https://github.com/spacejam/sled) labels it beta and recommends
SQLite when reliability is primary. These are architectural inputs, not measured
Quipu speedups.

### Preferred SQLite design

Use SQL cursor scans over graph/predicate/subject/object keys, pushing joined
BGP work into one planned operation where semantics permit. Prototype graph-first
covering/partial indexes for current facts on an isolated copy, with
`EXPLAIN QUERY PLAN`, write amplification and database-byte measurements. Current
schema already has attribute/value/transaction and graph-leading indexes; do not
add a second index from a name-only inventory. Do not replace the temporal log
with a current-state table. An optional transactionally maintained current-fact
projection can accelerate present-time queries while history stays queryable.

Build an iterator/batch boundary between storage and algebra. Keep compact term
IDs through joins, resolve display values at the boundary, and use SQL spill or
external sorting for large joins, DISTINCT and GROUP BY. Share immutable bounded
cache segments only after proving snapshot identity and invalidation. Put a
byte-based admission/eviction policy around graph models and projections; account
for holes/capacity, not just live entries. No whole-model build on a request that
already exceeds the budget.

Exact vector search can use a bounded top-K heap instead of retaining/sorting
all scores: O(K + d) working state, O(V log K) selection work, still O(Vd)
scoring. That is an algorithmic proposal, not a vector-backend switch. Preserve
ranking, ties, temporal filters and post-filter recall. An ANN index is a separate
proposal requiring measured recall and temporal deletion behavior.

OWL full materialization and canonical pack operations need separately reserved
memory and disk-spill paths. Stream fact IDs where possible, deduplicate using
external/indexed state, and make incomplete closure explicit. Moving inference
to a queue is not free: the read API must expose which transaction its derived
facts cover and avoid reporting stale inference as complete.

### Sharing, packs and deployment

Preserve three contracts separately: portable graph share, mergeable pack, and
lossless full-store recovery. `src/pack_full.rs:105` uses `VACUUM INTO` so a full
pack includes the WAL-visible committed state; `pack_restore.rs` distinguishes
restore (replacement) from unpack (merge). A SQLite main-file copy is not a
valid live backup. External sort can preserve canonical byte/hash ordering
without preserving today's whole-string implementation.

A non-SQLite authority cannot call a RocksDB directory a compatible `.qpack`.
Keep the existing SQLite interchange reader/writer or explicitly version a new
format, with feature negotiation and old-format import. Preserve term-space
identity, aliases, graph labels, valid/transaction times, vectors, stored shapes,
queries, operational tables and event cursor semantics. RDF-only export does
not contain everything needed to roll back a full store. Browser/Wasm SQLite
interchange is part of the portability cost, not an afterthought.

For an Oxigraph projection, SQLite remains the recovery authority. Record an
applied transaction watermark atomically with projection updates. Serve only
snapshots whose coverage is proven; otherwise use SQLite or return a clear
unavailable/stale result. Rebuild projection from a verified snapshot plus a
complete change log. Never claim cross-store atomicity from two successful
commits. Crash tests must cover a stop between the authoritative commit and
projection commit and between projection commit and acknowledgement.

Deployment must separate code availability from activation. Main-branch changes
can be automatically deployed; a future backend experiment therefore needs an
explicit disabled-by-default selection and no startup migration of the live
store. Package native dependencies for all supported targets, pin database-format
compatibility, and test opening old data before release. This document adds no
runtime code, config, migration or deployment step.

## Evaluation and decision gates

First reproduce the current application with a fixed database snapshot and
binary, then compare configurations. A synthetic million-fact fixture supplies
an allocation discriminator; a consistent production-sized backup supplies
real term lengths, history, graphs, vectors, rules and query distribution.
Keep the private backup private. Record counts, hashes and anonymized query-shape
summaries instead of publishing operational content.

For each candidate run isolated processes at concurrency 1/2/4/8, with identical
CPU/memory limits, no swap activity and recorded disk/thermal contention. Test:

- ready-idle RSS/PSS after synchronous readiness sampling;
- warmed single-pattern lookups, label/type joins, bounded search, GRAPH/FROM,
  path queries, GROUP BY/DISTINCT and deliberately large intermediates;
- a fixed query-first mix in synchronized bursts, followed by a quiet tail;
- write/read interleave, assertion/retraction churn and a large promotion;
- full and delta reasoning, graph projection, share/export/import/restore;
- separately, valid WatDiv 1M/10M/100M/1B checkpoints only when admitted by the
  existing benchmark protocol. Report unrun scales as unmeasured, not impossible.

Record actual build/profile/features, dataset hash, active/history counts,
request results/digests, wall/CPU, p50/p95/p99 over adequate repetitions,
process RSS/HWM/PSS/anonymous memory, cgroup peak, faults, disk bytes and WAL or
compaction growth. Use a fresh process for each cold-start arm; do not drop the
host's global page cache. Distinguish process-cold from OS-cache-cold. Baseline,
warm-up, herd and tail must use the same PID without a deployment in between.

Proposed acceptance gates, for discussion rather than claimed results:

1. **Correctness:** zero unexplained differential mismatches across temporal,
   graph, label, inference and sharing suites; interrupted work must never look
   like successful empty output. Check result multisets and blank-node semantics.
2. **Memory:** within an explicit 2 GiB process working budget at the million-fact
   mixed read workload with four active readers; separately bound inference and
   imports. This is an engineering target, not a supported production limit.
3. **Latency:** no more than 10% p95 regression on the accepted ordinary query
   mix; complex outliers listed individually, not hidden in a global average.
4. **Growth:** repeat a fixed-corpus read mix and assertion/retraction churn for
   at least 24 hours. After warm-up, explain retained-memory slope with live
   objects/capacity measurements; a short flat interval is insufficient.
5. **Durability:** kill/restart, recovery and restore drills preserve every
   acknowledged transaction; compare event cursors and metadata, not just triples.
6. **Operational cost:** publish startup/rebuild time, pack/restore time, peak
   temporary disk, steady disk per logical fact and write throughput under reads.

If bounded SQLite meets these gates, keep it and stop the backend migration.
If it fails because indexed disk access remains dominant after bounding
intermediates, compare the Oxigraph projection on equal durability and semantics.
Choose a new authority only if it beats the target workload and the operational
cost of the semantic adapter is acceptable. No ranking can be filled from the
currently missing large-scale receipts.

## Migration outline and rollback

1. **Observability first:** add allocation/cache/admission metrics and snapshot
   contract tests in a separately authorized implementation. No data migration.
2. **Bounded SQLite path:** opt-in evaluator/cache budgets; differential replay
   on copies. Roll back by selecting the old evaluator, preserving database format.
3. **Optional projection experiment:** separate storage directory, read-only
   shadow answers, explicit watermark. Roll back by disabling/deleting only the
   rebuildable projection after retaining diagnostics; SQLite stays intact.
4. **Only after a new decision:** snapshot at transaction T, bulk-convert all
   required tables, replay changes through a final watermark, quiesce writes,
   validate counts/hashes/semantic probes and atomically switch service selection.
5. **Rollback after activation:** preserve the old store and old binary. Before
   new writes, switching back is straightforward. After new writes, rollback
   requires a tested reverse change replay or a compatible authoritative journal;
   restoring T alone loses acknowledged writes and is not rollback. If neither
   replay path is proven, remain read-only or do not cut over.

Document the incompatible-version boundary explicitly. A deployment rollback
must not open a newer on-disk format with an old binary. Keep migration artifacts
and replay positions until a restore rehearsal verifies the retention policy.

## Non-goals and decisions still needed

No implementation, backend switch, production restart, memory-limit reduction,
reasoning-default change or new published performance
claim is included. Hardware purchases do not follow from these measurements.

Stiwi's decisions for a later implementation proposal:

- Is the initial target a 2 GiB four-reader service, a smaller embedded device,
  or a larger server? What latency and import throughput matter most?
- Must native, browser/Wasm and offline `.qpack` interoperability remain equally
  capable, or may a large-server backend have a narrower portability contract?
- Can derived graph/search projections be explicitly stale, and for how long?
  The default recommendation is coherent reads or an explicit refusal.
- Is the priority current-state query scale, temporal-history scale, vector
  search, or lossless sharing? Each selects a different bottleneck.
- What outage window and temporary disk budget are acceptable for a future
  conversion and rollback rehearsal?
