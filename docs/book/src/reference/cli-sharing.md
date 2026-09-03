# CLI: sharing, import and packs

Reference for the commands behind [Sharing & Federation](../sharing/README.md).
Every flag here is checked against `quipu --help` by `tests/cli_doc_drift.rs`, so
this page cannot quietly fall behind the binary.

A note on vocabulary, because it is about to matter more. **The share is the
artifact.** `quipu share` writes one as a directory of text files; `quipu pack`
writes one whose payload is carried in Quipu's own SQLite form. The SQLite form
is an *encoding* — an internal detail of how bytes are stored — not a second kind
of thing to learn. Where this page says "pack", read "a share carried in the
SQLite form".

---

## `quipu share` — produce a share

```text
quipu share --output <dir> [--graph IRI|--group-id ID|--construct QUERY]
            [--shapes NAME]... [--no-shapes] [--parent-share ID] [--turtle]
```

Writes a deterministic, git-native share into `<dir>`: `export.nt` (the facts),
`shapes.ttl` (the constraints they were validated against) and `manifest.json`
(hashes, producer, lineage).

| Flag | Effect |
|---|---|
| `--output <dir>` | where to write. Required. |
| `--graph <IRI>` | share one named graph |
| `--group-id <ID>` | share by group |
| `--construct <QUERY>` | share exactly what a CONSTRUCT query yields |
| `--shapes <NAME>` | include a named shape set; repeatable |
| `--no-shapes` | omit `shapes.ttl` — the receiver then has no constraints to validate against, so prefer not to |
| `--parent-share <ID>` | record lineage: the share this one descends from |
| `--turtle` | additionally write a Turtle view for humans |

`--parent-share` is what makes `quipu merge` possible later. A share without a
parent cannot be three-way merged — `merge` refuses with *"incoming share has no
parent_share; three-way merge has no base"* — so record it at production time,
when you know it, rather than trying to reconstruct it at reconnect time.

## `quipu import` — receive a share, into quarantine

```text
quipu import <share-dir> [--source <uri>] [--actor <id>] [--db <path>]
```

Verifies the manifest and payload hashes, then stages the result in a quarantine
graph. **It does not touch ROOT.** A hash mismatch is refused outright:

```text
share graph hash mismatch: manifest=… actual=…
```

| Flag | Effect |
|---|---|
| `--source <uri>` | record where the share came from; defaults to the directory path |
| `--actor <id>` | attribute the import |
| `--db <path>` | store file |

## `quipu import promote` — admit a staged share into ROOT

```text
quipu import promote <share-id> [--actor <id>] [--db <path>]
```

The second, separate verb. Nothing reaches ROOT because a file arrived; it
reaches ROOT because someone ran this. Keeping admission in its own command is
the point rather than an inconvenience — see the [primitive](../sharing/README.md).

## `quipu status` — has this share diverged?

```text
quipu status <share-dir> [--db <path>]
```

Reports divergence between the local store and the share's parent, as JSON. Read
it before `merge` to see what a reconnect would have to decide.

## `quipu merge` — three-way reconnect

```text
quipu merge <share-dir> [--actor <id>] [--db <path>]
```

Locates the common base through `parent_share`, merges shape-aware (SHACL
cardinalities decide what is a conflict), and **on conflict keeps the base value
and records a decision** rather than guessing.

| Exit | Meaning |
|---|---|
| `0` | merged |
| `2` | **conflicts** — nothing was guessed; the decision records name what needs a human |
| `1` | error |

Exit `2` is a distinct code precisely so a script can tell "needs a decision"
from "went wrong".

## `quipu pack` / `quipu unpack` — a share in the SQLite form

```text
quipu pack <graph-iri> --out <file.qpack.db> [--name N] [--version V] [--space N]
           [--shapes S]... [--queries Q]... [--with-vectors] [--format turtle]
quipu pack --verify <file.qpack.db>
quipu unpack <file.qpack.db> [--into <graph-iri>] [--db <path>]
```

| Flag | Effect |
|---|---|
| `--out <file>` | destination |
| `--verify <file>` | check an existing pack instead of writing one |
| `--name` / `--version` | identify the pack in its manifest |
| `--space <N>` | term space to write into |
| `--shapes <S>` / `--queries <Q>` | carry shape sets and named queries alongside the facts; repeatable |
| `--with-vectors` | include embeddings |
| `--format turtle` | carry the payload as Turtle |
| `--into <graph-iri>` | unpack into a named graph |

`--verify` answers "is this intact?" *before* you load it. Verify a pack you did
not produce yourself, every time — that is the whole reason the flag is separate
from `unpack`.

## `quipu knot` — assert facts, including identity across stores

```text
quipu knot <file.ttl> [--graph <iri>] [--shapes <shapes.ttl>]
           [--timestamp <ISO-8601>] [--db <path>]
```

Asserts Turtle into the store, validated against shapes. In the sharing context
this is how `owl:sameAs` between two stores' IRIs gets written — identity is a
fact in the graph, visible and retractable, not a string-matching heuristic.

`quipu load` is an alias for `knot`.

## Archives

```text
quipu graph freeze <iri> [--out <dir>] [--actor <who>] [--db <path>]
quipu graph thaw <iri> [--actor <who>] [--db <path>]
quipu graph list [--kind <token>] [--frozen] [--db <path>]
```

Deep freeze produces read-only, full-history graphs. See
[Graph Kinds & Deep Freeze](../concepts/graph-kinds.md).

---

## Keeping this page honest

`tests/cli_doc_drift.rs` reconciles **three** surfaces: the dispatch arms in
`src/main.rs`, the `--help` text, and this page. Checking any two is not enough —
when that test was written, `--help` documented `share`, `status`, `merge` and
`unpack` but **not `import`**, so a page-versus-help check would have passed while
the verb that receives a share stayed undiscoverable.
