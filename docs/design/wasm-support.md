# Design: WebAssembly Support — running Quipu without a server

> **Implementation status (2026-08-13):** 🚧 **Phases 1–2 landed; the rest
> designed.** Every blocker below was verified by building against
> `wasm32-unknown-unknown`, not inferred from the manifest. Every performance
> number was measured on this branch, native x86-64, `--release`,
> `--no-default-features`. The one number NOT measured is wasm-vs-native
> throughput; obtaining it is Phase 0. The browser test harness the remaining
> phases need is verified and documented in §9 — including in the headless
> remote container this work was previously triaged as blocked in.
>
> **Depends on [in-memory-read-model.md](in-memory-read-model.md).** The query
> architecture is decided there; this document is downstream of it.

**Summary:** Quipu's graph core is already wasm-clean — the whole
Oxigraph-family stack, `ring`, `petgraph`, `datafrog` and `regex` compile for
`wasm32-unknown-unknown` today. The four crates the library did not actually use
(`axum`, `tower-http`, `tokio`, `ureq`) are now feature-gated and gone from a
default build (§4.3). What remains is `getrandom`'s wasm backend, the clock and
filesystem shims (§4.4), and the one hard blocker: SQLite's C dependency (§4.2).

**What wasm should carry is not the episode log.** It is a *distilled* knowledge
pack — the derived layer produced by the reasoner, community detection and
ontology rules — served read-only over an in-memory read model. Under that
framing the browser holds tens of megabytes rather than gigabytes, the O(n²)
join ceiling is designed around rather than inherited, and SHACL's absence stops
being a contradiction because there is no write path to validate.

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

**The SPARQL engine keeps no in-memory index** — SQLite's B-trees are its index
and its page cache is its working set. `open_in_memory()` is just the `:memory:`
VFS: identical schema, identical SQL, pages in RAM.

That is *not* true of Quipu as a whole, and the distinction matters for this
port. `graph::project()` (`src/graph.rs:50`) pulls **every** current fact into a
`Vec` via `current_facts()` and builds a petgraph `DiGraph`; it backs PageRank,
shortest path, connected components and Louvain. `impact()`
(`src/impact.rs:79`) walks the store with one indexed lookup per frontier node.
So the codebase already contains both a full in-memory materialization and an
anchored walk that scales — see
[in-memory-read-model.md](in-memory-read-model.md) §2 for the measured
comparison. It is specifically `eval_bgp` that has neither.

For the port, the consequence is the same either way: **we do not have to move a
storage engine to wasm, we have to give SQLite somewhere to put pages** — and
then decide how much of the graph is resident on top of it.

## 3. What runs in the browser: the distilled layer

The unit shipped to wasm is **a knowledge pack of the derived graph**, not the
episode log.

Episodes are raw ingest — append-heavy, high-volume, valuable in aggregate and
individually dull. What agents actually consume is the knowledge *derived* from
them, and Quipu already produces it: `src/reasoner/` derives `affects` /
`dependsOn` from raw EAVT with `source = "reasoner:<rule-id>"` provenance;
`graph.rs` Louvain with `persist: true` consolidates emergent structure into
stated facts; `src/derivation.rs` records how a fact can be recomputed;
`src/context/` is the pipeline that serves agents. Named graphs give the derived
layer somewhere to live that extends ROOT without mutating it, and
`docs/design/knowledge-packs.md` already defines a pack as a self-contained,
attachable store file.

So the split is:

| Layer | Lives | Shape | Scale |
|---|---|---|---|
| Episode log | Server, SQLite | Append-heavy, bitemporal, governed | 10⁵–10⁶ episodes |
| Derived graph | Pack → browser, resident | Read-mostly, small, distilled | 10⁴–10⁵ entities |

This is what makes the use cases below tractable rather than aspirational —
a distilled pack is tens of megabytes where the raw log is gigabytes (§5).

### 3.1 Use cases

1. **Local-first agent memory.** Today Quipu needs a server. In wasm it is a
   library an agent embeds — browser extension, Electron/Tauri app, VS Code web
   extension. Ontology-derived memory that never leaves the device. For
   privacy-sensitive domains that is a product, not a feature.
2. **Browser-based agent harnesses.** [WebContainers][wc] run a full Node.js
   runtime in-browser via wasm; [bolt.new][bolt] is an agent driving one —
   filesystem, package manager, terminal, dev server, all in-tab. VS Code for
   Web and JupyterLite are the same shape. These have **no backend to run
   `quipu-server` against**; a wasm build is the whole integration.
