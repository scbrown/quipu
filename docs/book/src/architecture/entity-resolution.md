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

`quipu:distinctFrom` excuses exactly the pairing it names, and is stored as a
durable fact, so it is declared once rather than on every re-ingest:

```json
{ "name": "alice_smith", "type": "Person",
  "distinct_from": ["http://aegis.gastown.local/ontology/Alice"] }
```

That holds for contention too. When two nodes of one write claim the same
existing entity, the response says so in `resolution_contentions` — but it does
not pick a winner. Assigning contested entities would be a judgment made from a
similarity score the caller can see and quipu cannot justify, and quipu leaves
judgments to the reader.

```json
{
  "resolution_contentions": [
    { "iri": "http://aegis.gastown.local/ontology/Alice",
      "claimants": [ {"node": "alice_smith", "score": 0.94},
                     {"node": "a_smith", "score": 0.88} ] }
  ]
}
```

## What resolution can see

On a store with attached layers the two halves have different reach. The
canonical-name half reads the composed fact source, so it sees entities defined
in an attached knowledge pack. The embedding half does not: `vectors` is a
per-database table and a pack may carry a different embedding model, so unioning
the indexes could turn a working search into a dimension-mismatch error.

Every result therefore carries `vector_scope` — `{"kind": "whole_store"}` when
there is nothing attached, `{"kind": "local_only", "attached_layers": N}` when
the embedding half left N layers unsearched. Without it, an empty candidate list
means either "no duplicates" or "the layer your duplicate is in was never
searched", and the caller cannot tell which.

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
