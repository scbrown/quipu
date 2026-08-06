# Design: Named Graphs (Quads) — the `graph × valid-time × tx-time` model

> **Implementation status (2026-07-23, billy):** 🟡 **Partial — foundation landed.**
> The store layer (the `g` column, the `graphs` registry, overlay
> create/write/compose, in-place migration) shipped earlier. The SPARQL read
> surface — `GRAPH <iri>` / `GRAPH ?g` scoping, `FROM` / `FROM NAMED` dataset
> selection, and the `graph` query param — is added in the #36 finish-work.
> Verified by mechanism (`src/sparql/pattern.rs`, `src/sparql/mod.rs::apply_dataset`,
> `src/mcp/mod.rs::query_result`) + 13 tests; full lib suite green.
> **Remaining:** property paths and RDFS inference are ROOT-default-only (they
> **fail loud** elsewhere); the write side stays on the overlay path +
> `/episode` `graph` field (no `graph` param on `/knot` yet). These keep this 🟡.
> **§6 is designed and not yet built.** §7's cross-graph defect (§7.1) is
> **fixed** — quipu #56 ROOT-scoped the whole committed read path; the reasoner
> and SHACL scoping decisions in §7.2–§7.3 remain to build.

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

## 6. Graph-scoping the traversal reads (designed, not yet built)

Property paths and RDFS inference currently **fail loud** outside the ROOT
default graph. That was the right stopgap — silently reading `g = 0` from
inside a `GRAPH <iri>` block returns another graph's data — but it leaves
`GRAPH <iri> { ?s :dependsOn+ ?o }` unanswerable, which is precisely the query
Hank's per-branch worlds need.

### 6.1 What is actually hardcoded

Three call sites pin `g = 0` in SQL rather than taking it from the evaluation
context:

| Site | Line | Read |
|---|---|---|
| `sparql/property_path.rs` | `126`, `278`, `331` | path steps, `e`/`v` scans |
| `sparql/rdfs.rs` | `46` | `rdfs:subClassOf` closure |
| `sparql/rdfs.rs` | `103` | type-pattern expansion |

`TemporalContext.graph` already carries the answer at every one of these call
sites — `GraphScope` is threaded through `eval_pattern_seeded` and reaches both
modules. The work is to *use* it, not to plumb it.

### 6.2 Decision — paths and inference are single-graph, never cross-graph

A path or a subclass closure evaluates **entirely within one graph**: the same
`GraphScope` that bounds the enclosing BGP bounds every intermediate step.

- `GraphScope::Default(gids)` — reuse `sql_graph_in(gids)` (already written for
  BGP scoping in `sparql/triple.rs`) in place of the `g = 0` literal. A `FROM`
  union then traverses the merged graph, which is the RDF-merge semantics §4
  already commits to.
- `GraphScope::Named(gid)` — `g = ?gid` at every step.
- `GraphScope::AnyNamed` — **stays refused.** A path under `GRAPH ?g` would have
  to bind `?g` consistently across steps of unknown length; the natural reading
  ("the whole path lies in one graph") requires either a per-graph re-evaluation
  loop or a `?g` join column threaded through the transitive closure. Neither is
  worth building before a caller asks. The refusal keeps its current shape:
  an explicit error, never a silent ROOT read.

The rule to state once and hold to: **a path never crosses a graph boundary.**
Half a path in a tenant overlay and half in ROOT is not a fact either graph
asserts, and permitting it would make an overlay able to forge reachability in
its parent.

### 6.3 Subclass inference is deliberately *not* symmetric with paths

An ontology (`rdfs:subClassOf`) is normally asserted once, in ROOT, while
instance data lives in a branch or overlay. Scoping the closure to the *current*
graph means `?s a ex:Person` inside a named graph stops matching an `ex:Engineer`
whose superclass edge lives in ROOT — which reads as a regression, not a fix.

So the two halves scope differently, and the asymmetry is the design:

- **Instance matching** (`?s a ?C`) is scoped to the current graph.
- **The subclass closure** (`collect_class_and_subclasses`) reads the
  **schema graph**, which defaults to ROOT and does not follow the data scope.

This preserves today's behaviour for the ROOT case exactly, and makes the named
graph case do the useful thing (branch instances, shared ontology). It is also
reversible: if a branch ever needs to *override* the ontology, the schema graph
becomes the union `{current, ROOT}` with the nearest definition winning — the
same nearest-overlay-wins rule §3 already uses for facts.

An explicit `GRAPH <iri> { ?s a ?C }` where the caller wants literal, un-inferred
matching keeps that today via §5's export semantics; the escape hatch is
`FROM <iri>` with no schema graph, which we spell out when the flag exists.

### 6.4 Acceptance

- [ ] `GRAPH <iri> { ?s :p+ ?o }` traverses only `<iri>`; a path that would need
      a ROOT edge to complete yields no row
- [ ] `FROM <a> <b>` traverses the merge of `a` and `b`
- [ ] `GRAPH ?g { ?s :p+ ?o }` still errors, with a message naming §6.2
- [ ] `?s a ex:Person` inside a named graph matches instances in that graph via
      a subclass edge asserted in ROOT
- [ ] An overlay cannot make a path appear in its committed parent

## 7. SHACL and reasoner scope (designed, not yet built)

