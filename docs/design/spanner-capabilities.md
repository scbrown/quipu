# Design: Spanner-class capabilities over any structured data

> **Implementation status (2026-08-31):** 🔮 **Design only.** This is an
> investigation of Google Cloud Spanner (state of 2025–2026, Spanner Graph GA)
> and a capability-by-capability mapping onto the quipu stack. Nothing in this
> document is implemented by this document; where a row says ✅ the capability
> already exists in quipu or camayoc and is cross-referenced. Follow-up work is
> tracked as beads (see §6).

## 1. Why Spanner, and what "not just rows" means

Spanner's headline 2025 capability is **Spanner Graph**: a
`CREATE PROPERTY GRAPH` catalog object that lets the engine *interpret*
existing relational tables as a property graph — virtual, never materialized,
no ETL, writes to the base tables immediately visible in the graph, queried
with ISO GQL and joinable back into SQL. One row participates in the
relational, graph, full-text, and vector models simultaneously, under one
transaction and one optimizer.

The ask for quipu is that capability **generalized past relational rows**:
declare how *any structured data* — CSV, JSON, SQLite today; result tables
and streams later — reads as governed graph. Spanner can only do this for
tables it stores. The stack already holds the right generalization: **R2RML
is Spanner's mapping idea for rows, and RML is R2RML for arbitrary structured
sources.** Camayoc's governed RML-subset executor
(`camayoc/docs/design/rml-executor.md`, live since aegis-07hmc) already
fetches a governed mapping from quipu, reads a bounded structured source, and
commits deterministic quads through the SHACL-governed `/knot` lane. What is
missing is not the mapping language — it is the *capabilities Spanner layers
on top of the mapping*: edges from foreign keys, freshness without re-ETL,
dynamic schema with a governed precedence rule, and a change feed for
downstream consumers.

## 2. Spanner's mapping model, and its RDF mirror

How `CREATE PROPERTY GRAPH` works, term by term, against what the RML lane
already has:

| Spanner concept | Semantics | RML/quipu equivalent | Status |
|---|---|---|---|
| Node table | One row = one node; columns = properties; table name = default label | `rr:TriplesMap` + `rml:logicalSource`; `rr:class` emits `rdf:type` | ✅ executor v1 |
| `PROPERTIES (cols)` / `NO PROPERTIES` | Property subset selection | Predicate-object maps are already opt-in per column/reference | ✅ executor v1 |
| Element `KEY` (defaults to PK) | Node identity | `rr:subjectMap` with `rr:template` over key references | ✅ executor v1 |
| Edge table: `SOURCE KEY … REFERENCES` / `DESTINATION KEY … REFERENCES` | FK-shaped column pairs become edge endpoints; a join table is the canonical edge table | `rr:parentTriplesMap` + `rr:joinCondition` (referencing object maps) | ❌ **excluded from executor v1** |
| Shared label = enforced property-type signature | All element definitions with one label must expose same property names/types | SHACL `NodeShape` per class; quipu governance shapes | ✅ quipu SHACL, not yet applied to mapping output |
| `DYNAMIC LABEL (col)` / `DYNAMIC PROPERTIES (json)` | Label and properties are *data*; new types with zero DDL; **static definitions win over dynamic** | Low-trust quarantine planes (camayoc ingress discipline) | 🟡 planes exist; the precedence rule does not |
| Graph over SQL views | Curated graph over derived projections; `KEY` mandatory | Mappings whose logical source is a stored query / `rml:query` result | 🟡 `rr:SQL2008` over SQLite only |
| Virtual graph (no materialization) | Base-table writes immediately visible in graph | — | ❌ executor materializes once per invocation |
| GQL read-only; writes via DML on base tables | Graph is a read model; writes go through the governed relational path | Identical shape: mapped graphs are read models; writes go through `/knot` | ✅ by construction |

Two findings worth stating plainly:

1. **The camayoc v1 exclusions are exactly Spanner Graph's core.** Executor
   v1 deliberately excludes referencing object maps, joins, and dynamic
   predicates/graphs. Spanner's edge model *is* the referencing object map
   (`SOURCE KEY … REFERENCES` ≈ `rr:joinCondition`), and its dynamic-schema
   story is the dynamic-terms feature v1 refuses. The exclusions were correct
   for a bounded v1; they are now the roadmap.
2. **Quipu should not chase Spanner's virtualness literally.** Spanner's
   graph is virtual because Spanner owns the storage under it. Quipu does not
   own external CSV/JSON/SQLite sources, and a query-time fetch of an
   ungoverned source would bypass every ingress gate camayoc exists to
   enforce. The equivalent capability with quipu's discipline intact is
   **freshness-driven re-materialization**: the mapping registry knows each
   mapping's source hash and freshness contract, detects staleness, and
   re-runs the deterministic executor — converging on the same
   "base data changed → graph reflects it" behaviour, with provenance instead
   of magic. (§4.2)

## 3. Capabilities quipu already has, with the Spanner name for them

