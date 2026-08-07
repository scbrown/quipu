# Design: WebAssembly Support — running Quipu without a server

> **Implementation status (2026-08-07):** 🔬 **Investigation complete, not
> started.** Every blocker below was verified by building against
> `wasm32-unknown-unknown`, not inferred from the manifest. Every performance
> number was measured on this branch, native x86-64, `--release`,
> `--no-default-features`. The one number NOT measured is wasm-vs-native
> throughput; obtaining it is Phase 0.

**Summary:** Quipu's graph core is already wasm-clean — the whole
Oxigraph-family stack, `ring`, `petgraph`, `datafrog` and `regex` compile for
`wasm32-unknown-unknown` today. The build blockers are SQLite's C dependency and
four crates the library does not actually use. **But the binding constraint is
not the build — it is the BGP join evaluator, which is O(n²) and puts an
interactive ceiling at roughly 10³–10⁴ episodes on any target.** wasm does not
create that problem; it makes it arrive sooner and in a place the user can see.

---

## 1. Why this is close: the family, not the store

Oxigraph ships as two separable things, and Quipu deliberately uses only one.

**The family** — standalone crates that are pure RDF plumbing:

| Crate | Job | Used by Quipu |
|---|---|---|
| `oxrdf` | RDF data model (`NamedNode`, `Literal`, `Triple`) | ✅ |
| `oxttl` / `oxrdfio` | Parsers & serializers (Turtle, N-Triples, JSON-LD) | ✅ |
| `spargebra` | SPARQL **parser** → algebra tree. No execution | ✅ |
| `sparesults` | SPARQL results serialization | ✅ |
| `spareval` | The query **evaluator** | ❌ |
| `oxigraph` | The triplestore: `spargebra` + `spareval` + RocksDB | ❌ |

Quipu takes the parsers, the data model and the SPARQL front-end, then supplies
its own storage and its own evaluator. That is the right call, and it is why
this project is possible: Quipu is not storing triples, it is storing a
**bitemporal, governed, multi-tenant fact log that presents as RDF**. Named
graphs with committed/overlay classes, `valid_from`/`valid_to`, tombstones,
transaction provenance, term spaces, the event log, authority grants — none of
that fits inside "a set of triples."

**For the record, `oxigraph` itself does support wasm.** Verified: version 0.5.9
compiles clean for `wasm32-unknown-unknown` with
`default-features = false, features = ["js"]` (its default is `rocksdb`, which
does not). It appears in `Cargo.lock` only transitively, via `rudof_lib`. If the
SQLite path ever proved untenable, an in-memory Oxigraph backend is a real
fallback — but it would cost the entire temporal and governance model, so it is
a fallback, not a plan.

## 2. How the data is organized

EAVT fact rows in SQLite (`src/schema.rs`):

```sql
terms(id, iri)                                          -- IRI ↔ integer dictionary
facts(e, a, v, g, tx, valid_from, valid_to, op)         -- the fact log
transactions(id, timestamp, actor, source)              -- provenance
graphs(g, class, parent_branch, created_at)             -- named-graph registry
```

- `e`, `a` are term ids — dictionary-encoded subject and predicate.
- `v` is an opaque **tagged BLOB**, not a term id (`src/types.rs:35`):
  `Ref(i64)` for IRI/blank objects, plus `Str`, `Int`, `Float`, `Bool`, `Bytes`,
  and `Lang { lexical, lang }`. The tag byte gives round-trip fidelity with no
  schema lookup.
- `g` is the named graph; `0` is ROOT. Term ids are rowids so they are always
  ≥ 1 and never collide with the sentinel.
- Four index permutations (`idx_eavt`, `idx_aevt`, `idx_vaet`, `idx_tx`) plus
  `idx_geav` from the named-graph migration.

SPARQL evaluation: `spargebra` parses to algebra → `src/sparql/pattern.rs` walks
the algebra → each leaf triple pattern is compiled by `src/sparql/triple.rs` into
a parameterized SQL `WHERE` clause over `facts`, and SQLite's planner picks the
index.