3. **Zero-install evaluation.** Drop a knowledge pack on a web page and query
   it — nearly free once wasm lands, and the best demo Quipu could have.
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

- ✅ **`axum` / `tower-http` / `tokio` / `ureq` — RESOLVED** (`quipu-as2`).
  These were unconditional dependencies pulling `mio`
  (`error: This wasm target is unsupported by mio`), even though **nothing in
  the library used axum** — it is `src/server.rs` and `src/server/` only.

  They now sit behind **two** features rather than one, because they are
  different directions and only one of them is a server:

  | Feature | Carries | Why separate |
  |---|---|---|
  | `remote` | `ureq` | The federation **client** (`RemoteProvider`). An HTTP client is not a server, and a build may want one without the other. |
  | `server` | `axum`, `tower-http`, `tokio` | The REST/MCP **server**. Implies `remote`, because `quipu-server` builds a federated provider from config at startup. |

  Both default OFF; `lancedb` picked up its own `tokio` dependency, and `full`
  gained `server`. Verified: `cargo tree --no-default-features` has no `axum`,
  `tower-http`, `tokio`, `ureq`, `mio` **or** `hyper`. The wasm build now gets
  past `mio` and stops at `getrandom` (§4.4) — the next blocker in line.
- **`rudof_lib`** (the `shacl` feature) pulls `clap`, `crossterm`, `reqwest`.
  Not wasm-viable. See §7.
- **`ort`** (`onnx`, `load-dynamic`) and **`lancedb`** — out entirely. Browser
  embedding would go through onnxruntime-web in JS.

### 4.4 Source-level work

- **`getrandom`**: both 0.2 (via `ring`) and 0.3 need their `js`/`wasm_js`
  features *plus* `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'` (the §4.3
  build line above carries the flag).
- **Clocks** — ✅ **shimmed (quipu-gsg).** `Instant::now`/`SystemTime::now`
  panic on `wasm32-unknown-unknown`; every lib clock read now routes through
  `quipu::time` — `epoch_secs()` (wall clock), `Deadline` (the SPARQL query
  budget: `TemporalContext.deadline`, the progress handler, and the evaluator's
  in-loop polls), and `Stopwatch` (elapsed reporting). Native arms keep
  `SystemTime`/`Instant`; the wasm32 arms read `js_sys::Date::now()`
  (target-gated `js-sys` dep — wall-clock, which a query budget tolerates).
  Direct `Instant`/`SystemTime` calls remain only in `src/server.rs` (never
  compiles for wasm) and tests. Unverifiable against a real wasm build until
  the §4.2 SQLite blocker (quipu-qd2) falls — revisit the wasm arms then.
- **`std::fs`** — ✅ **gated (quipu-gsg).** The file-IO surface is
  `#[cfg(not(target_arch = "wasm32"))]`: `pack`/`unpack`/`read_manifest`/
  `verify`/`pack_turtle` (the in-memory pack halves — `canonical_content`,
  `content_hash`, `Manifest` — stay portable), `signing::load_or_generate`
  (construct a `SigningIdentity` from key bytes instead on wasm),
  `respace_file`, and `QuipuConfig::load`/`load_from` (configure
  programmatically on wasm). `metrics::process_memory` was already
  Linux-gated. `store/events.rs` no longer touches `std::fs` (the push
  delivery moved).
- **Threading**: single-threaded. The `open_read_only` reader pool
  (`src/store/mod.rs:273`) is moot in a tab. `parking_lot` compiles, but
  blocking on contention traps on the browser main thread — run the store in a
  Web Worker.

## 5. Practical limits — measured

**The episode log does not fit in a browser, and does not need to.** This
section sizes both layers so the split in §3 is a measurement rather than an
assertion.

### 5.1 Episode-log cost (server side)

Measured with `examples/scale_bench.rs` — synthetic Gas Town-shaped episodes
(3 nodes, 2 edges, descriptions), yielding **20 triples / 17 fact rows / 10
events** each. Native x86-64, `--release`, `--no-default-features`.

| Episodes | Triples | DB size | Bytes/episode |
|---:|---:|---:|---:|
| 1,000 | 20,000 | 8.4 MB | 8,376 |
| 2,500 | 50,000 | 20.8 MB | 8,302 |
| 5,000 | 100,000 | 41.6 MB | 8,312 |
| 10,000 | 200,000 | 83.3 MB | 8,333 |
| 20,000 | 400,000 | 168.9 MB | 8,444 |

