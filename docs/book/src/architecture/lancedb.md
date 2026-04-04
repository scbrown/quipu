# LanceDB Vector Backend

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