**There is no in-memory index alongside SQLite.** SQLite's B-trees *are* the
index; its page cache *is* the working set. `open_in_memory()` is just the
`:memory:` VFS — identical schema, identical SQL, pages in RAM. That is the
crux of the port: **we do not have to move a storage engine to wasm, we have to
give SQLite somewhere to put pages.**

## 3. Use cases

1. **Local-first agent memory.** Today Quipu needs a server. In wasm it is a
   library an agent embeds — browser extension, Electron/Tauri app, VS Code web
   extension. Ontology-enforced memory that never leaves the device. For
   privacy-sensitive domains that is a product, not a feature.
2. **Browser-based agent harnesses.** [WebContainers][wc] run a full Node.js
   runtime in-browser via wasm; [bolt.new][bolt] is an agent driving one —
   filesystem, package manager, terminal, dev server, all in-tab. VS Code for
   Web and JupyterLite are the same shape. These have **no backend to run
   `quipu-server` against**; a wasm build is the whole integration.
3. **Zero-install evaluation.** Drop a knowledge pack on a web page and query
   it. `docs/design/knowledge-packs.md` already defines a pack as a
   self-contained store file, so this is nearly free once wasm lands — and it is
   the best demo Quipu could have.
4. **Sandboxing.** Wasm isolation is [being adopted specifically for
   agent-generated code][sandbox]. One Quipu instance per tenant per sandbox is
   defense in depth on top of named graphs, not a replacement for them.
5. **Edge / WASI.** `wasm32-wasip1` runs on Cloudflare Workers, Fastly,
   wasmtime — Quipu as an edge-deployed knowledge cache.

Cases 2 and 3 want `wasm32-unknown-unknown`; case 5 wants WASI. **Anchor on the
browser** — it is where the harnesses are, it is the harder target so WASI
largely falls out, and OPFS gives us the export story for free.

## 4. Verified build blockers

Reproduce with:

```bash
rustup target add wasm32-unknown-unknown
RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
  cargo check --target wasm32-unknown-unknown --no-default-features --lib
```

### 4.1 What already works

A probe crate carrying `parking_lot`, `oxrdf`, `oxttl`, `oxrdfio`, `sparesults`,
`spargebra`, `ring`, `regex`, `serde_json`, `petgraph`, `datafrog`, `toml`,
`strsim`, `hex` and `thiserror` compiles clean for `wasm32-unknown-unknown`.
**The entire RDF, reasoning and signing core is already portable** — Ed25519
signing included.

### 4.2 The one hard blocker: bundled SQLite

```text
sqlite3/sqlite3.c:14884:10: fatal error: 'stdio.h' file not found
```

`libsqlite3-sys` with `bundled` compiles SQLite's C through `cc`, and
`wasm32-unknown-unknown` has no libc. Everything Quipu is — `Store`, SPARQL,
episodes, governance, the label lattice — sits on that connection. Two routes:

| Route | Target | Notes |
|---|---|---|
| **`sqlite-wasm-rs`** via `[patch]` | `wasm32-unknown-unknown` | libsqlite3-sys-compatible shim, memory + OPFS VFS. Browser-native. **Recommended.** |
| wasi-sdk sysroot | `wasm32-wasip1` | `libsqlite3-sys`'s own build.rs already special-cases wasi targets and ships `wasm32-wasi-vfs.c`; rusqlite exposes a matching `wasm32-wasi-vfs` feature. Good for wasmtime, not for a tab. |

### 4.3 Blocked by construction, fixed by feature-gating

- **`axum` / `tower-http` / `tokio`** are unconditional dependencies
  (`Cargo.toml:77-79`) and pull `mio`:
  `error: This wasm target is unsupported by mio`. But **nothing in the library
  uses axum** — it is `src/server.rs` and `src/server/` only. Moving these
  behind a `server` feature unblocks the build and is worth doing on its own
  merits.
- **`ureq`** (`src/provider.rs:311,442`) — blocking sockets. Gate it, or add a
  `fetch`-based provider.
