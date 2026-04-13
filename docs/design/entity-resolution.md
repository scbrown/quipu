# Entity Resolution

Entity resolution prevents duplicate entities from fragmenting the knowledge graph.
When an agent asserts a fact about a new entity, the resolver checks whether a similar
entity already exists and returns hints to the caller.

## How it works

On entity write (episode ingest or direct fact insert), if resolution is enabled:

1. **Embedding similarity**: The entity's name and properties are embedded and
   compared against the existing vector index (LanceDB or SQLite). Matches above
   the configured threshold are returned as candidates.

2. **Canonical name matching**: The entity's name is compared against all existing
   `rdfs:label` values using Jaro-Winkler string similarity. This catches typos
   and case variations that embedding similarity may miss.

3. **Merge and dedup**: Results from both phases are merged, deduplicated by IRI
   (keeping the highest score), and truncated to `top_k`.

## Configuration

Add a `[quipu.resolution]` section to your config:

```toml
[quipu.resolution]
enabled = true       # Enable resolution (default: false)
threshold = 0.85     # Similarity threshold (default: 0.85)
top_k = 3            # Max candidates per entity (default: 3)
strict_mode = false  # Reject near-duplicates (default: false)
```

## Modes

### Advisory (default)

When `strict_mode = false`, the resolver returns candidates as hints in the
write response. The agent decides whether to reuse an existing IRI or create
a new entity.

```json
{
  "resolution_hints": [
    {
      "node": "Alice Smith",
      "candidates": [
        {
          "iri": "http://aegis.gastown.local/ontology/alice",
          "score": 0.92,
          "matched_on": "canonical_name:jaro_winkler:0.92"
        }
      ]
    }
  ]
}
```

### Strict

When `strict_mode = true`, the write is rejected if near-duplicate candidates
are found. The agent must either:

- Reuse an existing IRI, or
- Assert `quipu:distinctFrom` on the new entity to mark it as intentionally
  separate from the candidates.

## MCP tool

The `quipu_resolve_entity` tool lets agents check before writing:

```json
{
  "name": "quipu_resolve_entity",
  "input": {
    "name": "Alice Smith",
    "properties": { "role": "engineer" },
    "threshold": 0.85,
    "top_k": 3
  }
}
```

Response:

```json
{
  "has_matches": true,
  "candidates": [
    {
      "iri": "http://aegis.gastown.local/ontology/alice",
      "score": 0.92,
      "matched_on": "canonical_name:jaro_winkler:0.92"
    }
  ],
  "count": 1
}
```

## Match explanations

The `matched_on` field explains how the match was found:

| Value | Meaning |
|-------|---------|
| `canonical_name:exact` | Exact case-insensitive label match |
| `canonical_name:jaro_winkler:0.92` | Jaro-Winkler string similarity |
| `embedding:0.91` | Vector embedding cosine similarity |

## Worked example

An agent ingests an episode about infrastructure:

```json
{
  "name": "infra-audit",
  "nodes": [
    { "name": "alice_smith", "type": "Person", "description": "SRE team lead" }
  ]
}
```

The resolver finds an existing entity `Alice` with label "Alice" and similar
properties. In advisory mode, the response includes:

```json
{
  "tx_id": 42,
  "count": 3,
  "resolution_hints": [
    ["alice_smith", [{"iri": "http://.../Alice", "score": 0.91, "matched_on": "embedding:0.91"}]]
  ]
}
```

The agent can then either reuse the existing IRI or keep the new entity.

## Design notes

- The threshold is deliberately conservative (0.85) to avoid false positives.
- Resolution is disabled by default, so existing workflows are unaffected.
- At threshold 0.99, resolution is effectively off (only exact matches).
- The resolver reuses the existing LanceDB/SQLite vector index -- no new store.
- `matched_on` is the explanation field: agents use this to decide trust level.
