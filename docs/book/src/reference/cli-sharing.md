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

Prerequisite: [load the identifier-policy catalogue](../sharing/README.md#prepare-an-outward-share)
and the shapes governing your data. The default destination is outward.
An empty block-tier catalogue exits 2 (cannot verify); a matching identifier
exits 1; a checked, clean share exits 0. `--no-shapes` does not bypass this check.

```text
quipu share --output <dir> [--graph IRI|--group-id ID|--construct QUERY]
            [--shapes NAME]... [--no-shapes] [--parent-share ID]
            [--since <parent-reference>] [--turtle] [--destination internal]
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
| `--since <reference>` | emit a parent-bound SPARQL Update delta instead of a full share; the parent may be a directory, archive or URL, not a `share_id` |
| `--turtle` | additionally write a Turtle view for humans |
| `--destination internal` | skip the outward scrub and stamp the manifest `destination: internal`. LAN-internal destinations only — see below |

### The outward scrub, and `--destination internal`

Every share is checked against the store's own `aegis:InternalIdentifierPattern`
catalogue — the rules tiered `block` — and refused if the payload matches one.
Nothing is rewritten: an internal hostname or an RFC1918 address is *entity
identity*, and silently editing it would produce a share that says something the
store never said.

That is the right default for a share bound for a public remote, and the wrong
one for a share bound for an internal forge, where those identifiers are the
point. `--destination internal` is the single explicit way to say so:

```text
quipu share --output qpack/today --destination internal
```

It does three things, and the third is what makes the first two safe:

1. **Skips the outward scrub entirely.** Not a per-pattern exception, not an
   allowlist — the check does not run.
2. **Stamps `destination: "internal"`** into `manifest.json`, and
   `quipu:destination "internal"` into `manifest.ttl`. The exemption travels
   with the bytes instead of living in the shell history of whoever produced
   them.
3. **Binds that stamp into `share_id`.** Unlike `attestation`, the field is
   *not* stripped before the manifest is hashed. Deleting it to launder the
   payload onward leaves a manifest that no longer hashes to the id it carries,
   and every consumer's verification refuses it.

There is no environment variable and no config setting. A share that says
nothing is scrubbed, including every share produced over HTTP: `POST /share`
cannot select a destination, because a caller who could would be turning the
guard off on a server they do not own.

**What the marker buys you downstream.** `quipu import` of a stamped share runs
the scrub the producer skipped. If the payload passes it imports normally — a
share marked internal out of caution is not quarantined for it. If the payload
fails, the import is refused unless the operator repeats the declaration with
`quipu import <dir> --destination internal`. So internal facts cannot enter a
store silently and then leave it in someone else's outward share.

**Deltas are scrubbed separately, and they have to be.** `delta.ru` is not built
from the store: its DELETE clause is lifted verbatim from the parent's
`export.nt`. An identifier retracted from the graph yesterday is still quoted in
today's delta — and the parent is usually the very internal share that was
allowed to carry it. A full outward share of the same store passes cleanly while
that delta does not, which is exactly why the delta document gets its own check
rather than riding on the result share's.

`--since` compares the current share with the referenced parent. Its default
8 MiB limit applies to the serialized delta file map (update, shapes and
manifests), not to the full result graph. A large unchanged graph can therefore
produce a small delta; a large insertion or deletion can still exceed the limit.
The producer still materializes the full result internally, so this transport
limit is not a memory bound. Use `--parent-share` to record an identity without
computing a delta.

`--parent-share` is what makes `quipu merge` possible later. A share without a
parent cannot be three-way merged — `merge` refuses with *"incoming share has no
parent_share; three-way merge has no base"* — so record it at production time,
when you know it, rather than trying to reconstruct it at reconnect time.

## `quipu import` — receive a share, into quarantine

```text
quipu import <share-dir|archive|URL> [--source <uri>] [--actor <id>]
            [--destination internal] [--db <path>]
quipu import delta <parent-share> <delta-share> [--actor <id>]
```

Verifies the manifest and payload hashes, then stages a local directory in its
selected store. Archives and URLs are fetched under fixed size limits and
materialized in a fresh in-memory store by default, so no downloaded artifact
or database is left behind. With an explicit `--db <path>`, archives and URLs
stage in that database using its loaded shapes and registered identities, just
like a directory. Carried shapes are not automatically adopted: a receiver
without a matching local vocabulary still quarantines the typed data. **Import never touches ROOT without promotion.** A hash mismatch
is refused outright:

```text
share graph hash mismatch: manifest=… actual=…
```

| Flag | Effect |
|---|---|
| `--source <uri>` | record where the share came from; defaults to the directory, archive path, or URL |
| `--actor <id>` | attribute the import |
| `--db <path>` | stage in this store, including archive and URL imports |

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

## `quipu pack --full` / `quipu restore` — whole-store packs

A `--full` pack is a different artifact from everything above: not a graph, but
the **whole store**, carried losslessly for internal backup. `share` carries
current facts; `--full` carries `facts` as whole rows — `g, tx, valid_from,
valid_to, op, retracted_tx` — so history, including what was retracted, travels
with it.

```text
quipu pack --full [--format text] --destination internal --out <path> [--db <path>]
quipu restore <file.qpack | text-pack-dir> [--force] [--db <path>]
```

| Flag | Effect |
|---|---|
| `--full` | pack the whole store losslessly, rather than one graph |
| `--format text` | render that whole-store pack as a git-friendly DIRECTORY of text instead of a SQLite file |
| `--force` | allow `restore` to replace a destination that still holds live facts |

Both forms refuse an outward destination, and that refusal is atomic — no output
is left behind. A full pack carries the event log and every operational table
that is not explicitly excluded, so publishing one is the operator's decision
rather than something this command may acquire by convenience.

`--format text` does **not** transport the declared *regenerated* set. Today that
is `vectors`: embeddings are derived data, roughly 2.2 GB of floats at homelab
scale, and `quote()` renders a BLOB as `X'<hex>'` — so inlining them would
produce a 4–5 GB "git-friendly" artifact, which is not one. The manifest instead
records what was left out, how many rows it was, and the recipe to rebuild it
(embedding model name, its SHA-256, and the dimension), and `restore` prints a
`REGENERATE:` line naming them. A restore from a text pack is therefore complete
in facts, history and provenance, and **not** complete in derived data until
those are rebuilt — which is why it says so rather than reporting plain success.

The binary `--full` pack still transports vectors: a backup that forces a
re-embed on restore is a poor backup. The two whole-store packs therefore carry
different content by design, and their content hashes are **not** comparable to
each other.

`--format text` writes `manifest.json`, `schema.sql`, and `data/<table>.sql`.
Rows are emitted one `INSERT` per line, ordered by the row text itself, so a row
moving on disk produces no diff and a committed pack changes only when its
contents do. `restore` rebuilds the store, checks referential integrity, and
**refuses unless the reconstruction hashes identically to what the manifest
claims** — nothing reaches the destination until that holds, so a dump missing a
file, a table or a single row is rejected rather than installed as a quietly
smaller store.

`restore` REPLACES; to merge a published pack into an existing store use
`unpack`. Each verb refuses the other's format by name rather than reporting an
intact artifact as corrupt.

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
  --introducer <who> --issued-at <epoch> --expires-at <epoch> [--allow-write] [--db <path>]
quipu attest allow-write|deny-write <session> [--db <path>]
quipu attest revoke <session> [--db <path>]
quipu attest list [--db <path>]
```

A binding is **share-only** unless an operator grants it write (`--allow-write` at
registration, or `allow-write` later). Only then may its key sign HTTP writes. `list`
prints each binding's `allow_write`, `expires_at` and `revoked`, so the scope a key
holds is visible without reading the store. `revoke` keeps the row: a revoked binding
refuses as `revoked`, which is a different finding from an unbound key. Rotation is a
new binding plus a revoke of the old one.

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

## `quipu share diff` — what changed between two packs

```text
quipu share diff <old> <new> [--format text|markdown|json]
```

Each side is a pack directory (the standard artifact's `payload.nq`, else a
legacy share's `export.nt`) or a single N-Triples/N-Quads file. Like the Git
transport commands below, it reads files only and opens no store.

The diff compares **facts**, not lines, and groups them by subject entity:

```text
~ Alice (people/alice)
  ~ age: "30"^^xsd:integer -> "31"^^xsd:integer
  - nickname: "Al"
+ Carol (people/carol)
  + rdfs:label: "Carol"
  + role: "designer"
2 entities: 1 changed, 2 added, 1 removed facts
```

That is the whole output for the fixture pair in `tests/fixtures/share-diff/`,
whose raw line diff is 11 lines of full IRIs and blank-node labels: Alice's
address is a blank node that RDFC relabelled (`_:b0` to `_:c14n7`) without any
change to its content, and it contributes nothing.

- An entity is shown by its `rdfs:label` with a compact name beside it; an
  unlabelled IRI is shown compactly (`prefix:local` for well-known vocabularies,
  otherwise its last two path segments). Predicates show their label or local
  name. When two distinct predicates under one entity would show the same name
  (`ex:name` and `schema:name` both labelled "name", or two IRIs ending
  `/name`), each carries its compact IRI — `name (ex/name)`,
  `name (schema:name)` — or its full IRI if even those collide. Compaction
  depends only on the IRI, never on the data, so both sides of a diff name
  things the same way.
- `~ predicate: old -> new` is reported only when the slot (subject, predicate,
  graph) holds exactly one value on **both** sides. A multi-valued slot shows
  its removed and added values separately.
- **Blank nodes are matched by structure, not label.** RDFC-1.0 can relabel
  every blank node between two versions of a payload; a pure relabel is zero
  lines. A blank node referenced from another node is shown inline
  (`[ city "Paris" ; zip "75001" ]`) as part of the referencing fact, so an edit
  inside it is a change of that fact. Structurally identical blank nodes on
  one slot are one fact with a count: cardinality matters to shapes
  (`sh:maxCount`), so adding a second copy is shown as
  `~ p: [ r "v" ] x1 -> [ r "v" ] x2`, and the textconv marks a fact asserted
  more than once with `xN`. Limits: identical values nested inside an inlined
  blank node still collapse; a blank node referenced only from inside a
  blank-node cycle has no named root and is not shown; a blank *graph name*
  is keyed by its label.
- `--format markdown` suits a PR comment; `--format json` is the same
  structure (`entities[].changed/added/removed`, plus totals) for tools.

## `quipu diff-textconv` — readable `git diff` for pack files

```text
quipu diff-textconv <file>
```

Prints one payload file as stable, labelled, entity-grouped text: one header
per entity (sorted by IRI, so a relabel never reorders the file), one
`predicate: value` line per fact, blank nodes inline. Git's `textconv` runs it
on both sides of a diff, so an ordinary `git diff`, `git log -p` or `git show`
reads like this instead of two lines of full IRIs:

```diff
@@ -1,5 +1,5 @@
 AAA Tracking (ability/aaa-tracking)
-  abbrev: "AAA"
+  abbrev: "AAA CHANGED"
   effectText: "x2 vs. air attacks"
```

Setup, once per clone (the attribute is already in this repository's
`.gitattributes`; add it to your own):

```bash
printf '*.nt diff=quipu\n*.nq diff=quipu\n' >> .gitattributes
git config diff.quipu.textconv "quipu diff-textconv"
```

Without the `git config` line the attribute is inert and Git diffs raw lines.
`git diff --no-textconv` shows the raw form on demand. A file that does not
parse (a working copy with merge conflict markers, say) is printed unchanged,
so `git diff` never fails on it. Textconv affects display only: merges, hashes
and `pendant-check` still operate on the canonical bytes.

## Git transport: driver, decisions, and CI

These commands operate on repository files and immutable Git snapshots. They do
not open a Quipu store. Use a build with the `shacl` feature (the default).

They shell out to the `git` executable found on `PATH` (the wrapper, the driver's
snapshot reads, the shapes three-way merge, and `pendant-check` all do). If `git`
cannot be found, the command exits 1 with `` `git` executable not found on PATH ``
and writes nothing.

```text
quipu git-merge <ref>
quipu merge-driver <base-file> <ours-file> <theirs-file> <path>
quipu pendant-resolve <base-ref> <ours-ref> <theirs-ref> <dir> <key> <choice>
quipu pendant-check <base-ref> <ours-ref> <theirs-ref> <result-ref>
```

Version attributes for each share directory (adjust `qpack` to your layout):

```gitattributes
qpack/export.nt merge=quipu
qpack/shapes.ttl merge=quipu
qpack/manifest.json merge=quipu
qpack/manifest.ttl merge=quipu
```

Start at the repository root with a clean index and worktree. Save the branch
tips for later checks:

```bash
ours=$(git rev-parse HEAD)
theirs=$(git rev-parse topic)
base=$(git merge-base "$ours" "$theirs")
quipu git-merge "$theirs"
```

`git-merge` supplies a command-local `merge.quipu.driver` definition and the
three immutable commit IDs to its child Git process. Nothing is installed in
global Git configuration. This context matters: Git does not promise to merge
`shapes.ttl` before `export.nt`. The driver reads and merges shapes from the
three commits first, rather than trusting whichever working file happens to
exist. Directly invoking the low-level driver without context refuses.
Overlapping shape edits refuse before the wrapper changes the worktree; resolve
those edits on the branches first. Criss-cross bases and incompatible
store/scope/destination/layout metadata also refuse.

The graph operator is the same one used by store-level `quipu merge`. Functional
conflicts retain the base value and create entries in `decisions.json`. No Git
conflict markers are written to N-Triples. New entities from opposite branches
with a common type are compared locally: whitespace-normalized, lowercase labels,
Jaro-Winkler similarity at least 0.90, top five candidates per new entity.
These are proposals, never automatic identity assertions. No paid model or
network call is used. This baseline can miss aliases and can propose false
matches; the decision belongs to the reviewer.

Exit 2 means decisions or Git conflicts remain. Inspect the sidecar, then choose:

```bash
quipu pendant-resolve "$base" "$ours" "$theirs" qpack conflict:0 ours
quipu pendant-resolve "$base" "$ours" "$theirs" qpack alias:0 reject
```

Conflict choices are `base`, `ours`, or `theirs`; alias choices are `accept`
(add an explicit `owl:sameAs`) or `reject` (keep identities distinct). Resolution
reconstructs the operator result from Git and refuses stale or altered proposal
records. The last resolution must pass SHACL. It rewrites canonical `export.nt`,
regenerates both manifests, drops old attestations and the optional derived
`export.ttl`, and records `parent_share=ours` plus both IDs in `merge_parents`.
Input `tx_anchor` and `created_at` remain the ours snapshot's metadata; a Git
merge does not fabricate a store transaction. Each file is replaced atomically;
the directory is not a multi-file transaction, so the hash gate must pass before
use. Review and stage the complete directory, then commit:

```bash
git add qpack
git commit
quipu pendant-check "$base" "$ours" "$theirs" HEAD
```

The wrapper always stops before commit, including clean merges. It reconciles
whole qpacks after Git because Git may skip individual drivers when one file is
unchanged. Blank-node graphs currently refuse divergent merges: canonical blank
labels are not stable identities across snapshots. Skolemize those nodes first.

**CI is required even when local driver use is documented.** A clone with no
driver definition, a forge merge button, or a rebase can fall back to text merge.
The repository's SHACL test job runs `scripts/ci/pendant-merge-check.py` with full
history and the PR's synthetic merge commit. It also replays two-parent merges introduced on the topic branch, then
reconstructs base/ours/theirs,
requires recorded resolutions, compares the committed graph, shapes and manifest
with the operator result, and validates SHACL. A hand-rehashed unsafe result
still fails. Push/nightly runs validate the committed packs too; a squash commit
alone cannot reconstruct the original branch pair, so the PR check remains
load-bearing. Keep the SHACL test check required in branch protection.

**Known gap: an administrator bypasses the gate.** The replay that can actually
fail is the PR check. On a `push` event the final replay is trivially
`(base, ours, theirs) = (before, before, result)`: a fast-forward of the previous
tip, so it re-validates the committed packs but cannot reconstruct a merge
that happened elsewhere. This repository's `main` protection uses strict status
checks, so a squash merge's tree equals the synthetic merge commit the PR check
tested. It does **not** enforce protection for administrators, so an admin's
direct push, or an admin merge that skips pending or failing checks, lands a
qpack that no merge replay has verified. Treat an admin bypass as unverified and
run `quipu pendant-check` on the original base/ours/theirs yourself, or enable
"include administrators" if that gap is unacceptable.

Measured on the repository's real Datalinks qpack, replacing the same functional
value on both branches produces a **text conflict**, not a clean double. Git
writes three marker lines into the RDF. Separate new IRIs with identical labels
merge cleanly without an identity decision; disjoint additions merge cleanly and
correctly. `tests/git_qpack_merge.rs` exercises those cases, decision resolution,
and the driver-absent CI refusals using actual Git repositories.
