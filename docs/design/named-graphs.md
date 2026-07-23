# Design: Named Graphs (Quads) — the `graph × valid-time × tx-time` model

> **Implementation status (2026-07-23, billy):** 🟡 **Partial — foundation landed.**
> The store layer (the `g` column, the `graphs` registry, overlay
> create/write/compose, in-place migration) shipped earlier. The SPARQL read
> surface — `GRAPH <iri>` / `GRAPH ?g` scoping, `FROM` / `FROM NAMED` dataset
> selection, and the `graph` query param — is added in the #36 finish-work.
> Verified by mechanism (`src/sparql/pattern.rs`, `src/sparql/mod.rs::apply_dataset`,
> `src/mcp/mod.rs::query_result`) + 13 tests; full lib suite green.
> **Remaining:** property paths and RDFS inference are ROOT-default-only (they
> **fail loud** elsewhere); SHACL shape targeting / reasoner rule scope across
> graphs is unspecified; the write side stays on the overlay path + `/episode`
> `graph` field (no `graph` param on `/knot` yet). These keep this 🟡.

**Status:** **Partial — the subset-export / federation foundation.** Named-graph
support is what lets a consumer *partition* the store into first-class subgraphs
and query a **selected subset** rather than the whole graph. It is the substrate
under per-tenant overlays (see [group-isolation.md](group-isolation.md)), Hank's
"branches as named graphs", per-source provenance scoping, and federation.

## 1. The three axes

Every fact in `facts` carries three orthogonal coordinates on top of `(e, a, v)`:

| Axis | Column(s) | Meaning |
|---|---|---|
| **graph** | `g` | *Which* subgraph the fact lives in. |
| **valid time** | `valid_from`, `valid_to` | *When in the world* the fact holds. |
| **transaction time** | `tx` | *When the store learned* the fact. |

The graph axis is the addition (#36). It is **orthogonal** to the two time axes:
retraction, time-travel (`valid_at` / `as_of_tx`), and contradiction detection
all scope **within** a graph — a retraction in graph A never touches graph B.

## 2. The `g` column

- `g` is an interned integer. **`g = 0` is the reserved ROOT / default committed
  graph** — the source of truth.
- A **named graph's `g` is the term id of its graph IRI** (term ids are rowids,
  always `>= 1`, so they never collide with the `0` sentinel). So resolving a
  `GRAPH <iri>` clause is just `lookup(iri) -> g`.
- `g` is **not** in the fact primary key: each graph-write is its own
  transaction, so a base fact and an overlay fact for the same `(e, a, v)`
  already coexist as separate rows keyed by `tx`. `g` denormalizes `tx -> graph`
  for query-time scoping (index `idx_geav`).

## 3. Graph classes — the `graphs` registry

One row per graph, keyed by `g`, with an enforced `class`:

- **`committed`** — a durable branch. ROOT (`g = 0`) is the seeded, self-rooted
  committed graph.
- **`overlay`** — a per-tenant / in-flight view layered **over** a committed
  parent **without mutating the base**. An overlay records asserts and
  *tombstones* (an `(e,a,v)` absent in the composed view); `parent_branch` binds
  it to its committed parent **at create time** (bind-once). `compose_view`
  resolves an overlay against exactly that root (nearest-overlay-wins), so many
  tenants can extend the same base independently with no corruption.

Writes go through `transact_to_graph` (committed) or the overlay primitives
(`overlay_create` / `overlay_write`); the base is never mutated by an overlay.

## 4. Default-graph semantics (the decision that is painful to reverse)

**Committed reads are ROOT-scoped by default** — the default graph is `g = 0`
alone, **not** an all-graph union (Decision 4). Silence must not accidentally
expose every tenant's overlay.

A query's `FROM` / `FROM NAMED` clauses (or the `graph` query param) redefine the
active dataset:

- **No dataset clause:** default graph = `{ROOT}`; a `GRAPH` clause may range
  over **every** named graph (`g <> 0`).
- **`FROM <g…>`:** the default graph becomes the **RDF merge** (union) of those
  graphs. An unknown graph contributes nothing; an all-unknown / empty `FROM`
  set is an **empty** default graph (no rows) — never a ROOT fall-through.
- **`FROM NAMED <g…>`:** restricts which named graphs a `GRAPH` clause can see.
- **`FROM` with no `FROM NAMED`:** activates **no** named graphs (per SPARQL 1.1)
  — a `GRAPH` clause then matches nothing.

## 5. The SPARQL / API surface

### Query

- **`GRAPH <iri> { … }`** — scope the enclosed patterns to one named graph.
  An unknown IRI (or one excluded by `FROM NAMED`) matches nothing.
- **`GRAPH ?g { … }`** — range over the active named graphs, **binding `?g`** to
  each match's graph IRI. The same `?g` across a BGP is enforced by the join.
- **`FROM` / `FROM NAMED`** — select the active dataset, as in §4.
- **`graph` query param** (`POST /query` / `quipu_query`) — a convenience that
  scopes the query's *default* graph to one named graph without writing a
  `FROM`/`GRAPH` clause. Backward compatible when omitted; an unknown IRI gives
  an empty default; a `FROM` clause in the query text overrides it.

### Write

Named-graph writes use the **overlay path** (`overlay_create` +
`overlay_write` / MCP `quipu_overlay_*`) or `POST /episode`'s `graph` field.
There is deliberately **no** `graph` param on `/knot` yet — arbitrary writes to a
named committed branch would bypass the committed/overlay class invariant; the
overlay path is the sanctioned route.

## 6. Scope boundaries / follow-ups (honest)

- **Property paths** (`?s :p+ ?o`) and **RDFS subclass inference** are supported
  **only on the ROOT default graph**. Inside a named `GRAPH`, or over a
  `FROM`-redefined default graph, they would read the wrong graph, so they
  **fail loud** rather than silently returning ROOT results. (`?s a ?C` inside a
  named graph matches *literally* — an export wants a graph's own triples, not
  cross-graph inference.)
- **SHACL shape targeting** and **reasoner rule scope** across graphs are not yet
  specified — shapes/rules currently operate on the committed default graph.
- The event log (#… P1) emits for **ROOT-graph commits only**; overlay writes are
  transient compose-only staging and do not emit.

## 7. Related

- [group-isolation.md](group-isolation.md) — multi-tenant partitioning; named
  graphs are the storage substrate it would build on if the deferral flips.
- quipu #36 (this feature) and #37 (provenance work-item co-occurrence).
- `src/store/overlays.rs` (overlay primitives), `src/sparql/pattern.rs` +
  `src/sparql/mod.rs` (the query surface), `src/schema.rs` (`facts.g`, `graphs`).
