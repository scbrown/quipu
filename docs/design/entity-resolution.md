# Entity Resolution

> **Implementation status (2026-08-21):** ✅ **Implemented.** The resolver lives
> in `src/resolution/` (embedding + Jaro-Winkler canonical-name matching over the
> composed fact source, merge/dedup by IRI, `top_k`); config `[quipu.resolution]`
> is parsed and applied (`src/config.rs`, wired in `src/server.rs` main); episode
> ingest returns `resolution_hints` and `resolution_contentions`
> (`src/episode/mod.rs`); the `quipu_resolve_entity` MCP tool is in
> `src/mcp/resolution.rs`, registered in `src/mcp/mod.rs`.
>
> **Correcting the 2026-07-23 banner, which claimed `quipu:distinctFrom` was
> "present" and "exercised live in production".** It was not. The term appeared
> only inside the sentence of the strict-mode error message that told callers to
> assert it — no namespace constant, no shape, no read path. Strict mode refused
> writes and named an override that nothing honoured, so the only real escape was
> disabling `strict_mode` for the whole store. It is implemented now
> (`namespace::QUIPU_DISTINCT_FROM`, `resolution::recorded_distinct_from`, the
> `distinct_from` field on an episode node) and tested in both directions. The
> lesson for banners: "verified by mechanism" has to mean a mechanism was run,
> because a status line asserting a feature exists is exactly as load-bearing as
> the feature, and harder to disprove.

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

3. **Excused pairings**: candidates the writer declared, or the graph already
   records, as `quipu:distinctFrom` are dropped before anything is scored.

4. **Merge and dedup**: Results from both phases are merged, deduplicated by IRI
   (keeping the highest score), and truncated to `top_k`.

## What each half can see on a composed store

The name half reads `Store::facts_source()` — the `UNION ALL` over the local
store and every attached layer — so on a composition it sees the shared
reference layer a tenant is most likely to duplicate. It did not always: it read
the bare `facts` table, which is `main.facts`. Resolving a name that an attached
knowledge pack already defined found nothing, and the tenant minted a duplicate
of an entity it had attached the pack in order to share. Predicate lookup goes
through `lookup_all` for the same reason — a layer interns `rdfs:label` in its
own term space, so a main-scoped `a = ?` selects none of its rows.

The vector half cannot follow it there. `vectors` is a per-database table, and an
attached pack may carry embeddings from a different model or dimension, so
unioning the indexes would turn a working search into a dimension-mismatch error
(`src/vector.rs` fails loud on that, deliberately). Rather than paper over it,
every result carries a `vector_scope`:

| `vector_scope` | Meaning |
|---|---|
| `{"kind": "whole_store"}` | No attachments — the vector index covers what the name half reads |
| `{"kind": "local_only", "attached_layers": N}` | N layers the embedding half did not search |

The point is not the missing coverage; it is that missing coverage and "no
duplicates exist" used to return the same empty list.

## Contention between nodes of one write

Nodes used to resolve one at a time in a loop, so nothing could see that two of
them had claimed the same existing entity. Advisory mode emitted two hints
pointing at one IRI, which reads as two independent near-misses; strict mode
refused whichever node came first and said nothing about the other. The write
was about to fragment one entity into several and no field said so.

`resolve_nodes` resolves a whole write in one pass and reports
`resolution_contentions`: entities that are the *top* candidate for more than one
node. Top candidate, not any candidate — two nodes can both resemble a third
entity without either claiming to be it, and reporting that as conflict would
train callers to ignore the field.

**Why this is not a stable matching.** Resolving N nodes against M entities is a
bipartite assignment problem, and the reflex is Gale-Shapley. Two reasons not to.
Stability is not the property wanted: stable matching guarantees no blocking
pair, not maximum total match quality, and it is proposer-optimal, so the answer
depends on which side proposes — and with one symmetric similarity score there is
no principled proposing side. If this ever does assign, max-weight bipartite
matching (Hungarian) is the right algorithm. More fundamentally, assigning is not
this layer's job: quipu stores facts true at write time and leaves judgments to
the reader, and picking which node "gets" a contested entity is a judgment made
from a score the caller can see and quipu cannot justify. The pass detects
contention. It never resolves it.

The batch pass is also why resolution stopped being O(nodes x labels): the label
scan is hoisted out of the per-node loop, so an episode of N nodes runs one scan
instead of N, each of which previously decoded every value and ran a
Jaro-Winkler comparison and an id-to-IRI resolve per row.

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
  separate from that candidate.

`quipu:distinctFrom` excuses **exactly the named pairing** — an override that
excused everything would be `strict_mode = false` wearing a costume. It is
written as a durable fact, so the writer declares it once and later ingests of
the same entity read it back; without that, every re-ingest would have to
re-declare it and any producer that forgot would be refused again.

An episode node declares it inline:

```json
{
  "name": "alice_smith",
  "type": "Person",
  "distinct_from": ["http://aegis.gastown.local/ontology/Alice"]
}
```

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
  "count": 1,
  "vector_scope": { "kind": "whole_store" }
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
- Resolution proposes and never merges — including when it detects contention.