- **`rudof_lib`** (the `shacl` feature) pulls `clap`, `crossterm`, `reqwest`.
  Not wasm-viable. See §7.
- **`ort`** (`onnx`, `load-dynamic`) and **`lancedb`** — out entirely. Browser
  embedding would go through onnxruntime-web in JS.

### 4.4 Source-level work

- **`getrandom`**: both 0.2 (via `ring`) and 0.3 need their `js`/`wasm_js`
  features *plus* `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'`.
- **Clocks**: `Instant::now`/`SystemTime::now` panic on
  `wasm32-unknown-unknown`. Ten sites outside tests and server, including the
  SPARQL deadline in `src/sparql/mod.rs:406` and the hot loop in
  `src/sparql/pattern.rs:52,93`, plus `src/time.rs:14`, `src/metrics.rs:60`,
  `src/governance/guard.rs:286`. Needs a `quipu::time` shim over
  `Date`/`performance.now`.
- **`std::fs`**: `pack.rs`, `signing.rs`, `store/events.rs`, `store/respace.rs`,
  `config.rs`, `metrics.rs`.
- **Threading**: single-threaded. The `open_read_only` reader pool
  (`src/store/mod.rs:273`) is moot in a tab. `parking_lot` compiles, but
  blocking on contention traps on the browser main thread — run the store in a
  Web Worker.

## 5. Practical limits — measured

This is the part that matters, and it is not what the build errors suggested.

**Method.** Synthetic Gas Town-shaped episodes: 3 nodes, 2 edges, descriptions
on every node. Each yields **20 triples / 17 fact rows / 10 events**. Native
x86-64, `--release`, `--no-default-features`, SQLite on local disk. Queries:
a bound-subject point lookup, a `?s a :Service` type scan with `LIMIT 100`, and a
2-hop join (`?d :targets ?s . ?s a :Service`) with `LIMIT 100`.

### 5.1 Storage — linear and heavy

| Episodes | Triples | DB size | Bytes/episode |
|---:|---:|---:|---:|
| 1,000 | 20,000 | 8.4 MB | 8,376 |
| 2,500 | 50,000 | 20.8 MB | 8,302 |
| 5,000 | 100,000 | 41.6 MB | 8,312 |
| 10,000 | 200,000 | 83.3 MB | 8,333 |
| 20,000 | 400,000 | 168.9 MB | 8,444 |

**~8.3 KB per episode, ~416 bytes per triple**, dead linear. Ingest holds at
315–390 episodes/s.

Where it goes, at 10k episodes (83.3 MB total, via `dbstat`):

| Component | Bytes | Share |
|---|---:|---:|
| `events` + its 2 indexes | 25.2 MB | **30.3%** |
| `facts` table | 10.0 MB | 12.0% |
| `facts` indexes (eavt, aevt, vaet, geav, tx, autoindex) | 34.5 MB | **41.4%** |
| `terms` + autoindex | 2.9 MB | 3.5% |
| `transactions` | 0.6 MB | 0.7% |

Two things stand out. **The fact indexes cost 3.4× the facts themselves.** And
**the event log is 30% of the database** — durable with no expiry by design (the
reactor-down-6wk fix), which is correct for a server with consumers and pure
overhead for a browser that has none.

### 5.2 Queries — the wall

| Episodes | Point lookup | Type scan (LIMIT 100) | 2-hop join (LIMIT 100) |
|---:|---:|---:|---:|
| 500 | 0.03 ms | 2.8 ms | 1,187 ms |
| 1,000 | 0.10 ms | 5.7 ms | 4,709 ms |
| 2,000 | 0.11 ms | 13.3 ms | 20,630 ms |
| 4,000 | 0.11 ms | 24.4 ms | 84,444 ms |
| 10,000 | 0.12 ms | 58.2 ms | — |
| 20,000 | 0.22 ms | 145.9 ms | — |

- **Point lookups are flat.** Bound-subject reads are effectively free at any
  size tested. Good.
