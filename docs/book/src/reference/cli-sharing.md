# CLI: sharing, import and legacy packs

Reference for the commands behind [Sharing & Federation](../sharing/README.md).
Every flag here is checked against `quipu --help` by `tests/cli_doc_drift.rs`, so
this page cannot quietly fall behind the binary.

A note on vocabulary: **the share is the portable artifact.** `quipu share`
writes its standard text files directly; releases may carry the same files in a
deterministic `.qpack.tar.gz` archive. The older `pack` and `unpack` commands
remain for local SQLite compatibility, but a `.qpack.db` is not the published
interchange format.

---

## `quipu share` — produce a share

```text
quipu share --output <dir> [--graph IRI|--group-id ID|--construct QUERY]
            [--shapes NAME]... [--no-shapes] [--parent-share ID]
            [--since <parent-share>] [--turtle]
```

Writes a deterministic, git-native share into `<dir>`: RDFC-1.0 canonical
`export.nt` (the facts), `shapes.ttl` (the constraints they were validated
against), and JSON plus PROV-O/DCAT/SPDX Turtle manifests.

| Flag | Effect |
|---|---|
| `--output <dir>` | where to write. Required. |
| `--graph <IRI>` | share one named graph |
| `--group-id <ID>` | share by group |
| `--construct <QUERY>` | share exactly what a CONSTRUCT query yields |
| `--shapes <NAME>` | include a named shape set; repeatable |
| `--no-shapes` | omit `shapes.ttl` — the receiver then has no constraints to validate against, so prefer not to |
| `--parent-share <ID>` | record lineage: the share this one descends from |
| `--since <reference>` | emit a parent-bound SPARQL Update delta instead of a full share; the parent may be a directory, archive or URL |
| `--turtle` | additionally write a Turtle view for humans |

`--parent-share` is what makes `quipu merge` possible later. A share without a
parent cannot be three-way merged — `merge` refuses with *"incoming share has no
parent_share; three-way merge has no base"* — so record it at production time,
when you know it, rather than trying to reconstruct it at reconnect time.

## `quipu import` — receive a share, into quarantine

```text
quipu import <share-dir|archive|URL> [--source <uri>] [--actor <id>] [--db <path>]
quipu import delta <parent-share> <delta-share> [--actor <id>]
```

Verifies the manifest and payload hashes, then stages a local directory in its
selected store. Archives and URLs are fetched under fixed size limits and
materialized in a fresh in-memory store, so no downloaded artifact or database
is left behind. **Import never touches ROOT without promotion.** A hash mismatch
is refused outright:

```text
share graph hash mismatch: manifest=… actual=…
```

| Flag | Effect |
|---|---|
| `--source <uri>` | record where the share came from; defaults to the directory path |
| `--actor <id>` | attribute the import |
| `--db <path>` | store file |

`import delta` verifies the full parent and the delta's lineage, hashes and
restricted `DELETE DATA` / `INSERT DATA` operations, materializes the declared
result, then sends that result through the same verified in-memory import path.

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

## `quipu pack` / `quipu unpack` — legacy SQLite compatibility

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

`--verify` answers whether a legacy SQLite pack is intact before loading it.
New repository and release workflows use `share` and `import`; they do not
publish `.qpack.db` files.

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

## Producer attestation and the three trust tiers

A share can carry a signed statement of **who produced it**. Every import reports the
tier it reached, and the three are genuinely different claims — not degrees of the
same one.

| tier | what it means |
|---|---|
| `transport` | No envelope. The payload hashes verify, so the bytes are intact, but nothing says who produced them. |
| `claimed` | A signature verifies against the key **the share itself supplied**. The bundle is unaltered since signing and its identity fields are bound together — but nobody here vouched for that key. Integrity without provenance. Replay is not defended at this tier. |
| `attested` | The signature verifies against a session binding **registered out of band** on the importing store. |

### Minting a share with an attestation

```text
quipu share --output <dir> ... --attest \
  --attest-agent <agent> --attest-session <session> --attest-introducer <who> \
  --attest-issued-at <epoch> --attest-nonce <32 hex chars> \
  [--attest-key <path>] [--attest-ttl <secs>]
```

`--attest-issued-at` is required rather than defaulted to the wall clock: two runs
over one pinned dataset must produce the same signed bytes, or the share is not
re-derivable. `--attest-nonce` must be 32 lowercase hex characters and is checked at
mint time — a share minted with any other nonce is refused by every importer.

The key comes from `--attest-key`, else `$QUIPU_SIGNING_KEY`, else
`.quipu/verifier.pk8`, created 0600 on first use. That is v1 host-file custody, the
same the governance plane uses; it is not an HSM.

### Registering a producer, out of band

```text
quipu attest register --agent <a> --session <s> --public-key <hex> \
  --introducer <who> --issued-at <epoch> --expires-at <epoch> [--db <path>]
quipu attest list [--db <path>]
```

**Importing a share never registers its producer.** This is the point, not an
omission: a key that vouches for the bundle it arrived in vouches for nothing, and an
attacker substituting the whole bundle would substitute the key with it. Registration
is a separate act by the consumer, using a key obtained some other way — the same rule
the governance plane states as *quipu never self-registers*.

So a first import from an unknown producer reports `claimed`, and reports it honestly.
Reaching `attested` requires someone to decide that this key is that producer.

**Automated callers should require `attested`.** Accepting `claimed` is reasonable, but
it should be a deliberate choice by a caller who says so, not the effect of a tier that
merely does not read as failure.

---

## Keeping this page honest

`tests/cli_doc_drift.rs` reconciles **three** surfaces: the dispatch arms in
`src/main.rs`, the `--help` text, and this page. Checking any two is not enough —
when that test was written, `--help` documented `share`, `status`, `merge` and
`unpack` but **not `import`**, so a page-versus-help check would have passed while
the verb that receives a share stayed undiscoverable.
