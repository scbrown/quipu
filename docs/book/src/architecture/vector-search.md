# Vector Search

Quipu stores vector embeddings alongside facts in SQLite and supports
cosine similarity search with temporal awareness.

## How It Works

Each entity can have an associated embedding -- a float vector that
captures its semantic meaning. Embeddings are stored in a `vectors` table
with bitemporal validity (same model as the fact log).

```text
vectors(entity_id, text, embedding, valid_from, valid_to)
```

Search computes cosine similarity between a query vector and all current
embeddings, returning the top-N matches ranked by score.

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

1. Execute a SPARQL query to find candidate entities
2. Compute vector similarity for each candidate
3. Return results ranked by similarity score

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

## Temporal Vector Search

Pass `valid_at` to search embeddings as they existed at a past point in time:

```rust
let results = store.vector_search(&query, 10, Some("2026-03-01T00:00:00Z")).unwrap();
```

Expired embeddings (where `valid_to` is set) are automatically excluded
from current searches.
