# Multi-DB Composition

> **Implementation status (2026-08-12):** 🟩 **Composed reads are built.**
> Term spaces and the space-aware allocator (`src/store/mod.rs`), `quipu db
> respace` (`src/store/respace.rs`), ATTACH mounting, graph registration and
> the composed facts source (`src/store/attach.rs`), and term aliases
> (`src/store/alias.rs` — quipu #76, with one deviation: the alias table is
> TEMP and rebuilt at open, not persisted). `GRAPH ?g` ranges attached graphs
> end-to-end. **Not built:** the full fail-loud cross-DB limit surface (quipu
> #77) and the blob sidecar (design §7, consumer-gated). Mounting is a
> library API today — `Store::open_with_attachments` — no config or CLI
> surface yet.
> See `docs/design/multi-db-composition.md`.

A quipu store is one SQLite file. Composition mounts several of those files —
a shared read-only reference layer beside a per-tenant memory store, a
knowledge pack beside both — as one queryable store, without merging them.
Each layer keeps its own lifecycle: it ships, versions and swaps
independently, it is distributed as a single file, and read-only mounting is
enforced by SQLite at the file level, not by query rewriting.

## ATTACH, and two kinds of alias

Composition is SQLite `ATTACH`: the store's one connection mounts each extra
file under a **schema alias** (validated `^[a-z][a-z0-9_]*$`, read-only via
`file:…?mode=ro`), and a single query planner sees every file — so composed
queries get real joins and index pushdown, which result-merging
[federation](federation.md) cannot offer.

```rust
let store = Store::open_with_attachments(
    "tenant.db",
    &[Attachment::read_only("shared", "reference.db")],
)?;
```

The design's §1.2 alias is a different thing: a **term alias**. The same IRI
interned independently in two files gets two ids — an alias, not a collision.
At open, a TEMP `term_alias` table is built by joining the files' `terms`
tables on IRI; lookups return every id an IRI denotes (`lookup_all`), query
predicates match all of them, and result bindings are canonicalised toward the
local id after SQL `DISTINCT`, so an entity present in both files is one row,
not two.

## Term spaces

Every term id in a store — `facts.e`/`a`/`g`, the graph registry, even ids
embedded inside `Value::Ref` blobs — is an integer assigned per file. Unioned
naively, two files' ids collide silently. Term spaces make ids globally
unique by construction: each database owns a space `s` and allocates ids
from `s · 2^40 + k` (`SPACE_SIZE = 2^40` — about 10¹² terms per space,
millions of spaces). The allocator in `src/store/mod.rs` reads the store's
space from the `term_spaces` registry and allocates within that half-open
range.

A legacy store's ids are `1..n` — exactly space 0 — so existing stores need
no rewrite, and new stores still allocate from space 0 unless configured. The
constraint that falls out: **at most one space-0 database per composition**;
`verify_attached_schema` refuses a colliding attach with a message naming the
fix.

## Respace

That fix is `quipu db respace`: rewrite a database into a chosen term space so
it can be attached beside another. The remap is paid once, offline. The source
is opened read-only and copied with `VACUUM INTO`; every rewrite happens in
the new file, and the original stays byte-identical. Respace derives its work
from the live schema — every column is classified as term-id-bearing or not,
and an unclassified column makes it refuse before writing anything, because a
missed column produces a store that opens, answers, and is wrong.

## What composes, and what does not

An attachment contributes **named graphs**, nothing else. Each attached graph
registers in the local `graphs` table (with a `source` column naming the
attachment), so `GRAPH <iri>` resolution and graph labels are uniform over
local and attached graphs. The attachment's own default graph and label
meta-graph are per-database and are not contributed. Two guarantees follow:

- **Attaching changes no existing query's result.** The default dataset stays
  the local default graph alone; a layer is visible only to a query that
  names one of its graphs. With no attachments, the generated SQL is
  byte-identical to an unattached store's.
- **Quipu never writes to an attached database.** Read-only mounting is
  enforced by SQLite; writing a local fact *into* an attached graph is
  refused at the Rust layer.

Permanently out, per the design's §6: cross-DB writes and cross-DB
transactions (SQLite's multi-file atomic commit does not work in WAL mode,
and quipu is WAL). Transaction ids are file-local with no cross-file
ordering, so `as_of_tx` over a composed store is refused with an error rather
than answered wrongly — **valid-time travel works** (`valid_from`/`valid_to`
are portable ISO strings), transaction-time does not cross files. The event
log stays local. The remaining fail-loud refusals are tracked as quipu #77.

## Operating it

```bash
quipu db respace --into 7 --out shared-s7.db --db shared.db
```

`--out` is required — respace writes a fresh file and never overwrites — and
the report prints the space moved from and to, and the rows touched. A store
whose space fills up gets the same advice: the allocator's exhaustion error
names respace. Knowledge packs (`quipu pack`) are attachable artifacts too: a
pack declares its term space in its manifest, and attaching verifies the two
agree.

## See also

- [CLI reference](../reference/cli.md) — `quipu db respace`, `quipu pack`,
  and `quipu graph import` (the copy-based fallback to attaching)
- [Named graphs](../concepts/named-graphs.md) — the substrate; an attachment
  is a source of named graphs
- [Federation](federation.md) — the remote half; composition sits below the
  provider seam, federation above it
- `docs/design/multi-db-composition.md` — the full design, including the
  consumer-gated blob sidecar (§7)
