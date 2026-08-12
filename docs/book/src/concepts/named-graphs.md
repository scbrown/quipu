# Named Graphs, Overlays & Datasets

> **Implementation status:** ✅ **Built** on the sanctioned surfaces — the `g`
> column and `graphs` registry, overlay create/write/compose, `GRAPH` /
> `FROM` / `FROM NAMED` evaluation, the `graph` query param, and named
> datasets. Deliberately **not** built: a `graph` param on `POST /knot`
> (writes go through overlays or `/episode`'s `graph` field), and property
> paths under `GRAPH ?g` (explicitly refused, never a silent ROOT read).
> See `docs/design/named-graphs.md` for the full design.

Every fact in quipu's EAVT store carries a graph coordinate on top of
`(entity, attribute, value)` and the two time axes — the store is really a
quad store. The `g` column says *which* subgraph a fact lives in, orthogonal
to *when it holds* (`valid_from`/`valid_to`) and *when the store learned it*
(`tx`). Retraction, time-travel, and contradiction detection all scope
within a graph: a retraction in graph A never touches graph B.

- `g = 0` is the reserved **ROOT** graph — the default committed graph, the
  source of truth.
- A named graph's `g` is the interned term id of its graph IRI, so resolving
  `GRAPH <iri>` is a single term lookup.
- The `graphs` registry keeps one row per graph with an enforced `class`:
  `committed` (a durable branch; ROOT is the seeded, self-rooted one) or
  `overlay` (a layer over a committed parent). A graph's class is fixed at
  create.

## Querying: `GRAPH`, `FROM`, and the `graph` param

**Committed reads are ROOT-scoped by default.** The default graph is ROOT
alone, not an all-graph union — silence must never expose another tenant's
overlay. A query widens its dataset only by saying so:

- `GRAPH <iri> { … }` scopes the enclosed patterns to one named graph. An
  unknown IRI matches nothing.
- `GRAPH ?g { … }` ranges over the active named graphs, binding `?g` to each
  match's graph IRI. Property paths under `GRAPH ?g` are refused with an
  explicit error rather than silently reading ROOT.
- `FROM <g…>` makes the default graph the RDF merge (union) of those graphs.
  An unknown graph contributes nothing; an all-unknown `FROM` set yields an
  **empty** default graph — never a fall-through to ROOT.
- `FROM NAMED <g…>` restricts which named graphs a `GRAPH` clause can see. A
  query with `FROM` but no `FROM NAMED` activates no named graphs (per
  SPARQL 1.1), so `GRAPH` matches nothing.

```sparql
SELECT ?s ?title
FROM <http://example.org/graphs/derived>
WHERE { ?s <http://example.org/title> ?title }
```

`POST /query` and the `quipu_query` MCP tool also take a `graph` request
param — a convenience that scopes the query's *default* graph to one named
graph without writing a `FROM` or `GRAPH` clause:

```json
{"query": "SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o }",
 "graph": "http://example.org/graphs/derived"}
```

Omitting it keeps the ROOT default; an unknown IRI gives an empty default
graph; a `FROM` clause in the query text overrides the param. The param also
resolves **dataset names**: passing a dataset IRI scopes the query to that
dataset's members, so `FROM <dataset>` and `"graph": "<dataset>"` mean the
same thing. The same `graph` param on `quipu_export` / `POST /export`
exports one named graph's facts instead of ROOT.

Property paths follow a fixed graph scope without crossing it: a
`GRAPH <iri>` closure stays inside that graph, a `FROM <a> FROM <b>` path
traverses their merge. A path never crosses a graph boundary — half a path
in an overlay and half in ROOT is not a fact either graph asserts.

## Overlays

An overlay is a scratch layer over a committed parent branch: hypotheses go
in the overlay, the committed base is never mutated. Two write primitives,
one uniform read:

- **Create** (`quipu_overlay_create` / `POST /overlay/create`) registers an
  overlay-class graph **bound once** to its committed parent branch (ROOT by
  default). Idempotent; rebinding to a different parent is an error — the
  binding is unforgeable.
- **Write** (`quipu_overlay_write` / `POST /overlay/write`) takes one of
  three ops: `assert` and `retract` are graph-scoped writes into the
  overlay; `tombstone` marks a specific `(e, a, v)` from the parent as
  *absent* in the overlay's composed view, without touching the parent.
- **Compose** (`quipu_overlay_compose` / `POST /overlay/compose`) resolves
  the stack `[overlay > parent-branch-root]` with a single rule: a triple is
  present iff **asserted and not tombstoned**, nearest-overlay-wins. Overlay
  asserts shadow the parent; overlay tombstones hide parent triples;
  everything else falls through.

Many tenants can extend the same base independently this way, and a
committed read never sees any of them unless the query names the overlay.
This is the sanctioned write path for named graphs: there is deliberately no
`graph` param on `POST /knot`, because an arbitrary write to a named
committed branch would bypass the committed/overlay class invariant.
`POST /episode` does accept a `graph` field for ingestion into a named
graph. The event log emits for ROOT-graph commits only; overlay writes are
compose-only staging and do not emit.

## Datasets

A dataset is a **name for an arbitrary set of graphs**, queryable as one
unit — the reusable form of a `FROM a b c` clause, so a graph set can be
labelled, governed, and handed to another agent. Managed via the
`quipu_datasets` MCP tool or `POST /datasets` (create / list / show /
remove).

```bash
curl -s localhost:3030/datasets -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "create", "name": "http://example.org/datasets/hot",
       "members": ["http://example.org/graphs/a", "http://example.org/graphs/b"]}'
```

- `FROM <dataset-iri>` (and the `graph` query param) expands to the
  dataset's members at resolve time; everything downstream reads the
  expanded set.
- A dataset is never implicitly active — the ROOT-alone default is
  untouched; you get a dataset's graphs only by naming it. A member naming
  an unregistered graph contributes nothing.
- Members may carry a declared ordering (`{"graph": …, "ord": N}`);
  duplicate ranks are refused rather than tiebroken silently. An empty
  dataset is refused.
- Datasets are mirrored into the meta-graph as `quipu:Dataset` /
  `quipu:includesGraph` facts, so they are queryable and governed like any
  other fact. Datasets are orthogonal to the overlay branch tree: the branch
  tree is compose's resolution root, datasets are overlapping named sets.
- The labels of a query's active dataset compose across its member graphs —
  see [Graph Labels](./graph-labels.md) for how freshness/trust/policy
  labels fold over the graphs a query actually reads.

## Related

- [REST API](../reference/rest-api.md) — `/query`, `/export`, `/overlay/*`,
  `/datasets`
- [MCP Tools](../reference/mcp-tools.md) — `quipu_query`, `quipu_export`,
  `quipu_overlay_*`, `quipu_datasets`
- [Graph Labels](./graph-labels.md) — labels on graphs and their composition
  over datasets
- Design doc: `docs/design/named-graphs.md`