- **Type scans are linear** in store size — `LIMIT` is not pushed down, so the
  scan is done before the limit applies.
- **The 2-hop join is quadratic.** Doubling episodes multiplies time by
  3.97, 4.38, 4.09. Fitting `t = k·n²` with k = 4.75 µs/episode² predicts 76 s
  at 4,000 against 84 s measured.

The cause is in `src/sparql/triple.rs:35-45`. `eval_bgp` is a **nested-loop join
with no join reordering and no hash join**: for each pattern, for each row
accumulated so far, it issues a fresh SQL query. Worse, the binding code calls
`store.resolve()` and `store.lookup()` per result row per pattern — two more
dictionary round-trips each. A 2-pattern BGP whose first pattern yields N rows
costs N+1 statements plus ~2N dictionary lookups.

What that model predicts:

| 2-hop join budget | Max episodes |
|---|---:|
| 1 s | ~460 |
| 10 s | ~1,450 |
| 30 s (default `query_timeout_ms`) | ~2,510 |

Observed behaviour matches: the join completes in 20.6 s at 2,000 episodes and
times out at 2,500.

### 5.3 So: can we hit low millions of episodes?

**No — and not because of wasm.** At 1,000,000 episodes:

- **Storage: ~8.3 GB.** Beyond the memory VFS entirely. On OPFS it is within
  quota on a large disk, but it is a big ask for a browser origin.
- **A single 2-hop join: ~55 days.** Six orders of magnitude past usable.

The honest framing is that **Quipu cannot do low millions of episodes on any
target today.** The interactive ceiling for join queries is ~10³–10⁴ episodes,
native, on this branch. wasm inherits that ceiling; it does not cause it.

### 5.4 Memory ceilings, once the join is fixed

- `wasm32` has a **4 GB address space**. [Memory64 is shipping in every browser
  except Safari as of early 2026][memory64] with browser caps around 16 GB, but
  it costs the engine's 32-bit pointer optimizations — a real performance
  penalty, and only worth taking above 4 GB.
- **Memory VFS**: the database must fit in linear memory. A practical 1–2 GB
  budget means **~120k–240k episodes** at today's 8.3 KB.
- **OPFS VFS**: the database lives on disk; linear memory holds only page cache
  and working set. Size is bounded by origin quota instead —
  [Chrome allows an origin up to 60% of total disk][opfs], typically hundreds of
  MB to several GB, and [OPFS-backed SQLite handled 8–10 concurrent workers
  reliably in 2026 testing][powersync].

**OPFS is therefore not optional if scale matters.** The memory VFS is a
development convenience.

### 5.5 What would actually raise the ceiling

Ranked by leverage. None of these are wasm work:

1. **Replace the nested-loop BGP join** with hash joins and join reordering — or
   push whole BGPs into a single SQL statement and let SQLite plan them. This is
   the single highest-leverage change available in the codebase; it converts
   2-hop joins from O(n²) to roughly O(n).
2. **Cache the term dictionary in memory.** Removes ~2N SQL round-trips per
   pattern from every result set.
3. **Push `LIMIT` into the SQL** so bounded queries stop being linear scans.
4. **Event-log retention policy.** Reclaims ~30% of storage; in a browser with
   no consumers, not writing events at all is defensible.
5. **Index and encoding trim.** Fact indexes cost 3.4× the data. `valid_from` /
   `valid_to` as ISO-8601 TEXT are ~20 bytes each where an integer would be 8.

With (1) and (2), low millions becomes a genuine conversation. Without them it
is not, in a browser or on a server.

### 5.6 The number we do not have

**Wasm-vs-native throughput has not been measured.** Expect SQLite-in-wasm to be
slower than native, but no figure in this document is a wasm figure and none
should be quoted as one. Producing that number is Phase 0.

## 6. Export to SQLite — preserved, and nearly free

The `.db` file stays the interchange format.

- **In-browser persistence**: OPFS gives SQLite real random-access file storage
  that survives refresh and browser close.
