# Search-rooted SPARQL

POST `/search_query` retrieves seeds before evaluating SPARQL. The existing
`/hybrid_search` instead filters with SPARQL before vector ranking.

```json
{
  "query": "ClosePatch",
  "mode": "keyword",
  "seed_limit": 20,
  "seed_variable": "s",
  "sparql": "SELECT ?s ?next WHERE { ?s <https://example.org/next> ?next }",
  "query_options": {"verbose": true}
}
```

Mode defaults to keyword and needs no embedding provider. Semantic uses the
existing vector retriever. Hybrid requires both retrievers and fuses their
distinct IRIs using reciprocal rank fusion with constant 60. Supply an
embedding array for reproducibility, or use the configured query embedder.
No failed retriever silently falls back; keyword refuses embeddings.

The lexical index must be explicitly enabled and fully backfilled. This read
never activates or backfills it. Lexical search covers ROOT only. Optional
entity_type, group_ids and valid_at use the existing retriever contracts;
they do not establish tenant isolation. Query datasets are selected separately
through SPARQL or query_options.graph.

Seed limit is an integer in 1–100, default20. Each retriever contributes at most
that many entities. The response includes seed IRIs, keyword_rank and
semantic_rank (null for a missing contribution), and rrf_score. Candidate count
and seeds_truncated describe fusion, not all matching entities.
Search_complete=false explicitly marks bounded retrieval. Query result order
is determined by SPARQL rather than seed rank.

The parsed SELECT receives a VALUES relation beneath projection, aggregation,
sorting and limits. Empty search produces an empty relation, never an
unrestricted query. The seed variable must occur in the graph pattern.
The initial form supports basic triple patterns, GRAPH and joins, with outer
filters and solution modifiers. OPTIONAL, UNION, paths, SERVICE and subqueries
refuse because the existing bind-join optimization does not cover them.
Ordinary query deadlines, join/result bounds and configured label floors apply;
a small seed set does not make arbitrary traversal cheap.

The result field contains the ordinary query response, preserving inference,
labels and truncation markers. Query_options forwards ordinary query options,
but cannot override query text, valid_at, transaction time or federation.
Use top-level valid_at for retrieval and traversal together. Transaction-time
search is unsupported. This is a REST and Rust library operation; a fleet MCP
adapter must expose it explicitly before it is available as an MCP tool.

Rollback removes the new read endpoint/library call. No schema, facts, shapes,
index activation or stored query registrations are changed by this feature.
