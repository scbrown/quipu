# Vector Search

Quipu stores vector embeddings alongside facts and supports cosine
similarity search with temporal awareness. Two backends are available:
the default SQLite backend (brute-force) and an optional
[LanceDB backend](lancedb.md) with approximate nearest neighbor search
and predicate pushdown.

## How It Works

Each entity can have an associated embedding -- a 384-dimensional float
vector that captures its semantic meaning (compatible with
`all-MiniLM-L6-v2`). Both backends implement the `KnowledgeVectorStore`
trait, so calling code is backend-agnostic.

The default SQLite backend stores embeddings in a `vectors` table with
bitemporal validity (same model as the fact log):

```sql
vectors(entity_id, text, embedding, valid_from, valid_to)
```

Search computes cosine similarity between a query vector and all current
embeddings, returning the top-N matches ranked by score. For larger
datasets, the [LanceDB backend](lancedb.md) provides ANN search with
predicate pushdown.

## Storing Embeddings

```rust
use quipu::store::Store;

let store = Store::open("my.db").unwrap();

// Generate embedding externally (e.g., all-MiniLM-L6-v2)
let embedding: Vec<f32> = model.encode("Traefik reverse proxy");

// Store it
store.embed_entity(entity_id, "Traefik reverse proxy", &embedding, "2026-04-04T00:00:00Z").unwrap();
```

## Searching

```rust
let query_embedding = model.encode("web proxy");
let results = store.vector_search(&query_embedding, 10, None).unwrap();

for m in &results {
    println!("{} (score: {:.3})", m.text, m.score);
}
```

Each `VectorMatch` contains:

| Field | Description |
|-------|-------------|
| `entity_id` | The matched entity's term ID |
| `text` | The text that was embedded |
| `score` | Cosine similarity (0.0 to 1.0) |
| `valid_from` | When this embedding became active |
| `valid_to` | When it expired (None = current) |

## Hybrid Search

The `quipu_hybrid_search` tool combines SPARQL filtering with vector ranking:

1. **Extract pushdown filter** -- simple type patterns (`?s a <Type>`) are
   converted to a filter string for the vector backend
2. **Vector search with filter** -- LanceDB applies the filter during ANN
   search; SQLite oversamples 5x and post-filters
3. **Cross-filter with SPARQL** -- full SPARQL query runs independently,
   results intersected for consistency

```json
{
  "tool": "quipu_hybrid_search",
  "input": {
    "sparql": "SELECT ?s WHERE { ?s a <http://example.org/WebApp> }",
    "embedding": [0.1, 0.2, ...],
    "limit": 5
  }
}
```

This lets you narrow by type or relationship first (SPARQL), then rank by
semantic meaning (vector) -- combining structured and unstructured search.
With LanceDB, the type filter is pushed down into the vector index for
O(log n) filtered search. See [LanceDB Vector Backend](lancedb.md) for
details.

## Temporal Vector Search

Pass `valid_at` to search embeddings as they existed at a past point in time:

```rust
let results = store.vector_search(&query, 10, Some("2026-03-01T00:00:00Z")).unwrap();
```

Expired embeddings (where `valid_to` is set) are automatically excluded
from current searches.
