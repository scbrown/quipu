# Knowledge Packs

> **Implementation status (2026-08-12):** ✅ **Built.** `src/pack.rs` —
> `Manifest`, `pack`, `pack_turtle`, `unpack`, `verify`, `content_hash` — with
> the `quipu pack` / `quipu pack --verify` / `quipu unpack` CLI, and the
> stored-query registry packs draw from (`src/store/queries.rs`).
> `--space <term-space>` on pack export is built too (2026-08-25). Still open:
> the design's retrieval-policy block (`quipu:defaultDataset` /
> `quipu:recommendsFloor`) — a pack today carries the graph's labels but not
> the fuller policy vocabulary. See `docs/design/knowledge-packs.md`.

A knowledge pack is a **distributable graph artifact**: one named graph's
current facts, plus the shapes, stored queries, and labels that make it usable,
in a single file you can version, hand to another agent or environment, verify,
and import. The artifact format *is* the database format — a pack is an
ordinary Quipu SQLite store with a one-row `pack_manifest` table describing
itself.

## What goes in a pack

- **Facts** — the current facts of the source graph, written through the
  ordinary transaction path so term ids are correct by construction (a raw row
  copy would carry the producer's private id assignment; a pack never does).
- **Shapes** — named explicitly with `--shapes`, since shapes are global and
  carry no graph linkage. The selection is recorded in the manifest.
- **Stored queries** — named with `--queries`, so a domain layer ships the
  competency questions that make it usable on arrival, not just its triples.
- **Labels** — the graph's freshness/trust/policy label travels with the pack,
  so a consumer can compose it without a side channel.
- **Vectors** (optional, `--with-vectors`) — embeddings re-keyed by IRI.
  Restricted to the built-in SQLite vector backend; a delegated or LanceDB
  backend cannot be enumerated, so the flag is refused rather than silently
  producing a pack with no vectors.
- **The manifest** — pack format, name, producer-declared semver, term space,
  content hash, creation time, source graph, producer identity, and row counts.

## Creating a pack

```bash
quipu pack urn:example:graph --out domain.qpack.db --name "domain" --version 1.0.0
quipu pack urn:example:graph --out domain.qpack.db --shapes s --queries q --with-vectors
```

The output is a single clean file — the build goes through `VACUUM INTO`, so
no `-wal`/`-shm` siblings ride along beside the file you actually copy.

`--format turtle` emits an **interop bundle** instead: a directory of
`graph.ttl`, `shapes.ttl`, `queries.json`, and `manifest.json`, for consumers
that are not Quipu. It is export-only — nothing unpacks it — but it carries
the *same* content hash as the `.qpack.db` form, because the hash is computed
from canonical content, not from the emitted bytes.

## Unpacking

```bash
quipu unpack domain.qpack.db --into urn:local:domain --db my.db
```

`unpack` materializes the pack's facts into a local graph (defaulting to the
pack's own graph IRI) and installs its shapes and stored queries **through the
versioned write paths** — never an overwrite of registries the consumer
already has. The report states what arrived: facts, shapes, queries.

## Verification

```bash
quipu pack --verify domain.qpack.db
```

Verification recomputes the pack's **content hash** and compares it to the
manifest. The hash is sha256 over the lexically sorted, deduplicated
N-Triples of the graph plus the packed shapes, queries, and labels — sorted
because the store's own emission order depends on term-id assignment, and the
hash must describe the *content*, not the producer. Two stores holding the
same triples hash the same.

This makes the hash the pack's citable version reference: pin it in an
environment, promote the same hash-verified file dev → staging → prod, roll
back by re-attaching the prior pack. Never re-pack between environments —
that creates a different artifact and defeats the hash as the promotion
identity. There are no signatures; verification is integrity, not provenance.

## Use cases

- **Shipping a distilled derived layer** — pack a curated or derived graph
  with its shapes and competency queries, and hand consumers a verifiable
  artifact instead of access to the producing store.
- **Small, portable subsets** — cut a wasm-sized slice of a larger graph for
  embedded or edge deployment, where a single self-describing file matters.
- **Environment promotion** — one artifact, one hash, attached identically in
  each environment.

## Built vs designed

Built: `quipu pack`, `quipu unpack`, `quipu pack --verify`, the Turtle interop
bundle, vector export on the SQLite backend, the stored-query registry, and
`--space <term-space>` on export — the pack is built in space 0 and shipped
through the same respace machinery as `quipu db respace`, so a consumer can
attach it as-is without an id collision (`.qpack.db` packs only; a Turtle
bundle carries IRIs, not term ids, so `--space` does not apply there).
Designed but not yet built: the retrieval-policy block (default-dataset and
recommended-floor facts a consumer could SPARQL), and delta/diff packs —
v1 packs are whole-layer, read-only artifacts.

## Related

- [CLI reference](../reference/cli.md) — the `quipu pack` / `quipu unpack`
  flags in full.
- `docs/design/knowledge-packs.md` — the full design, including the
  retrieval-policy vocabulary and the promotion workflow.
