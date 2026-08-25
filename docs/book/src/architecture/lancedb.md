# LanceDB Vector Backend

> **Implementation status (2026-08-25):** 🟩 **Selectable from config.** The
> backend is fully built and trait-conformant — `LanceVectorStore` behind
> `#[cfg(feature="lancedb")]` (`src/vector_lance.rs`), all
> `KnowledgeVectorStore` methods including `only_if()` predicate pushdown, the
> `VectorSearchDelegate` wrapper, and `quipu migrate-vectors`
> (`src/migration.rs`) — and since quipu-lv7 the binaries **read
> `vector.backend`** and install it at open
> (`src/config/vector_backend.rs`; `src/cli_open.rs` for every `quipu`
> subcommand, `src/server/base.rs` for `quipu-server`). `Store::vector_store()`
> already preferred a local backend over the built-in table, so selecting it is
> all search, resolution, auto-embed and the MCP/REST search tools needed.
>
> **Two things to know before turning it on:**
>
> - **The shipped release binaries are NOT built with the feature.** `lancedb`
>   is deliberately outside the `full` bundle — protoc plus the whole datafusion
>   tree is a real cost for a backend most deployments do not use. A binary
>   built without it **refuses** `backend = "lancedb"` at startup, naming the
>   rebuild, rather than falling back to the SQLite table: a deployment that has
>   run `quipu migrate-vectors` would otherwise have every search answered out
>   of the store it migrated away from. Build with
>   `cargo build --features full,lancedb` to ship it.
> - **It needs a Tokio runtime.** `quipu-server` is `#[tokio::main]`; the CLI
>   enters one for the whole dispatch when the configured backend requires it.
>
> *(This banner previously read "code-complete but inert in the shipped
> binaries" — accurate when it was written, and the shape that rots into a false
> affordance if left: `set_local_vector_backend` had zero non-test callers and
> `vector.backend` was set-but-not-read, so `migrate-vectors` moved embeddings
> into a store nothing then selected.)*

Quipu supports two vector storage backends: the default SQLite backend and
an optional LanceDB backend for production workloads. Both implement the
`KnowledgeVectorStore` trait.

## Dual-Backend Architecture

```text
                    ┌──────────────────────────┐
                    │  KnowledgeVectorStore     │
                    │         (trait)           │
                    └────────┬─────────────────┘
                             │
              ┌──────────────┴──────────────┐
              │                             │
     ┌────────┴────────┐         ┌──────────┴──────────┐
     │  SQLite (default)│         │  LanceDB (optional) │
     │  Brute-force     │         │  ANN + pushdown     │
     │  cosine sim      │         │  Arrow columnar     │
     └─────────────────┘         └─────────────────────┘
```

| Aspect | SQLite | LanceDB |
|--------|--------|---------|
| Storage format | f32 BLOB in `vectors` table | Arrow RecordBatch columns |
| Search algorithm | Brute-force cosine similarity | Approximate nearest neighbor |
| Predicate pushdown | No (5x oversampling fallback) | Yes (`only_if()` clause) |
| Complexity | O(n) scan | O(log n) with filter |
| Metadata columns | entity\_id, text, valid\_from, valid\_to | + entity\_type, source\_episode |
| Async requirement | None | Tokio runtime required |
| Feature flag | Always available | `lancedb` feature |

## Enabling LanceDB

Two steps, and both are required — the feature compiles the backend, the config
key selects it.

**1. Build with the feature.**

```bash
cargo build --features full,lancedb
```

**2. Select it in `.bobbin/config.toml`.**

```toml
[quipu.vector]
backend = "lancedb"
lancedb_path = ".bobbin/quipu/quipu-vectors"
```

Moving existing embeddings across first is one command:

```bash
quipu migrate-vectors --from sqlite --to lancedb --dry-run   # see the count
quipu migrate-vectors --from sqlite --to lancedb
```

Selecting the backend on a directory that has never been written creates the
empty table, so a fresh deployment does not have to migrate first.

### As a library dependency

Add the `lancedb` feature flag:

```toml
[dependencies]
quipu = { git = "https://github.com/scbrown/quipu", features = ["lancedb"] }
```

Or build from source:

```bash
cargo build --features lancedb
```

## The KnowledgeVectorStore Trait

Both backends implement this trait (defined in `src/vector.rs`):

```rust
pub trait KnowledgeVectorStore {
    fn embed_entity(&self, entity_id: i64, text: &str,
                    embedding: &[f32], valid_from: &str) -> Result<()>;
    fn close_embedding(&self, entity_id: i64, valid_to: &str) -> Result<()>;
    fn vector_search(&self, query: &[f32], limit: usize,
                     valid_at: Option<&str>) -> Result<Vec<VectorMatch>>;
    fn vector_search_filtered(&self, query: &[f32], limit: usize,
                              filter: Option<&str>,
                              valid_at: Option<&str>) -> Result<Vec<VectorMatch>>;
    fn vector_count(&self) -> Result<usize>;
}
```

The `Store::vector_store()` method returns `&dyn KnowledgeVectorStore`,
so calling code is backend-agnostic.

## Delegated Vector Search

When Quipu is used as a Bobbin dependency, embeddings are rebuildable derived
data that belong in the index layer (Bobbin), not the durable knowledge layer
(Quipu). The `VectorSearchDelegate` trait enables this separation:

```rust
pub trait VectorSearchDelegate: Send + Sync {
    fn vector_search(&self, query: &[f32], limit: usize,
                     valid_at: Option<&str>) -> Result<Vec<VectorMatch>>;
    fn vector_search_filtered(&self, query: &[f32], limit: usize,
                              filter: Option<&str>,
                              valid_at: Option<&str>) -> Result<Vec<VectorMatch>>;
    fn text_search(&self, query: &str, limit: usize,
                   valid_at: Option<&str>) -> Result<Vec<VectorMatch>>;
    fn vector_count(&self) -> Result<usize>;
}
```

When a delegate is set via `Store::set_vector_search_delegate()`:

- **Search forwards to delegate**: `vector_store()` returns a wrapper that
  routes all search calls to the delegate
- **Auto-embedding is skipped**: the transact hook does not generate embeddings
  (Bobbin owns the embedding lifecycle)
- **Write methods are no-ops**: `embed_entity()` and `close_embedding()` on the
  delegated store silently succeed without writing

When no delegate is set (standalone mode), Quipu falls back to its own
SQLite or LanceDB vectors as before.

## Hybrid Search with Predicate Pushdown

The `quipu_hybrid_search` tool uses a three-phase approach:

**Phase 1 -- Extract pushdown filter.** Simple SPARQL type patterns
(`?s a <TypeIRI>`) are converted to a SQL filter string like
`entity_type = 'TypeIRI'`.

**Phase 2 -- Vector search with filter.** The filter is passed to
`vector_search_filtered()`:

- **LanceDB**: applies the filter during ANN search (`only_if()` clause),
  so only matching vectors are scanned
- **SQLite**: ignores the filter and oversamples by 5x, relying on
  post-filtering

**Phase 3 -- Post-filter by SPARQL candidates.** The full SPARQL query
executes independently, and vector results are intersected with SPARQL
results for consistency.

```text
SPARQL: SELECT ?s WHERE { ?s a <Person> }
                │
                ├─► Extract type filter: entity_type = 'Person'
                │
                ├─► Vector search with pushdown (LanceDB)
                │       or oversample 5x (SQLite)
                │
                └─► Post-filter: intersect with SPARQL candidates
                        │
                        ▼
                    Ranked results
```

## Embedding Dimensions

All backends use 384-dimensional float32 vectors, compatible with the
`all-MiniLM-L6-v2` model. When running as a Bobbin subsystem, the shared
ONNX embedding pipeline provides vectors automatically.

## Temporal Awareness

Both backends track `valid_from` and `valid_to` for each embedding:

- Current embeddings have `valid_to = NULL`
- Expired embeddings are excluded from searches unless `valid_at` is specified
- Time-travel queries (`valid_at`) return embeddings active at that timestamp
