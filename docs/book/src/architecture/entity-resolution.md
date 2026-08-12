# Entity Resolution

> **Implementation status (2026-08-12):** ✅ **Built.** The resolver lives in
> `src/resolution.rs` (embedding + Jaro-Winkler matching, dedup by IRI,
> `top_k`); `[quipu.resolution]` config is parsed and applied; episode ingest
> returns `resolution_hints` (`src/episode/mod.rs`); the `quipu_resolve_entity`
> MCP tool (`src/mcp/resolution.rs`) and the `POST /resolve` probe are wired,
> with `strict_mode` enforced on ingest. See `docs/design/entity-resolution.md`.

Independent ingests mint independent entities. Two agents describing the same
service — one as `example-service`, one as `Example Service` — produce two
IRIs, and every fact after that fragments across them. Entity resolution is
the countermeasure: before (or instead of) a write, it asks the graph "does
something like this already exist?" and returns scored candidates.

## Scoring

`resolve_entity` in `src/resolution.rs` runs two matchers and merges their
results:

1. **Embedding similarity** — the proposed name and properties are joined into
   one text, embedded, and searched against the existing vector index (SQLite
   or LanceDB — the same index `/search` uses, no separate store). Only runs
   when an embedding provider is configured.
2. **Canonical name matching** — the name is compared against every current
   `rdfs:label` value. A case-insensitive exact match scores `1.0`; otherwise
   Jaro-Winkler string similarity applies, which catches typos and case
   variations that embeddings miss.

Candidates above the threshold from both phases are merged, deduplicated by
IRI (highest score wins), sorted descending, and truncated to `top_k`. Each
candidate carries an explanation:

```json
{
  "iri": "http://example.org/ontology/example-service",
  "score": 0.92,
  "matched_on": "canonical_name:jaro_winkler:0.92"
}
```

`matched_on` is one of `canonical_name:exact`,
`canonical_name:jaro_winkler:<score>`, or `embedding:<score>` — agents use it
to decide how much to trust the match.

## Two surfaces

### On-demand probe

`POST /resolve` (handler `tool_resolve_entity`, also exposed as the
`quipu_resolve_entity` MCP tool) answers "what would resolution say?" without
writing anything — no transaction, no vectors, guaranteed by test rather than
by signature. It works even when `[quipu.resolution].enabled = false` and
needs no bearer token:

```bash
curl -s localhost:3030/resolve -X POST \
  -H "Content-Type: application/json" \
  -d '{"name": "example-service", "properties": {"type": "DatabaseService"}}'
```

The response is `has_matches`, `candidates`, and `count`. Optional `top_k` and
`threshold` default to the `[quipu.resolution]` config, so the probe and the
ingest path agree by construction.

### Ingest-time hints

When resolution is enabled, episode ingest
(`ingest_episode_with_resolution`) resolves each node before writing. Matches
do not block the write — they ride along in the response as
`resolution_hints`, one entry per node with candidates:

```json
{
  "tx_id": 42,
  "count": 3,
  "resolution_hints": [
    {
      "node": "example-service",
      "candidates": [
        { "iri": "http://…/example-service", "score": 0.91, "matched_on": "embedding:0.91" }
      ]
    }
  ]
}
```

The same field appears on `POST /episode` and the episode MCP tools, which
share one handler.

## What it does not do

Resolution **proposes; it never merges**. Advisory mode (the default) writes
the new entity anyway and leaves the reuse-or-keep decision to the caller.
With `strict_mode = true`, ingest goes further: a write whose node matches an
existing entity is rejected outright, and the error names the top candidate —
the caller must reuse the existing IRI, or assert `quipu:distinctFrom` to
record that the entities are intentionally separate. There is no automatic
dedup, no silent IRI rewriting, and no background merge job.

## Configuration

```toml
[quipu.resolution]
enabled = true       # ingest-time hints (default: false)
threshold = 0.85     # similarity floor, 0.0–1.0 (default: 0.85)
top_k = 3            # max candidates per entity (default: 3)
strict_mode = false  # reject near-duplicate writes (default: false)
```

Resolution is off by default, so existing write workflows are unaffected; the
`/resolve` probe answers regardless. At `threshold = 0.99` it is effectively
exact-match only.

## See also

- [REST API — `POST /resolve`](../reference/rest-api.md)
- [MCP tools — `quipu_resolve_entity`](../reference/mcp-tools.md)
- Design doc: `docs/design/entity-resolution.md`
