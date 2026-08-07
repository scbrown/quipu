# Design: Knowledge Packs — a graph, its shapes, its queries, and its retrieval policy as one artifact

> **Implementation status (2026-08-06):** ⬜ **Designed, not built.** Depends on
> unbuilt substrate: term spaces and attach
> ([multi-db-composition.md](multi-db-composition.md), quipu #74/#75), graph
> labels ([graph-labels.md](graph-labels.md), #65), named datasets (#69), and
> the versioned shape registry ([shape-versioning.md](shape-versioning.md),
> #71). The stored-query registry (§2) has no dependencies and can start
> immediately.

**Status:** The layering designs make knowledge *attachable*; nothing yet makes
it *distributable*. A knowledge layer should be a single artifact you can pack,
version, hand to another agent or environment, verify, and activate — carrying
not just its triples but its SHACL shapes, its lattice labels, and its
**retrieval policy**: the named competency queries that make it usable, the
label floors its producer recommends, and the dataset it expects to be
activated with. (Prior art: TrustGraph's "knowledge cores" get the lifecycle
right — a file that is offline → loaded → activated; the design below reaches
the same lifecycle with one format instead of three, because in Quipu the
artifact format can *be* the database format.)

## 1. The artifact is a database

`quipu pack` produces a fresh, self-contained Quipu SQLite store:

```text
quipu pack <graph-iri> --out <file.qpack.db>
    [--name <n>] [--version <semver>] [--space <term-space>]
    [--shapes <name>...] [--queries <name>...]
    [--with-vectors] [--format turtle]
```

A pack contains: the graph's current facts, the pack's own `terms`, the
selected shapes and stored queries, the graph's meta-graph label facts, the
retrieval-policy block, optional vectors, and a one-row `pack_manifest` table.
**Unpacking is attaching** — a pack is an attachable layer file exactly as
[multi-db-composition.md](multi-db-composition.md) defines one, so the entire
`FROM`/`GRAPH`/dataset/label machinery applies to it with no import step.

### 1.1 Why the term-id problem does not apply

A raw copy of a store is unshareable — `facts.e/a/g` and the `Value::Ref`
tagged BLOB inside `facts.v` embed per-database term rowids
(multi-db-composition.md §1). `pack` never copies rows. It **writes the
destination through the normal `transact_to_graph` path**, which re-interns
every IRI into the pack's own term space — so ids are correct *by
construction*, `Ref` BLOBs included. The manifest records the space; a
shareable pack allocates from a non-zero space (#74), and `--space` chooses it.

### 1.2 The manifest

One row in `pack_manifest`:

| Field | Contents |
|---|---|
| `pack_format` | Integer format version, starting at 1 |
| `name`, `version` | Pack identity; `version` is producer-declared semver |
| `term_space` | The space this pack's ids are allocated from |
| `content_hash` | §1.3 |
| `created_at`, `source_graph` | Provenance basics |
| `producer` | JSON mirroring `GET /version`: quipu version, `git_sha`, compiled `features` (features matter — `shacl`/`owl` change what a store contains), plus `embedding_model`/`dim` when vectors are present |
| `counts` | Facts / shapes / queries / vectors, for cheap sanity checks |
| policy block | Default dataset members + recommended floors (§3) |

The policy block and identity are **also written as meta-graph facts inside
the pack**, so a consumer can SPARQL a pack's self-description rather than
needing a side-channel reader.

### 1.3 The content hash

sha256 over the **lexically sorted N-Triples serialization** of
(graph ∪ shapes ∪ queries ∪ labels). Sorting the serialized lines is what
makes the hash deterministic: the store's own export ordering is by term id
(`ORDER BY e, a` — insertion-order dependent, and not even a total order),
so hashing emission order would tie the hash to id assignment. Sorted
serialized triples are id-free and total.

This hash is the pack's citable version reference. It is precisely the kind
of stable, declared reference the verdict-evidence rules permit
([policy-edit-hooks.md](policy-edit-hooks.md) rejected hashing *graph state*
because it has no stable serialisation — this constructs one, for a bounded
snapshot). `quipu pack --verify <file>` recomputes and compares.

### 1.4 Mechanics worth pinning now

- The destination is a second in-process `Store` (`Store::open` on a fresh
  path — nothing blocks this; the single-connection design is per-Store, not
  per-process).
- Ship via `VACUUM INTO`: the working store is WAL and would otherwise leave
  `-wal`/`-shm` siblings beside the "single file."
- `--with-vectors` re-keys embeddings **by IRI join** (the `vectors` table is
  keyed by local term id, which does not travel), and is **restricted to the
  built-in SQLite vector backend in v1** — the Lance/delegate backends have
  no enumerate/scan surface. The manifest records model + dim; attach warns
  on mismatch with the consumer's configured embedder.
- Shapes are global (`shapes` has no graph linkage), so pack takes explicit
  `--shapes <name>…`; the selection is recorded in the manifest.
- `--format turtle` emits the interop bundle instead — `graph.ttl` (via
  `export_rdf_subset`), `shapes.ttl`, `queries.json`, `manifest.json` —
  export-only in v1.

## 2. The stored-query registry

Today the named-query catalog (`src/mcp/named_query.rs`) is compiled-in Rust:
seven schema-agnostic entries, a flat global array, no scoping field of any
kind. Consumers cannot ship competency questions with their domain — which is
exactly what a domain layer needs to be *usable* on arrival.

**Tables**, versioned in the [shape-versioning.md](shape-versioning.md)
close-don't-overwrite style from day one:

```sql
CREATE TABLE queries (
  name TEXT NOT NULL, description TEXT NOT NULL, template TEXT NOT NULL,
  dataset TEXT,              -- optional scope: NULL = global
  valid_from TEXT NOT NULL, valid_to TEXT, tx INTEGER
);
CREATE TABLE query_params (
  query_name TEXT NOT NULL, ordinal INTEGER NOT NULL,
  name TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('iri','text','int')),
  required INTEGER NOT NULL, dflt TEXT, description TEXT NOT NULL,
  PRIMARY KEY (query_name, ordinal)
);
```

This mirrors the compiled-in `NamedQuery`/`ParamSpec` shape exactly (name,
description, template, *ordered* params with kind/required/default), so one
renderer serves both.

- **Load-time validation, not call-time surprise.** On `load`: the template
  must parse under spargebra with params substituted by placeholder values;
  a `{param}` reference with no spec is an error; an optional param with no
  default is an error. (The last one fixes a latent hole in the compiled-in
  path, where such a placeholder is left verbatim in the SPARQL.)
- **Dispatch:** `quipu_ask` consults the compiled-in catalog first
  (unchanged), then the registry. The listing merges both, each entry flagged
  `"source": "builtin" | "stored"`. A dataset-scoped stored query activates
  its dataset (#69 `FROM` expansion) unless the caller overrides.
- **Surface:** `POST /queries` + MCP `quipu_queries` with
  `load|list|get|remove` actions — the `tool_shapes` pattern, including the
  hard error on unknown action. The route is classified **WRITE** in
  `http_auth` (loading mutates the store; "when unsure, it is a write").
  `quipu_ask` stays on the read-only pool: registry *reads* are fine there,
  and the pooled-tool survival test pins it. CLI: `quipu queries
  load|list|get|remove`.

## 3. Retrieval policy — recommend, never enforce

A pack carries three policy artifacts:

1. **Its stored queries** (§2) — the competency questions that make the layer
   usable by name rather than by hand-written SPARQL.
2. **A default dataset declaration** — `quipu:defaultDataset` meta-graph
   facts naming the graphs this pack expects to be activated with (its own
   graph, typically plus a terminology layer).
3. **Recommended label floors** — `quipu:recommendsFloor` facts: the minimum
   freshness/trust the producer considers safe for consumers of this layer.

The rule that keeps this safe: **a pack recommends; the consumer's config
enforces.** Floors in a pack are surfaced (printed at attach, queryable in
the meta-graph) but never applied — enforcement remains the consumer's
`[quipu.labels]` opt-in (#68). A pack that could tighten enforcement could
DoS its consumer; one that could loosen it could bypass the consumer's own
floor. Neither is acceptable, so neither is possible.

Together with the manifest this makes a pack a **capability manifest** in the
sense the stack already uses: it declares what the layer contains, what
questions it answers, what trust it claims, and what it needs — and every
claim is queryable and hash-anchored.

## 4. Unpack, verification, and promotion

- **Attach path (the normal one):** `verify_attached_schema` (#75) also reads
  `pack_manifest` when present — term-space collision check, optional
  `--verify-hash` recompute, and the manifest's labels/policy surfaced to the
  consumer. Beyond that, attaching a pack is just attaching a layer.
- **Import path:** `quipu unpack <file> [--into <graph-iri>]` materializes
  the pack into a local graph (the #74 import-with-remap machinery) for when
  attaching is overkill, and installs the pack's shapes and queries into the
  local registries **through the versioned write paths** — never an
  `INSERT OR REPLACE` clobber of what the consumer already has.
- **Promotion is a workflow, not a feature:** pin = record a pack's
  `content_hash` in the consumer's meta-graph; promote dev → staging → prod =
  attach the same hash-verified file in each environment; roll back =
  re-attach the prior pack. Every step is existing machinery once labels
  land; the doc's contribution is naming the workflow so environments do it
  the same way.

Operator sequence: explicitly verify the pack hash before mounting; record
that exact `content_hash` as the environment pin; mount the same respaced
artifact in the next environment; roll back by restoring the prior pinned
artifact. Never re-pack between environments: that creates a different
artifact and defeats the hash as the promotion identity.

## 5. Scope boundaries (honest)

- **Packs are read-only artifacts.** No in-place mutation; a change is a new
  version with a new hash.
- **No delta/diff packs in v1.** Whole-layer artifacts only.
- **`--with-vectors` requires the built-in SQLite vector backend** in v1;
  Lance/delegate backends cannot enumerate. The manifest records model+dim;
  a mismatch with the consumer's embedder warns, it does not convert.
- **The Turtle bundle is export-only in v1** — no tar import path.
- **tx-time does not travel.** A pack inherits the multi-db §6 asymmetry:
  valid-time is portable, transaction time is local to the file. `as_of_tx`
  across a pack boundary refuses, loudly.
- **A pack's floors never change consumer enforcement** (§3). Stated twice
  because it is the load-bearing safety property of the design.

## 6. Build order

1. This document.
2. **Stored query registry** (§2) — independent of everything; can start now.
3. **Retrieval-policy vocabulary** (§3's meta-graph predicates + SHACL) —
   depends on #65 (meta-graph) and #69 (datasets); small.
4. **`quipu pack`** (§1) — depends on #74 (term spaces), plus 2 and 3 for
   the queries/policy payload. Includes the sorted-N-Triples hash helper and
   the stale `export_rdf` docstring fix as a drive-by.
5. **`quipu unpack` + pack-aware attach verification** (§4) — depends on 4
   and #75.

## 7. Related

- [multi-db-composition.md](multi-db-composition.md) — the substrate: term
  spaces, attach, the read-only layer contract a pack file satisfies.
- [graph-labels.md](graph-labels.md) — the labels a pack carries and the
  floors it recommends.
- [shape-versioning.md](shape-versioning.md) — the close-don't-overwrite
  registry pattern the queries table reuses, and the versioned path unpack
  installs through.
- [named-graphs.md](named-graphs.md) — the graph model underneath all of it.
- `src/mcp/named_query.rs` — the compiled-in catalog the registry extends.
