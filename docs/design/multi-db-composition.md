# Design: Multi-DB Composition — term spaces, ATTACH, and the blob sidecar

> **Implementation status (2026-08-06):** 🟨 **Step 2 of §9 is built; ATTACH is
> not.** There is still no `ATTACH DATABASE` anywhere in the tree; `Store` holds
> one `rusqlite::Connection` in WAL mode. What exists:
>
> - **§1.1 term spaces — BUILT.** `term_spaces` registry, space-aware
>   allocation (`s · 2^40 + k`, `k` derived from the table), legacy stores
>   space 0 by definition. `src/store/mod.rs`, `src/store/term_space_tests.rs`.
> - **`quipu db respace` — BUILT.** `src/store/respace.rs`. Reads the source
>   read-only and writes a new file; the original is left byte-identical.
> - **§1.2 aliases / §2 ATTACH / §5 cross-DB limits — NOT built** (quipu #75,
>   #76, #77).
>
> The blob sidecar (§7) is **design-accepted / consumer-gated** — do not build
> it until a payload consumer exists.
>
> ⚠️ **One thing this document did not say, found while building respace.** §1
> lists the term-id-bearing state as `facts.e`, `facts.a`, `facts.g` and the
> `Ref` blob. That is incomplete: `graphs.g`, `graphs.parent_branch` and
> **`vectors.entity_id`** are also term ids. The first two were caught by #74's
> acceptance amendment; `vectors.entity_id` was named in no issue, no comment
> and no document, and was found only by enumerating the live schema. Respace
> therefore derives its work from the schema and refuses on any column it
> cannot classify — see the module docs. **Do not re-state a list of
> term-id-bearing columns here.** Every copy of that list so far has been
> wrong, including this one.

**Status:** The driving deployment already exists as a convention Quipu knows
nothing about: NeuralAmplifier's tenancy design gives each recurring principal
its own Quipu database for durable memory, "with a shared read-only datalinks
db mounted alongside" — chosen because a database per brain is a file, not a
cluster, and `group_id` is provenance, not isolation. That "mounted alongside"
is `ATTACH`, and Quipu cannot do it. Composing multiple SQLite files buys four
things at once: independent lifecycle per layer (a static reference layer
ships, versions and swaps independently of the hot instance layer),
distribution (a layer is one file you hand to another agent or tenant),
isolation (read-only attach enforced at the file level, not by query
rewriting), and freshness scoping (small hot DB for churn, large cold DB for
stable knowledge).

## 1. The crux: term identity across files

`terms(id INTEGER PRIMARY KEY, iri UNIQUE)` means `id` is a rowid assigned
sequentially per database. Two databases assign the same id to different IRIs
and different ids to the same IRI. And the ids are everywhere: `facts.e`,
`facts.a`, `facts.g` are term ids — and **object-position `v` embeds a term id
inside an opaque tagged BLOB** (`src/types.rs`, `Value::Ref(id)` →
`[TAG_REF, id.to_le_bytes()]`) that SQL cannot rewrite.

A naive `UNION ALL` across attached `facts` tables is therefore silent, total
corruption. Three options were considered:

- **(A) Query-time remap** — build `term_map(foreign → local)` at attach and
  join through it. **Rejected:** the `Ref` BLOB cannot be rewritten in SQL, so
  every attached fact would have to be materialized in Rust, destroying the
  "attach a large cold DB and pay nothing" motivation.
- **(B) Import-with-remap** — `quipu graph import <db> --as <iri>`: read the
  foreign DB, remap ids, rewrite `Ref` BLOBs once, write into a local named
  graph. Correct, simple, uses only existing write paths. **Kept as the
  fallback and migration tool** — but it duplicates the shared layer into
  every tenant DB and needs re-import on every upstream change, killing
  independent lifecycle. Not the answer.
- **(C) Disjoint term-space partitioning — the design.** Term ids become
  globally unique by construction.

### 1.1 Term spaces

Each database owns a **space** `s` and allocates term ids from
`s · 2^40 + k`: ~1.1 × 10¹² terms per space, ~8.4 × 10⁶ spaces — both
dimensions have orders of magnitude of headroom in an i64 rowid.

- Ids from an attached DB are already meaningful locally. **No remap, ever.**
- **Legacy stores need no rewrite.** An existing store's ids are `1..n` —
  exactly the range space 0 owns. A legacy DB *is* a space-0 DB; migration is
  one row in a new `term_spaces` table.
- Constraint that falls out: **at most one space-0 DB per composition.** A DB
  intended for sharing is created with an explicit space, or re-spaced offline
  by `quipu db respace` — option (B)'s remap, paid once, offline, preserving
  the file as its own artifact.
- **New DBs still allocate from space 0 unless configured.** Silently changing
  id allocation for everyone would break test fixtures and any golden file
  that depends on `1, 2, 3…`.

### 1.2 Aliases, not collisions

The same IRI interned independently in two spaces gets two ids. That is an
alias, not a collision. At attach time, build:

```sql
CREATE TABLE term_alias (canonical_id INTEGER NOT NULL, alias_id INTEGER NOT NULL);
-- populated by: SELECT l.id, r.id FROM main.terms l JOIN shared.terms r USING (iri)
```

Bounded by the IRIs present in *both* files — for the intended deployment
(shared reference DB + per-tenant memory) that is the shared vocabulary:
hundreds to thousands of rows, not millions.

Read path: `lookup(iri)` grows a `lookup_all(iri) -> Vec<i64>` sibling, and
the resolution sites in `src/sparql/triple.rs` emit `a IN (…)` instead of
`a = ?`. Object-position `Ref` values resolve to a set of encodings the same
way.

**The sharp edge — where the subtle bug lives:** SQL `DISTINCT` runs over raw
ids, so an aliased entity present in both files yields two rows that are
semantically one. The fix is canonicalization in Rust *after* the SQL
DISTINCT, deduping bindings — cost proportional to the result set, not the
store. This gets its own issue and its own test file, with the adversarial
fixture: two stores interning the *same* IRI at *different* rowids and
*different* IRIs at the *same* rowid.

## 2. ATTACH mechanics

`Store::open_with_attachments(path, &[Attachment { alias, path, read_only }])`;
`ATTACH` runs immediately after `INIT_SQL`, before migrations.

- **Attached DBs are never migrated.** They may be read-only and they are
  another owner's artifact. `verify_attached_schema` refuses an attach whose
  `facts` lacks `g`, or whose term space collides with one already attached,
  with a message naming the fix.
- **Read-only** uses the URI form `file:/path/shared.db?mode=ro`
  (`PRAGMA query_only` is connection-wide and cannot scope to one attachment).
- SQLite allows a bound parameter for the attach *filename* but the schema
  alias is a name, not an expression — it is interpolated and validated
  `^[a-z][a-z0-9_]*$`.
- **Quipu never writes to an attached database.** Nearly free structurally —
  every write funnels through `transact_to_graph`, whose SQL uses unqualified
  (`main`) table names — and enforced at the Rust layer anyway.

## 3. Attached graphs are just named graphs

The insight that makes composition cheap: an attached layer contributes one or
more **`g` values that are already valid in the local id space** (term spaces,
§1.1). So an attachment is a *source of named graphs*, not a fourth axis —
every existing `FROM` / `GRAPH` / dataset / label mechanism works on it
unchanged.

At attach, each attached graph registers in `main.graphs` with a new
`source TEXT` column naming the attachment alias (not a foreign key — FKs do
not span attached databases). One registry, uniform `GRAPH <iri>` resolution,
uniform labels ([graph-labels.md](graph-labels.md) — note that labelling an
attached graph is the one point where the two designs meet, since `graphs.g`
is a term id).

**Union is explicit-`FROM` only; the default stays main-ROOT-alone.** The
direct analogue of [named-graphs.md](named-graphs.md) §4's decision, for the
identical reason: silence must not widen the dataset. **Attaching a DB adds
named graphs; it never changes what an existing query returns.** That is the
compatibility guarantee.

## 4. The SQL: `facts_source()`

Not a SQL VIEW named `facts` — that would shadow the real table and break
every write. A helper:

```rust
fn facts_source(&self) -> Cow<'_, str>
// no attachments  -> "facts"                      (today's exact SQL)
// with attachments-> "(SELECT … FROM main.facts
//                      UNION ALL SELECT … FROM shared.facts) AS facts"
```

Two non-negotiable tests:

1. **The no-attachment path produces byte-identical SQL to today.** A
   regression here taxes every query in the product.
2. **The graph predicate is pushed inside each UNION branch**, verified by
   `EXPLAIN QUERY PLAN`. Pushed outside, SQLite scans both files on every
   triple pattern. `idx_geav` exists per-file (each DB's own migration created
   it), so the pushed-down predicate is indexed on both sides.

## 5. The `GraphProvider` seam — composition is not federation

`GraphProvider::query` takes a SPARQL string and returns a whole
`QueryResult`, so `FederatedProvider::query_all` can only *concatenate result
sets* and tag `_provider`. It cannot join across providers, cannot push a
filter down, cannot make `GRAPH ?g` bind a remote graph. A local ATTACH can do
all three, because one SQLite query planner sees everything.

So: **ATTACH lives below `GraphProvider`, in the store.** `GraphProvider`
stays for *remote* federation
([federation-remote-provider.md](federation-remote-provider.md)), where
result-level merge is the only option available. The seam they *share* is
labels: `ProviderStatus` gains the label fields, and a remote must carry a
**declared** trust label rather than an inferred one — the SARC trust
boundary, surfaced at the federation edge.

## 6. Not possible — permanently, and this document says so

- **Cross-DB writes: out.** Not deferred — out. Every write is `main`-only.
- **Cross-DB transactions: out.** SQLite's atomic multi-file commit uses a
  super-journal that **does not work in WAL mode**, and Quipu is WAL. Since
  attachments are never written this never arises — but it is the reason
  "write to attached" cannot become a feature without leaving WAL.
- **Transaction-time travel across attachments: refused, loudly.** `tx` ids
  are file-local; `transactions` is per-file; there is no ordering between two
  files' tx sequences. `as_of_tx` over a dataset spanning attachments fails
  with an error naming this section — the [named-graphs.md](named-graphs.md)
  §6.2 refusal style, never a silent wrong answer. **Valid-time works** —
  `valid_from`/`valid_to` are ISO strings and portable. This asymmetry —
  valid-time portable, transaction-time local — is the most important honest
  limit in this design. It is fixable later by a tx-space partition mirroring
  the term space; deferred, not impossible.
- **The event log stays main-only** (it is already ROOT-only).

## 7. The blob sidecar — references in the graph, bytes beside it

> **Consumer-gated.** Designed here so the vocabulary and the anti-pattern
> rule exist; **not scheduled** until a payload consumer (a blog, a document
> store) actually arrives — per the same keeper-gate discipline as
> [group-isolation.md](group-isolation.md).

For payload-shaped content (posts, rendered pages, images, documents) the
graph is the index, never the warehouse — the separation this stack already
practices (`shapes`/`ontologies` are side tables, not facts; NeuralAmplifier
keeps its briefing, decision log and saves outside the graph).

- A **blob sidecar** is its own SQLite file with one table:
  `blobs(hash TEXT PRIMARY KEY, bytes BLOB, media_type TEXT, created_at
  TEXT)`, keyed by content hash (sha256). Under this design it is *just
  another attachment* — same read-only attach, same distribution story. No new
  infrastructure class.
- The graph holds the **reference plus everything graph-shaped**:
  `?post quipu:contentRef <urn:blob:sha256:…>` alongside author,
  published-at, tags, relations — and the lattice labels. The blob is
  immutable by construction, so **freshness and trust attach to the
  reference, not the bytes**: "is this post current?" is a graph question;
  "what are its bytes?" is a PK lookup. A content hash is also exactly the
  stable, hashable version reference the verdict-evidence reasoning permits
  (hashing *graph state* was rejected in
  [policy-edit-hooks.md](policy-edit-hooks.md) because it has no stable
  serialisation; a content hash has nothing else).
- **Forbidden, as a rule and not a suggestion: payloads in `facts.v`.** The
  BLOB column would accept them, and they would bloat `idx_eavt`, the term
  dictionary, and every scan.
- **No general-purpose KV surface.** SQLite already is one, and an open-ended
  KV write path on a governed store is the ungoverned-namespace failure this
  repo keeps fencing. Hot mutable consumer state (drafts, caches, session
  data) belongs in the consumer's own file.
- Two things keep this off the critical path: blob reads bypass the SPARQL
  path entirely (they want a `GET /blob/<hash>` route — the one place this
  workstream would touch `http_auth.rs`'s completeness test), and GC is a real
  design problem (a blob unreferenced by any current fact may still be
  referenced by a *retracted*, time-travelable fact — v1 is append-only, and a
  `quipu blob gc` that respects bitemporal reachability is deferred).

## 8. Migration & compatibility

Additive throughout, in the `migrate_named_graphs` style:

1. `graphs` gains `source TEXT` (guarded `ALTER TABLE ADD COLUMN`).
2. New tables (`term_spaces`, `term_alias`, and §7's sidecar schema when
   gated in) are `CREATE TABLE IF NOT EXISTS` in `INIT_SQL` — safe; but any
   index on a *new column of an existing table* goes in the migration
   function, not `INIT_SQL` (the recorded aegis-akb8 hazard: `INIT_SQL` runs
   against pre-migration stores where the column does not exist yet).
3. `term_spaces` seeded `(0, <this db>)` — legacy DBs are space 0 by
   definition, no data rewrite.
4. `[[quipu.attachments]]` config lands **in the same change as its
   consumer** — the `unwired_warnings` build guard requires it, and this repo
   has already deleted one inert key as a false affordance.

Defaults that keep every deployment byte-identical: no attachments →
`facts_source()` returns `"facts"`, zero cost; no configured space → space 0;
attaching adds graphs without changing any existing query's results.

## 9. Build order

1. This document.
2. Term-space partitioning + `quipu db respace` — **hard prerequisite**;
   nothing else here can start.
3. ATTACH plumbing + `facts_source` + `graphs.source` registration — with the
   byte-identical-SQL test and the `EXPLAIN QUERY PLAN` pushdown test. If
   feature-gated, wired into the CI matrix in the same change.
4. Alias resolution across spaces (`lookup_all`, `IN (…)`, post-DISTINCT
   canonicalization) — own issue, own test file.
5. Fail-loud cross-DB limits (`as_of_tx`, event log) — must not lag step 3;
   the first cross-DB `as_of_tx` that silently lies is unrecoverable trust
   damage.
6. (Gated) blob sidecar, when a consumer exists.

## 10. Related

- [named-graphs.md](named-graphs.md) — the substrate; §4's ROOT-alone default
  is reused here as attach-adds-nothing-implicitly.
- [graph-labels.md](graph-labels.md) — labels on attached graphs; the designs
  meet at `graphs.g` being a term id.
- [federation-remote-provider.md](federation-remote-provider.md) — the remote
  half; composition (this doc) is below the provider seam, federation above.
- [group-isolation.md](group-isolation.md) — per-tenant DBs are the isolation
  boundary that `group_id` is not; this doc is what makes the shared layer
  mountable beside them.
- [knowledge-packs.md](knowledge-packs.md) — the distribution format: a pack
  is an attachable layer file with a manifest, content hash, and retrieval
  policy.
