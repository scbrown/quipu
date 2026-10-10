# Structured candidate search

Structured search is experimental, defaults off, and currently supports SQLite
and unattached ROOT only. Enable `[quipu.search] structured = true` in an isolated store
for evaluation. Ordinary natural-language queries keep their existing behavior.
This source packet does not authorize production activation.

Supply exactly one `structured_query` string or `filters` object alongside a
ranking `query` or precomputed semantic `embedding`. Both semantic and keyword
ranking select candidates before top-K. Keyword ranking and unfielded text
leaves require an enabled, fully backfilled keyword index.

```json
{
  "query": "single writer",
  "structured_query": "type:Service AND (status:open OR owned_by:Alice) NOT deprecated:true",
  "mode": "keyword"
}
```

The equivalent strict JSON expression uses one key per expression:

```json
{
  "and": [
    {"term": "type:Service"},
    {"not": {"term": "status:closed"}}
  ]
}
```

`and` and `or` take exactly two operands. Unknown keys, malformed expressions,
ambiguous labels, unknown prefixes and unsafe IRIs refuse explicitly. Expressions
support implicit AND, parentheses, OR, NOT, unary minus, quoted phrases and
trailing prefix wildcards. AND binds more tightly than OR. Every OR branch needs
a positive term; NOT subtracts from a bounded positive universe.

`type` means `rdf:type`. Other fields resolve through loaded shape prefixes,
full IRIs in angle brackets, or unique exact labels. For example,
`ex:status:open` or `<https://example.org/status>:open`. Values match exact
literal text or a uniquely labeled one-hop object; explicit object IRIs and
CURIEs avoid label ambiguity. Numeric ranges accept finite numbers; date ranges
accept canonical `YYYY-MM-DD` dates. Date comparison orders literal lexical
forms, so the attribute must use canonical date lexical forms as well.

The parser caps input at 4096 bytes, 64 tokens and eight nested groups. JSON
expressions cap at 64 nodes and depth eight. Each candidate set caps at 4096
entities and 32768 intermediate SPARQL rows; overflow refuses and asks for narrowing. SQLite vector ranking reads
only candidate entities, caps their vector rows at 32768, and deduplicates
before top-K. One ambient deadline spans selection and ranking. Candidate
selection and label resolution share `valid_at` with vector/keyword ranking.
A response marks structured candidates as complete only after these gates pass.

The initial packet explicitly refuses delegated/LanceDB backends, named-graph
parameters, anchors, ranking overrides, group and entity-type scope parameters.
Use explicit type/attribute expressions for ROOT candidate scope. No unsupported
scope is silently discarded and no global top-K fallback is used. Composition
with named-graph and hybrid source packets requires further review.

CLI examples:

```sh
quipu search 'single writer' --mode keyword \
  --structured-query 'type:Service status:open' --db scratch.db
quipu search 'single writer' --mode keyword \
  --filters '{"term":"type:Service"}' --db scratch.db
```

REST `/search` and native MCP `quipu_search` share the new fields. Proxy schemas
must expose them before proxy clients can request structured search. Stage
acceptance additionally requires a frozen corpus replay and measured memory,
latency and serving controls; passing source tests does not establish rollout.