### 7.1 The bug this uncovered — ✅ fixed (quipu #56)

`Store::current_facts()` filters `op = 1 AND valid_to IS NULL` and **nothing
else** — it has no `g` predicate, so it returns facts from *every* graph. Since
overlays write into the same `facts` table, every caller of it is already
cross-graph today:

| Caller | Consequence |
|---|---|
| `reasoner/evaluate.rs:316` | rules derive from overlay facts and write conclusions to ROOT |
| `graph.rs:55,754,1114` | PageRank / centrality counts every tenant's overlay |
| `rdf.rs:268` | full export leaks overlay facts into a ROOT dump |
| `owl_parse.rs:128,145` | ontology parse sees overlay class axioms |
| `reconcile/mod.rs:233` | entity resolution matches across tenants |

This is the same class of defect as #53 — silent, and each layer looks healthy.
It was live the moment a second graph existed.

Fixing it turned up a **write**-path case worse than the read leak:
`retract_triples` selects via `entity_facts(entity)` and then commits the
retraction datums with `transact(...)`, which writes to ROOT. Un-scoped, a
`/retract` on an entity that also had overlay facts generated retractions for
*another graph's* facts and wrote them into ROOT — exactly the "a retraction in
graph A does not touch graph B" invariant #36 claims to hold.

**Resolution (#56):** the whole shared committed read path is ROOT-scoped —
`current_facts`, `entity_facts`, `facts_as_of`, `entity_history`,
`attribute_history`, `detect_contradictions`, and the `has_surviving_*` orphan
guards. `schema::ROOT_GRAPH` replaces the inline `0`. Callers wanting a specific
graph use `current_facts_in_graph(g)`; no caller wanted the old cross-graph
behaviour, so no `*_all_graphs` variant was added.

`overlays.rs` is unaffected — it carries its own graph-aware SQL and never went
through these functions. That is asserted by a regression test, and each of the
seven scoping tests fails without the fix.

### 7.2 SHACL shape targeting

Validation takes a serialized Turtle payload, not a store handle
(`Validator::validate(&self, data: &[u8])`), so shape *targeting* is already
graph-agnostic — whatever the caller serializes is what gets validated. The
decision is therefore about the **write gate**, not the validator:

- `validate_on_write` validates **the delta being written, against the shapes
  loaded in ROOT**, regardless of which graph is being written.
- Shapes are ontology, and by §6.3 ontology lives in ROOT. An overlay cannot
  weaken its parent's constraints by asserting laxer shapes into itself.
- A future per-branch shape set would compose ROOT ∪ branch with the branch
  only able to *add* constraints, never remove — but there is no caller yet, so
  it is not built.

### 7.3 Reasoner rule scope

- A ruleset **ranges over exactly one graph and writes conclusions back into
  that same graph.** Cross-graph derivation is refused for the §6.2 reason: a
  conclusion derived from overlay premises does not belong to ROOT.
- `evaluate(store, ruleset, timestamp)` gains a graph parameter defaulting to
  ROOT, so existing callers are unchanged.
- Derived facts stay tagged `reasoner:<rule-id>` as today; the graph is the
  additional coordinate, so retracting a branch drops its derivations with it.

### 7.4 Acceptance

- [x] `current_facts()` is ROOT-scoped; a store with an overlay returns the same
      facts it returned before the overlay existed (#56)
- [x] A ROOT retraction does not touch an overlay's facts (#56)
- [x] The half-ghost guard does not count overlay facts as survivors (#56)
- [x] Time travel and contradiction detection are ROOT-scoped (#56)
- [x] Overlay compose still sees the overlay (regression guard, #56)
- [ ] A ruleset run against a branch writes its conclusions into that branch
- [ ] An overlay asserting a laxer shape does not relax validation of its parent

## 8. Scope boundaries / follow-ups (honest)

- **Write-side `graph` param on `/knot`** stays deferred: arbitrary writes to a
  named committed branch would bypass the committed/overlay class invariant. The
  overlay path and `/episode`'s `graph` field are the sanctioned routes.
- The event log (P1) emits for **ROOT-graph commits only**; overlay writes are
  transient compose-only staging and do not emit.
- Property paths under `GRAPH ?g` remain refused (§6.2).

## 9. Related

- [group-isolation.md](group-isolation.md) — multi-tenant partitioning; named
  graphs are the storage substrate it would build on if the deferral flips.
- [graph-labels.md](graph-labels.md) — freshness/trust/policy labels on the
  graphs this doc partitions, composing by meet/join over the active dataset;
  named datasets (saved `FROM` sets) as the overlapping-set complement to the
  `parent_branch` tree.
- [multi-db-composition.md](multi-db-composition.md) — attached read-only
  databases contributing named graphs via disjoint term spaces; reuses §4's
  ROOT-alone default as "attaching adds graphs, never widens a query."
- [shape-versioning.md](shape-versioning.md) — the time axis for the shapes
  §7.2 keeps in ROOT.
- quipu #36 (this feature) and #37 (provenance work-item co-occurrence).
- `src/store/overlays.rs` (overlay primitives), `src/sparql/pattern.rs` +
  `src/sparql/mod.rs` (the query surface), `src/schema.rs` (`facts.g`, `graphs`).