**~8.3 KB per episode, ~416 bytes per triple**, dead linear. Ingest holds at
315–390 episodes/s. Where it goes at 10k episodes (83.3 MB, via `dbstat`):

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
overhead for a browser that has none. A pack export carries neither.

> **`idx_eavt` dropped (quipu-fcg).** Measured per-index at 10k episodes:
> eavt 9.6 MB, vaet 9.4 MB, aevt 9.3 MB, the `(e,a,v,tx)` PK autoindex
> 6.3 MB, geav 5.9 MB, tx 2.0 MB. `EXPLAIN QUERY PLAN` across the
> representative mix (point lookup, e+a, predicate scan, reverse-v, a+v,
> unbound default-graph, `GRAPH ?g`, valid-time travel, facts-by-tx, the
> write path's close/exists probes, the event log's e-only prior-fact check)
> chose `idx_eavt` for **nothing**: since `idx_geav (g,e,a,v)` landed
> (quipu #36) every hot path binds `g` alongside `e` — SPARQL pushes a graph
> condition on every triple pattern and the direct read paths are ROOT-scoped
> (quipu #56) — and the one e-only probe is a covering lookup on the PK
> autoindex. Every plan is **identical** with it absent. A fresh 10k ingest
> lands at 73.3 MB vs 83.3 (−12.0%, 7,329 bytes/episode) at a slightly
> faster 332 episodes/s (one fewer index to maintain per write); dropping
> the index from an existing 10k store and vacuuming lands at 69.2 MB
> (**−16.9%**). Removed from `INIT_SQL`;
> `migrate_drop_eavt` drops it from existing stores on open, after
> `migrate_named_graphs` guarantees `idx_geav` exists. The remaining
> permutations each own an access pattern the others cannot serve (aevt:
> predicate scans; vaet: reverse value lookups; geav: everything g-scoped;
> tx: retraction/event paths) — **keep**. Timestamp re-encoding
> (TEXT→INTEGER, ~20 bytes → 8 per bound) is deliberately NOT smuggled into
> this change; it touches the bitemporal core and needs its own bead.
>
> **Retention landed (quipu-9z9).** `Store::prune_events` deletes events by
> age but never past any registered consumer's committed offset, so the
> durable-replay guarantee survives — a lagging consumer's backlog is retained
> regardless of age. Opt-in via `[quipu.events] retention_days` (server prunes
> hourly); unset keeps today's keep-forever behaviour. **Measured on this
> table's 10k-episode store:** pruning all 100,012 events (the no-consumers
> deployment this section calls pure overhead) takes the database from
> 83.3 MB to 53.5 MB after `VACUUM` — a **35.8% reduction**, matching the
> dbstat share above plus freed overhead.

At 1M episodes that is **~8.3 GB** — a server artifact, decisively not a browser
one.

### 5.2 The query ceiling, and why it is designed around

`eval_bgp` is quadratic: a 2-hop join takes 1.2 s at 500 episodes, 4.7 s at
1,000, 20.6 s at 2,000 and 84.4 s at 4,000, timing out against the default 30 s
budget at ~2,510. Full analysis, cause and fix in
[in-memory-read-model.md](in-memory-read-model.md) §1.

That ceiling is **not** inherited by the design in §3, for two reasons:

1. The derived layer is 10⁴–10⁵ entities, well inside where even the current
   evaluator is usable.
2. The read model replaces the nested-loop join with a hash join — measured at
   **0.294 ms** for a 10,000-row 2-hop join over 170k facts, against a SQL path
   that timed out on the same store with `LIMIT 100`.

A browser build should ship with the read model, not without it. That is the
dependency this document declares at the top.

### 5.3 What a browser actually holds

Resident cost of the read model is **~350 bytes/fact** (measured,
`examples/mem_read_model.rs`):

| Contents | Facts | Resident | Verdict |
|---|---:|---:|---|
| Distilled pack, 10⁴ derived entities | ~10⁵ | ~35 MB | ✅ Comfortable, memory VFS or OPFS |
| Distilled pack, 10⁵ derived entities | ~10⁶ | ~350 MB | ✅ OPFS, workable |
| Raw log, 10⁴ episodes | 170k | 60 MB | ⚠️ Fits, but why |
| Raw log, 10⁶ episodes | 17M | ~6 GB | ❌ Beyond `wasm32` entirely |

### 5.4 Address-space and storage ceilings

- `wasm32` has a **4 GB address space**. [Memory64 ships in every browser except
  Safari as of early 2026][memory64] with caps around 16 GB, but it costs the
  engine's 32-bit pointer optimizations — only worth taking above 4 GB, which
  the design in §3 never approaches.
- **Memory VFS**: the database must fit in linear memory. Fine for a distilled
  pack, hopeless for the episode log.
- **OPFS VFS**: the database lives on disk, linear memory holds only page cache
  and the read model. Bounded by origin quota instead — [Chrome allows an origin
  up to 60% of total disk][opfs], and [OPFS-backed SQLite handled 8–10
  concurrent workers reliably in 2026 testing][powersync].

**OPFS is the target.** The memory VFS is a development convenience.

### 5.5 The number we do not have

**Wasm-vs-native throughput has not been measured.** Expect SQLite-in-wasm and
the read model build to be slower than native, but no figure in this document is
a wasm figure and none should be quoted as one. Producing it is Phase 0.

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

## 7. SHACL — resolved by the distillation split

`rudof_lib` pulls `clap`, `crossterm` and `reqwest`, and `shacl` is a **default**
feature (`Cargo.toml:93`) and `required-features` on both binaries. A wasm build
is therefore `--no-default-features` — **no SHACL**.

Framed as "a Quipu that accepts unvalidated writes," that was a contradiction of
the project's stated pitch. Framed as §3, it is not a problem at all: **the
browser serves a pack that was validated on the server that produced it.** A
read-only consumer of an already-validated artifact has nothing to validate.

That makes **query-only the design rather than a compromise**, and it is the
recommendation. The other options stay on the table only if a browser write path
is ever wanted:

| Option | Consequence |
|---|---|
| **Query-only wasm** | ✅ Recommended. No write path, so no validation gap. Matches §3 exactly. |
| **Validate on import** | Browser writes are provisional until a server accepts them. Needs a provisional/validated distinction on the wire. |
| **Wasm-capable SHACL** | Largest scope; needs a `rudof` that builds for wasm, or an in-tree validator. |

One invariant survives regardless: a fact that never passed shapes must never be
presented as one that did. If a write path is added later, that distinction has
to be explicit in the response, not inferred.

## 8. Plan

**Phase 0 — Measure.** Get the wasm-vs-native number. A minimal
`sqlite-wasm-rs` harness ingesting episodes and running the three queries above.
Everything downstream is scoped by this result. Runnable headless — §9.3.

**Phase 1 — Decouple. ✅ LANDED** (`quipu-as2`). Split into `server` and
`remote` rather than one feature — see §4.3 for why, and for the verification
that the whole HTTP stack is now absent from a default build.

**Phase 2 — Portability shims.** `quipu::time` over the ten clock sites; gate
`std::fs`; wire the `getrandom` features and the `RUSTFLAGS` cfg.

**Phase 3 — VFS.** `sqlite-wasm-rs` via `[patch]`. Memory VFS first, OPFS
second. The reload-persistence acceptance runs headless via §9.3; the unit
suite via §9.4.

**Phase 4 — Export/import.** rusqlite `serialize`/`deserialize`, plus a pack
round-trip test that asserts a browser-produced pack opens natively.

**Phase 5 — CI.** A `wasm32-unknown-unknown` job in the matrix. Per
`AGENTS.md`, the feature does not ship dark — this lands *with* the feature, not
after it.

### 8.1 Ordering against the read model

[in-memory-read-model.md](in-memory-read-model.md) is a **prerequisite, not a
parallel track.** Its Phase 1 (bulk dictionary load) and Phases 2–3 (the read
model and routing `eval_bgp` through it) should land first, on native, where
they are independently valuable and testable against the full suite.

Shipping wasm before them would put the quadratic evaluator in a browser tab —
the worst place to discover it. Shipping after means the browser build inherits
a linear join and a resident model sized for exactly the artifact it carries.

The dependency runs one way only: nothing in the read model needs wasm.

## 9. Running the browser tests headless — verified in the remote container

Phases 0, 3 and 4 all need a real browser: Phase 3's acceptance is **OPFS
persistence across a page reload**, and §5.5's missing number is meaningless
unless measured against the OPFS VFS it would ship with. The 2026-08-12 bead
triage (quipu-qd2, quipu-ajz) marked this "not feasible in the current
container." That triage is **wrong for the Claude Code remote container** —
everything required is preinstalled or reachable, and the two load-bearing
claims below were verified by running them, not by reading compatibility
tables. Re-verify the inventory before trusting this section in a different
container image.

### 9.1 What the container provides (verified 2026-08-13)

| Piece | Where | Status |
|---|---|---|
| Chromium 141 headless | `/opt/pw-browsers/chromium-1194/chrome-linux/chrome` | ✅ preinstalled (Playwright-managed) |
| Playwright 1.56 | `/opt/node22/lib/node_modules/playwright` | ✅ preinstalled, matched to the Chromium above |
| chromedriver 147 | `/opt/node22/bin/chromedriver` | ⚠️ preinstalled, **6 majors ahead of the browser** — see §9.4 |
| `wasm32-unknown-unknown` | `rustup target add` | ✅ `static.rust-lang.org` is proxy-open |
| Crates | `index.crates.io` / `static.crates.io` | ✅ proxy-open; `sqlite-wasm-rs`'s `precompiled` feature means no emscripten needed |
| Matched chromedriver 141 | `googlechromelabs.github.io`, `storage.googleapis.com`, npm mirrors | ❌ proxy-blocked — a matched driver **cannot be downloaded**, which is why §9.4 needs the build-check bypass |

### 9.2 Verified: OPFS survives reload and relaunch in headless Chromium

The exact API `sqlite-wasm-rs`'s `opfs-sahpool` VFS sits on —
`navigator.storage.getDirectory()` → `FileSystemFileHandle.createSyncAccessHandle()`
inside a Web Worker — works in this headless Chromium, served over
`http://localhost` (a secure context, which OPFS requires). Probe: a worker
writes a marker file via a sync access handle; the page is reloaded and the
worker reads it back; the browser is then fully closed and relaunched on the
same persistent profile and reads it back again.

```text
write: wrote
after reload, read:quipu-opfs-ok
after browser restart, read:quipu-opfs-ok
```

Two details matter for reproducing this:

- **Persistence lives in the profile.** OPFS is per-origin storage inside the
  browser profile. Playwright's default `launch()` uses a throwaway profile,
  so the acceptance test must use `launchPersistentContext(profileDir, ...)`
  — with a throwaway profile a relaunch would (correctly) read nothing and
  the test would be asserting the wrong thing.
- **Headless args.** `{ headless: true, args: ['--no-sandbox'] }` sufficed;
  Chromium 141's headless is the new unified mode, no `--headless=new`
  incantation needed under Playwright.

### 9.3 Route A — Playwright harness (the Phase 3 acceptance test)

`wasm-bindgen-test` has no page-reload concept, so the persistence criterion
cannot be expressed in it at all. Drive it from Node instead:

1. `wasm-pack build --target web` (or `wasm-bindgen` directly) a small
   harness crate that opens a store on the OPFS VFS, transacts, and reads
   back — the wasm side stays dumb, assertions live in the driver.
2. Serve the pkg over a localhost HTTP server (any static server; OPFS needs
   the secure context, `http://localhost` qualifies — `file://` does not).
3. From Playwright: `launchPersistentContext` → load page → transact →
   `page.reload()` → assert the read → close the context, relaunch on the
   same profile → assert again. The relaunch leg is stronger than the stated
   acceptance and costs one extra line.

This is also the natural home for the **Phase 0 spike** (quipu-ajz): the same
harness page timing ingest and the three §5 queries, once against the memory
VFS and once against OPFS, browser relaunch between runs to defeat caching.

### 9.4 Route B — `wasm-pack test` for the unit suite

For ordinary `#[wasm_bindgen_test]` tests (no reload), the standard
`wasm-pack test --headless --chrome` flow works, with two shims for the
version mismatch (§9.1 — a matched driver is not downloadable here):

- **`CHROMEDRIVER`** pointed at a wrapper script that execs the preinstalled
  chromedriver with `--disable-build-check` prepended — chromedriver 147
  refuses Chromium 141 otherwise. Verified: with the flag, a WebDriver
  session against the Playwright Chromium binary creates cleanly and reports
  `browserVersion: 141.0.7390.37`.
- **`webdriver.json`** in the crate root, setting `goog:chromeOptions.binary`
  to the §9.1 Chromium path and args
  `["--headless=new", "--no-sandbox", "--disable-dev-shm-usage"]` (the last
  two are the stock fixes for Chrome-in-container failures).

The bypass is a container-local expedient, not a pattern to ship: six majors
of WebDriver drift means an obscure protocol quirk is *possible*, so if Route
B misbehaves, suspect the mismatch first and fall back to Route A. **Phase 5
CI is unaffected** — GitHub Actions installs matched Chrome + chromedriver
pairs natively, so the workflow (quipu-ame) uses none of this.

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