- **Export**: rusqlite's own `serialize` feature (not currently enabled here)
  wraps `sqlite3_serialize` — hand it a connection, get the exact bytes of a `.db`
  file, hand those to a `Blob`, and it downloads. The result opens in `sqlite3`
  and in `quipu` unchanged.
- **Import**: `sqlite3_deserialize` takes the bytes back.
- **Packs work as-is.** `src/pack.rs` re-interns every fact through
  `transact_to_graph` rather than copying rows, so term ids and `Value::Ref`
  payloads are correct by construction — a browser-produced pack is portable to
  any other store with no remapping.

Round trip: browser → OPFS → serialize → download → `quipu attach`. No exporter
to write, no format divergence.

## 7. Open question: SHACL

`rudof_lib` pulls `clap`, `crossterm` and `reqwest`, and `shacl` is a **default**
feature (`Cargo.toml:93`) and `required-features` on both binaries. A wasm build
is therefore `--no-default-features` — **no validation on write**.

For a project whose stated pitch is strict ontology enforcement, a build that
silently accepts unvalidated facts is a contradiction, not a missing feature.
The options:

| Option | Consequence |
|---|---|
| **Query-only wasm** | No write path in wasm. Honest, smallest, and matches use cases 2 and 3. Recommended for v1. |
| **Validate on import** | Browser writes are provisional; validation happens when the pack reaches a server. Needs a clear provisional/validated distinction on the wire. |
| **Wasm-capable SHACL** | Largest scope; would need a `rudof` that builds for wasm, or an in-tree validator. |

This is a contract decision, not an implementation detail, and it should be made
before code is written. It also interacts with FR-3-style tiering: a fact
written in a browser without validation is not the same kind of fact as one that
passed shapes, and the response should not be able to claim otherwise.

## 8. Plan

**Phase 0 — Measure.** Get the wasm-vs-native number. A minimal
`sqlite-wasm-rs` harness ingesting episodes and running the three queries above.
Everything downstream is scoped by this result.

**Phase 1 — Decouple (ships independently, valuable regardless).** Move
`axum` / `tower-http` / `tokio` / `ureq` behind a `server` feature. Pure hygiene;
the library should not carry a web server.

**Phase 2 — Portability shims.** `quipu::time` over the ten clock sites; gate
`std::fs`; wire the `getrandom` features and the `RUSTFLAGS` cfg.

**Phase 3 — VFS.** `sqlite-wasm-rs` via `[patch]`. Memory VFS first, OPFS
second.

**Phase 4 — Export/import.** rusqlite `serialize`/`deserialize`, plus a pack
round-trip test that asserts a browser-produced pack opens natively.

**Phase 5 — CI.** A `wasm32-unknown-unknown` job in the matrix. Per
`AGENTS.md`, the feature does not ship dark — this lands *with* the feature, not
after it.

**Not in this plan, but gating the headline use case:** the join evaluator (§5.5
items 1–3). A wasm build without it is real and useful at 10⁴–10⁵ episodes —
one agent's working memory, one repository's knowledge, one knowledge pack.
That is a good v1. Low millions is a separate, larger project against
`src/sparql/`, and it should be scoped as one rather than smuggled in under a
wasm banner.

---

## References

- [WebContainers: Node.js in the browser][wc]
- [bolt.new — agent driving a WebContainer][bolt]
- [WebAssembly sandboxing for AI agents][sandbox]
- [Memory64 browser support][memory64]
- [OPFS and storage quota][opfs]
- [State of SQLite persistence on the web, May 2026][powersync]

[wc]: https://blog.stackblitz.com/posts/introducing-webcontainers/
[bolt]: https://github.com/stackblitz/bolt.new
[sandbox]: https://thenewstack.io/webassembly-sandboxing-ai-agents/
[memory64]: https://caniuse.com/wf-wasm-memory64
[opfs]: https://web.dev/articles/origin-private-file-system
[powersync]: https://powersync.com/blog/sqlite-persistence-on-the-web