Investigation is only useful if it also says where we are ahead. Spanner's
docs give precise names and contracts for things quipu does informally;
adopting the *contracts* costs a documentation pass, not an engine.

- **Timestamp-bound reads.** Spanner offers strong reads, *exact staleness*
  ("read at T sees every transaction with commit ts ≤ T and none after — a
  consistent prefix of global transaction history"), and *bounded staleness*.
  Quipu's bitemporal EAVT log (`docs/book/src/concepts/temporal-model.md`)
  already serves `--valid-at` / transaction-time reads — and keeps history
  **indefinitely**, where Spanner GCs versions after a `version_retention_period`
  capped at 7 days. Quipu's `earliest_version_time` is the first transaction.
  Worth adopting: the explicit **consistent-prefix contract** wording on the
  read API, and a bounded-staleness mode for the in-memory read model and
  federation (`federated_from_config`), where "no staler than N" is a useful
  latency trade.
- **MVCC via commit timestamps.** Spanner stamps every row version with a
  TrueTime commit timestamp. Quipu stamps every fact with a `tx` id and
  timestamp in an append-only log — same mechanics, single-node so no
  TrueTime needed. Fork-at-any-event (`fork-at-any-event.md`) goes further
  than Spanner's PITR (restore-only) by making any past event a first-class
  branch point.
- **Interleaving / traversal locality.** Spanner's best practice — interleave
  the edge table under its source node, keep a secondary index interleaved in
  the destination for reverse traversal — is the classic argument for
  subject-clustered triple storage with inverse permutations. Quipu's
  `idx_eavt`/`idx_aevt`/`idx_vaet` covering indexes plus `idx_geav` are that
  design already.
- **Multi-model on the same rows.** Spanner: `TOKENLIST` + search index,
  `VECTOR INDEX` (ScaNN), both callable inside GQL, so retrieval joins graph
  patterns without a consistency seam. Quipu: LanceDB-backed vector search
  and graph projection over the same fact log
  (`architecture/vector-search.md`, `architecture/graph-projection.md`). The
  gap is full-text (no inverted index; see §5) and the seam-lessness — vector
  results and SPARQL results are combined by the caller, not inside one query.
- **FGAC / definer's-rights views.** Spanner has role-based table/column
  grants and *no native row-level security* — the documented pattern is
  definer's-rights views. Quipu's named-graph planes, group isolation, and
  guarded write lanes are already a stronger row(graph)-level story; stored
  queries running with declared scope are the definer's-rights analog.

## 4. Proposed capabilities (the gaps)

### 4.1 RML v2: edges from keys — referencing object maps

Admit `rr:parentTriplesMap` + `rr:joinCondition` (and only those two terms —
still no functions, no dynamic predicates) into the governed subset:

- **camayoc executor:** implement referencing object maps for same-source
  joins first (two triples maps over one logical source, the CSV/JSON/SQLite
  cases), then cross-source joins bounded by the existing byte/row limits.
  Join evaluation is hash-join over generated subject keys; determinism holds
  because both sides are already deterministic.
- **quipu shapes:** extend the aegis-9r8bp governance section in
  `shapes/governance.ttl` so a referencing object map is structurally valid —
  exactly one parent triples map, one or more join conditions each with
  exactly one `rr:child` and one `rr:parent` reference.
- **Spanner semantics to copy:** dangling-edge behaviour is *explicit*.
  Spanner only refuses an edge whose endpoint is missing if a constraint
  says so. The executor equivalent: a join that finds no parent row emits no
  triple (standard R2RML), and the invocation report counts unmatched joins
  so silence is visible.

### 4.2 Mapping freshness: re-materialization instead of virtual graphs

Give the mapping registry the *effect* of Spanner's virtual graph without
query-time source access:

- Each governed mapping already records `aegis:freshness` and a verified
  source hash. Add a `stale` verdict: source hash at last materialization ≠
  current verified hash, or freshness window elapsed.
- A `remap` operation re-runs the executor for stale mappings only; the
  idempotent `unchanged` outcome makes over-triggering harmless.
- Served reads on a mapped graph carry the materialization timestamp and
  source hash — the same "omitted rather than faked" freshness discipline
  yupana uses for code facts. A consumer who needs Spanner-strength freshness
  triggers `remap` and re-reads; one who tolerates staleness reads the stamp.

### 4.3 Dynamic schema with the static-wins precedence rule

Spanner's schemaless graph (`DYNAMIC LABEL` / `DYNAMIC PROPERTIES`) is the
governed store's quarantine plane wearing different clothes, and it ships one
rule quipu should adopt verbatim: **statically defined properties take
precedence over a dynamic property of the same name.** For quipu:

- Model-inferred and schemaless facts continue to land in low-trust planes
  (camayoc ingress discipline), never in governed graphs.
- A composed read that overlays a quarantine plane on a governed graph
  resolves conflicts *governed-wins*, deterministically — the overlay/
  tombstone machinery (`op = 2`) already composes views; this adds the
  precedence contract for same-subject-same-predicate collisions.
- Promotion out of quarantine stays what it is today: an explicit governed
  write, never a precedence flip.

### 4.4 Change feed: the transaction log as a consumer contract

Quipu's transaction log is already a change stream with no API. Spanner's
change-stream design says what the API should promise:

- A `changes` read surface: from transaction id T (or timestamp), return
  ordered records `(tx, sequence, op, graph, s, p, o-old/new)`.
- **Value capture modes** copied outright: `new_values`, `old_and_new_values`,
  `new_row` (full current entity state at commit, so consumers skip the
  read-back that Spanner docs call out as the anti-pattern).
- **Ordering contract** copied outright: per entity, records arrive in commit
  order; across entities, no promise. Heartbeats advance a watermark during
  quiet periods so a consumer can distinguish "idle" from "broken".
- First consumer: bobbin's index/embedding maintenance — re-embed exactly the
  entities whose facts changed, instead of rescanning. Second: the event-push
  delivery worker becomes a special case of this surface.
- Unlike Spanner (1–30 day retention), the log is permanent; a feed cursor
  never expires.

### 4.5 Read-model interop: `GRAPH_TABLE` for consumers who speak tables

Spanner's `GRAPH_TABLE(… MATCH … RETURN …)` makes a graph result a plain
table joinable with anything, and GQL's pipeline
(`MATCH → FILTER → LET → NEXT → RETURN`) is a working-table algebra much like
SPARQL's. Quipu already returns bindings tables from SPARQL; the capability
worth borrowing is the *composition*: stored queries whose results are
themselves queryable/mappable sources (the §4.1 executor reading a stored
query result as a logical source — the mirror image of Spanner's
graphs-over-views). This closes the loop: structured data → graph via RML,
graph → structured data via stored queries, each governed.

## 5. Explicitly out of scope

- **TrueTime / distributed commit wait** — single-node store; the commit
  timestamp semantics already hold locally.
- **Full-text search engine** — real gap, separate investigation; bolting an
  inverted index onto the fact log deserves its own design doc rather than a
  paragraph here.
- **ISO GQL as a query surface** — SPARQL is quipu's native surface and
  property-graph↔RDF impedance is a research topic, not a backlog item. If a
  GQL-speaking consumer materializes, revisit via the projection API.
- **Query-time OBDA (true virtual graphs)** — rejected above (§2, finding 2)
  as incompatible with ingress governance; re-materialization gets the
  benefit without the bypass.

## 6. Follow-up work

Filed as beads, smallest-first so each lands on its own:

- **quipu:** governance shapes for referencing object maps (§4.1 shapes half).
- **quipu:** change-feed read surface with capture modes and per-entity
  ordering contract (§4.4).
- **quipu:** governed-wins precedence contract on composed overlay reads
  (§4.3).
- **quipu:** mapping freshness verdict + `remap` (§4.2, registry half).
- **camayoc:** executor v2 — referencing object maps / joins (§4.1, executor
  half; filed in camayoc's tracker, since the executor is camayoc's).

## References

- [Spanner Graph overview](https://docs.cloud.google.com/spanner/docs/graph/overview)
- [Spanner Graph schema statements](https://docs.cloud.google.com/spanner/docs/reference/standard-sql/graph-schema-statements)
- [Manage schemaless (dynamic) graph data](https://docs.cloud.google.com/spanner/docs/graph/manage-schemaless-data)
- [GQL query statements](https://docs.cloud.google.com/spanner/docs/reference/standard-sql/graph-query-statements) and [GQL within SQL](https://docs.cloud.google.com/spanner/docs/reference/standard-sql/graph-sql-queries)
- [TrueTime and external consistency](https://docs.cloud.google.com/spanner/docs/true-time-external-consistency), [Timestamp bounds](https://docs.cloud.google.com/spanner/docs/timestamp-bounds), [PITR](https://docs.cloud.google.com/spanner/docs/pitr)
- [Change streams](https://docs.cloud.google.com/spanner/docs/change-streams) and [details](https://docs.cloud.google.com/spanner/docs/change-streams/details)
- [Full-text search](https://docs.cloud.google.com/spanner/docs/full-text-search), [Vector indexes](https://docs.cloud.google.com/spanner/docs/vector-indexes), [FTS with Spanner Graph](https://docs.cloud.google.com/spanner/docs/graph/full-text-search-and-graph)
- [Graph schema best practices (interleaving)](https://docs.cloud.google.com/spanner/docs/graph/best-practices-designing-schema)
- [FGAC overview](https://docs.cloud.google.com/spanner/docs/fgac-about)
- [W3C R2RML](https://www.w3.org/TR/r2rml/) and [RML 1.1.2](https://rml.io/specs/rml/v/1.1.2/)
- Camayoc: [`docs/design/rml-executor.md`](https://github.com/scbrown/camayoc/blob/main/docs/design/rml-executor.md)
