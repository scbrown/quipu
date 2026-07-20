# Changelog

All notable changes to this project will be documented in this file.

## [0.3.4] - 2026-07-20

### Added

- *(graph)* Export project() — the typed graph API was unusable from outside([1143b7b](https://github.com/scbrown/quipu/commit/1143b7be703b15d19b843abb4204115b3c5f1903))
- *(store)* Named-graph column on facts — additive ROOT-default foundation (quipu #36)([b57bfab](https://github.com/scbrown/quipu/commit/b57bfab30eb779d2778889b3635dc2949ada6766))
- *(store)* Graph-scoped writes — overlays extend ROOT without mutating it (quipu #36)([aca75f3](https://github.com/scbrown/quipu/commit/aca75f3c74febf580acfd4a30a93bcc9c254cf53))
- *(episode)* /episode `graph` field — write knowledge into a named overlay (quipu #36)([8196a08](https://github.com/scbrown/quipu/commit/8196a084b21608028ef72437c9d78fbcb640cf33))
- *(query)* Content-negotiated W3C SPARQL 1.1 results — fix lossy shape([8e5ea77](https://github.com/scbrown/quipu/commit/8e5ea77d48589f1633467534cc441bb658c96ea5))
- *(store)* Named-graph overlay primitives — create / write / tombstone / compose (#36/#37)([bf1ecd1](https://github.com/scbrown/quipu/commit/bf1ecd13e0e409754c772a81f8ab22b85f1e70f3))
- *(shapes)* Governance-plane ("the loom") ontology + SHACL shapes (Phase 1)([325630c](https://github.com/scbrown/quipu/commit/325630c1ea1b3aabd05feaaaac4b13f6cbebb0b7))
- *(mcp)* Provenance-based work-item co-occurrence — /cooccurrence (quipu#37)([edc845d](https://github.com/scbrown/quipu/commit/edc845de025ca98887fb36b0336e53076d636a29))
- *(mcp)* Committed-tier policy evaluation — /policy/check (the loom, Phase 1 runtime)([02eff07](https://github.com/scbrown/quipu/commit/02eff07ee413d5f3ae3fc21c577303224be996d3))
- *(mcp)* Verifier registry — the Phase-0 authority layer (the loom)([c41ddcb](https://github.com/scbrown/quipu/commit/c41ddcb2b439b23bef91592774fd5beb9d23d84d))
- *(signing)* V1 verdict signing — the loom's Phase-0 root of trust([2336fc7](https://github.com/scbrown/quipu/commit/2336fc7031be230d3e0927cf5851c0ec130ea64d))
- *(retract)* Triple-level retraction — entity + predicate + value([758cf51](https://github.com/scbrown/quipu/commit/758cf5171482cde144466c561685ffb7c9196c88))
- *(shapes)* AnsibleGroup + TerraformResource node shapes([184aef4](https://github.com/scbrown/quipu/commit/184aef4197156d02e34e335e9c23094ea631dcb8))
- *(shapes)* Declare Incident, FailurePattern, SoftwareVersion, StoragePool([25e47a7](https://github.com/scbrown/quipu/commit/25e47a7e2b19274ea1b1c6cb4d0750c1c621d7ac))
- *(shapes)* Declare ClaudeCodeHook; aegis:Guard deliberately NOT declared([b7b31ad](https://github.com/scbrown/quipu/commit/b7b31ad449f5f60cebd7385fe670bb41a0715df7))
- *(server)* GET /version — the git SHA of the running build([6ee8412](https://github.com/scbrown/quipu/commit/6ee84129e2d72ca7d6f2763ea67e3a84e20a1ca8))

### Documentation

- The documented build must produce quipu-server — name --features onnx([1da8a09](https://github.com/scbrown/quipu/commit/1da8a090a21a60ae7811b7ba3a696476feffed0e))

### Fixed

- *(onnx)* Bind embedder inputs by name, not positionally([cafb2f7](https://github.com/scbrown/quipu/commit/cafb2f75375e5dffcd424d7f55f0389d6f532217))
- *(onnx)* Use real attention mask — pad tokens collapsed embeddings([cb7e620](https://github.com/scbrown/quipu/commit/cb7e62080bc6feabeb09c08053da25eb867f8112))
- *(shapes)* Reconcile SHACL shapes to what /episode actually emits([3d2cad3](https://github.com/scbrown/quipu/commit/3d2cad3dc0d0348a516db004075920391c50bd23))
- *(sparql)* IsBlank() was aliased to isIRI() — it matched the whole store([f0c70b7](https://github.com/scbrown/quipu/commit/f0c70b7fbf5b2cef674fbfe3e8488cb437c92cd3))
- *(episode)* Reject untyped nodes with a clear error, not a whole-episode Turtle 400([3769fa9](https://github.com/scbrown/quipu/commit/3769fa987cbc548d4b8205a42243ba4f84cff0b6))
- *(store)* Create idx_geav in the named-graph migration, not INIT_SQL([ae75e80](https://github.com/scbrown/quipu/commit/ae75e80e50fc9068d3cef89788895648e59f3d10))
- *(overlay)* Dedupe compose_view over re-asserted base facts (found in 69co live deploy)([e1288fe](https://github.com/scbrown/quipu/commit/e1288fe376c7b323cbeb9118a196530eebf7518f))
- *(cli)* --version/--help are pure reads, never open a store([a87ce9f](https://github.com/scbrown/quipu/commit/a87ce9f09a13fde6137150038a5e452bbab78c82))
- *(search)* Dedupe /search results by entity, keep best-scoring row([4f1c506](https://github.com/scbrown/quipu/commit/4f1c50677fa3fafae322a88c78cda7ccb95f3798))
- *(retract)* Episode retraction no longer orphans node identity([bfe7948](https://github.com/scbrown/quipu/commit/bfe7948d087affdd9448d026138ed5a3bb72e637))
- *(rdf)* Preserve language tags and datatypes in the Value model([f4d49df](https://github.com/scbrown/quipu/commit/f4d49dffa3e11ab4cc350bdef4c6ebd5bf447fad))
- *(shapes)* Add `get` (content read-back) and reject unknown actions([eb319be](https://github.com/scbrown/quipu/commit/eb319be56129be7677b77e02e0783b56f691ed87))
- *(retract)* One retraction datum per triple, not per backing row([26bd04b](https://github.com/scbrown/quipu/commit/26bd04bec37ff5af025d445d3f28c0aea2b4663a))
- *(retract)* Two-type coverage + refuse orphaning an entity's last rdf:type([0bae616](https://github.com/scbrown/quipu/commit/0bae6168f585e91187f957194f01d653693633ed))
- Scrub internal identifiers to zero, untrack the runtime store, add the RATCHET([258c6d7](https://github.com/scbrown/quipu/commit/258c6d7744cc5da2b585790a252b496666233473))
- *(auth)* Close 3 write routes that bypassed read-only + bearer auth, and enforce the list([7604448](https://github.com/scbrown/quipu/commit/7604448232809e8e7ea3b5379269c1bbfc2269b0))
- *(config)* Actually mint IRIs under the configured base_ns([7d54b10](https://github.com/scbrown/quipu/commit/7d54b105723324fbacef259519be6c29574f6739))
- *(group_ids)* Make code, doc, test and schema agree — provenance, not isolation([3b1762a](https://github.com/scbrown/quipu/commit/3b1762a453f5b46530639b08d109d0403a8638e8))
- *(server)* Re-tier 5 writing ro_handler! routes as rw, and enforce tier==classification([51a1436](https://github.com/scbrown/quipu/commit/51a1436bcd814bd75774cccc5ec9549338c01df1))

### Testing

- *(shapes)* Guard the SHACL shape invariants against drift([038c2f3](https://github.com/scbrown/quipu/commit/038c2f314024ca10f8f0309e63a5795fe0521ac9))
- *(retract)* Lang/typed literals stay precisely retractable([0579895](https://github.com/scbrown/quipu/commit/057989520b78893191aa4d7145d34821cb0f8c49))

## [0.3.3] - 2026-07-13

Graph analytics, a live report endpoint, episode-scoped retraction, and
caller-controlled CLI ingest.

### Added

- **Deterministic Louvain community detection (#31)** — community
  structure over the entity graph with stable, reproducible assignments.
- **Live graph report endpoint + `quipu_report` MCP tool (#32)** —
  on-demand orientation over the graph (size, central entities, activity)
  exposed via HTTP and MCP.
- **Episode-scoped logical retraction endpoint (#33)** — retract the
  facts asserted by a named episode without disturbing others.
- **`--base-ns` on `episode` (#28)** — override the namespace IRIs are minted in
  (defaults to the built-in aegis namespace), so non-aegis deployments can use
  the validation-carrying episode abstraction instead of routing around it.
- **`--timestamp` on `knot` / `episode` / `retract` (#27)** — supply the
  source-true `valid_from` (e.g. an upstream event time) instead of the exporter
  wall-clock, so bitemporal history imports keep their original valid-time. A
  lightweight ISO-8601 shape check rejects malformed values.

## [0.3.1] - 2026-06-27

Critical SPARQL query-engine correctness fixes.

### Fixed

- **FILTER builtins were no-ops (#12)** — `FILTER(CONTAINS(...))`, `isIRI`,
  `STRSTARTS/STRENDS`, and nested `CONTAINS(LCASE(STR(?x)), ..)` returned ALL
  rows regardless of the predicate (only `Regex` was handled; everything else
  passed through). Implemented CONTAINS/STRSTARTS/STRENDS/isIRI/isBlank/
  isLiteral/isNumeric (bool) + STR/LCASE/UCASE (value). This restores text
  search in `tool_context`/`unified_search` (and Bobbin's `knowledge_context`,
  which wraps it) and entity-linking filters.
- **COUNT / OPTIONAL inflation (#13)** — a triple re-asserted across
  transactions left multiple current rows for the same `(e,a,v)`; BGP queries
  lacked `DISTINCT`, so duplicates multiplied under OPTIONAL/joins (e.g. 23174
  rows for 11 entities) and inflated `COUNT` (the bogus Shapes-Distribution
  counts). Added `DISTINCT` to the current-fact selects (BGP, rdf:type/subclass,
  property paths) — one solution per current triple.

## [0.3.0] - 2026-06-27

Graph-algorithm ranking, cross-origin API access, and a Web UI overhaul.

### Added

- **PageRank & Personalized PageRank** over the projected graph —
  `page_rank()` + `PageRankConfig`, exposed via `tool_project`
  (`"algorithm": "pagerank"` / `"ppr"`, with `seeds`/`damping`/`max_iters`),
  the `quipu project` CLI, REST `POST /project`, and MCP. Closes the
  long-standing "centrality" gap (only `in_degree` shipped before). Consumed by
  Bobbin's PPR retrieval ranking signal.
- **CORS** on the HTTP API (`/query`, `/search`, `/episode`, …) incl. OPTIONS
  preflight, so browser clients like Bobbin's Knowledge tab can call quipu
  cross-origin (#5).

### Fixed (Web UI)

- **Graph Explorer** uses a force-directed (cose) / hierarchical layout instead
  of the unreadable grid at large entity counts (#6).
- **Timeline** orders newest-first, hides decommissioned `graphiti-fact-*`
  episodes, and shows a summary line + per-episode entity-count chips (#8;
  partial #7).
- **Workbench SPARQL editor** renders on first view (was blank when initialized
  in a hidden container) with a plain-textarea fallback (#9).

### Known issues (fast-follow, 0.3.1)

- #7 (remove `graphiti-fact-*` episode data) and #10 (merge duplicate
  `aegis:WebApplication` entities) are deploy-gated live-data migrations; the UI
  symptom for #7 is already mitigated above.

## [0.2.0] - 2026-04-12

### Reasoner

- **Impact analysis CLI** — `quipu impact <entity-IRI>` walks entity edges via
  BFS with configurable hop depth and predicate filters
  ([c49ee8e](https://github.com/scbrown/quipu/commit/c49ee8e))
- **Datalog rule engine** — rule AST, Turtle DSL parser, stratified
  negation-as-failure with cycle detection, semi-naive evaluation via `datafrog`
  with full provenance tracking; `quipu reason` CLI command
  ([1f71b44](https://github.com/scbrown/quipu/commit/1f71b44),
  [8710ea8](https://github.com/scbrown/quipu/commit/8710ea8),
  [2473eb4](https://github.com/scbrown/quipu/commit/2473eb4),
  [37c192e](https://github.com/scbrown/quipu/commit/37c192e))
- **Reactive evaluation** — `TransactObserver` keeps derived facts fresh as base
  facts change; delta-aware re-evaluation triggered only by affected predicates
  ([aab6d30](https://github.com/scbrown/quipu/commit/aab6d30))
- **Counterfactual queries** — `Store::speculate()` forks a hypothetical view via
  SQLite SAVEPOINT; `quipu impact --remove` flag, REST `POST /impact` endpoint,
  and `quipu_impact` MCP tool
  ([563e6c2](https://github.com/scbrown/quipu/commit/563e6c2))

### Web UI

- **SPARQL Workbench** — syntax-highlighted CodeMirror editor with tabular/JSON
  output, query examples library, and time-travel parameter support
  ([65b5967](https://github.com/scbrown/quipu/commit/65b5967))
- **Temporal Navigator** — episode timeline with chronological view, extracted
  entities, and metadata display
  ([fc0e0ab](https://github.com/scbrown/quipu/commit/fc0e0ab))
- **Web component export** — embeddable `<quipu-graph>`, `<quipu-sparql>`,
  `<quipu-entity>`, `<quipu-timeline>`, `<quipu-schema>` custom elements for
  embedding Quipu panels in any page
  ([2153019](https://github.com/scbrown/quipu/commit/2153019))
- **Semantic Web APIs** — Spotlight entity recognition (`POST /spotlight`),
  Triple Pattern Fragments (`GET /fragments`), OpenRefine reconciliation
  (`POST /reconcile`), and content negotiation on `/entity/{iri}`
  ([2153019](https://github.com/scbrown/quipu/commit/2153019))

### Server

- **Entity format sub-path routes** — `GET /entity/{iri}/json` and
  `/entity/{iri}/ttl` replace suffix-based routes for axum 0.8+ compatibility
  ([583de29](https://github.com/scbrown/quipu/commit/583de29),
  [4d80832](https://github.com/scbrown/quipu/commit/4d80832))

### Test Fixtures

- **Seed binary and justfile recipes** — `just fixtures seed` and
  `just fixtures load` for populating test databases with realistic data
  ([cf0518a](https://github.com/scbrown/quipu/commit/cf0518a),
  [564436e](https://github.com/scbrown/quipu/commit/564436e))

### Documentation

- Comprehensive mdbook chapters for the reasoner — concepts, rule-builder
  tutorial, and CLI reference
  ([860dec3](https://github.com/scbrown/quipu/commit/860dec3))
- Reasoner design document
  ([340a55d](https://github.com/scbrown/quipu/commit/340a55d))
- Test fixtures design document
  ([3638c16](https://github.com/scbrown/quipu/commit/3638c16))

## [0.1.0] - 2026-04-05

Initial public release.

### Knowledge Graph Core

- **EAVT bitemporal fact log** — immutable fact storage with transaction time
  and valid time, time-travel queries, full audit trail
  ([49b5321](https://github.com/scbrown/quipu/commit/49b5321))
- **RDF data model** — IRIs, blank nodes, typed literals via oxrdf; import/export
  Turtle, N-Triples, JSON-LD, RDF/XML
  ([4e44b38](https://github.com/scbrown/quipu/commit/4e44b38))
- **SPARQL 1.1 query engine** — SELECT, ASK, CONSTRUCT, DESCRIBE with BGP, JOIN,
  UNION, FILTER, OPTIONAL, ORDER BY, GROUP BY, HAVING, aggregates, BIND, property
  paths, RDFS subclass inference, and temporal queries (`valid_at`, `as_of_tx`)
  ([a742c91](https://github.com/scbrown/quipu/commit/a742c91),
  [97a9e7e](https://github.com/scbrown/quipu/commit/97a9e7e),
  [c5795ce](https://github.com/scbrown/quipu/commit/c5795ce),
  [8102262](https://github.com/scbrown/quipu/commit/8102262),
  [b839298](https://github.com/scbrown/quipu/commit/b839298),
  [46db89f](https://github.com/scbrown/quipu/commit/46db89f),
  [280ac51](https://github.com/scbrown/quipu/commit/280ac51))
- **SHACL validation** — write-time schema enforcement with persistent shape
  storage and structured feedback (severity, focus node, path, message); optional
  via `shacl` feature flag
  ([08f8cb8](https://github.com/scbrown/quipu/commit/08f8cb8),
  [cf4de8d](https://github.com/scbrown/quipu/commit/cf4de8d),
  [9949807](https://github.com/scbrown/quipu/commit/9949807))
- **Aegis ontology SHACL shapes** — pre-built shapes for infrastructure entities
  ([da19a7b](https://github.com/scbrown/quipu/commit/da19a7b))
- **Code entity SHACL shapes** — shapes for CodeModule, CodeSymbol, Document,
  Section, Bundle
  ([182dfa7](https://github.com/scbrown/quipu/commit/182dfa7))

### AI-Native Features

- **Episode ingestion** — structured write path for agent-extracted knowledge
  with typed nodes, edges, provenance tracking, SHACL validation gate, and
  batch ingestion
  ([4e26495](https://github.com/scbrown/quipu/commit/4e26495),
  [9f70a0c](https://github.com/scbrown/quipu/commit/9f70a0c))
- **Dual vector backends** — default SQLite (brute-force cosine similarity) or
  optional LanceDB (ANN with predicate pushdown, Arrow columnar storage, full-text
  search) via `--features lancedb`
  ([0723c08](https://github.com/scbrown/quipu/commit/0723c08),
  [ea669c9](https://github.com/scbrown/quipu/commit/ea669c9),
  [bb86cb6](https://github.com/scbrown/quipu/commit/bb86cb6),
  [455a8e8](https://github.com/scbrown/quipu/commit/455a8e8))
- **Hybrid search** — SPARQL filters candidates, vector similarity ranks them;
  type constraints pushed down into the vector index
  ([ff46399](https://github.com/scbrown/quipu/commit/ff46399))
- **Auto-embed on write** — entities automatically embedded at knot/episode
  ingestion time
  ([126b7ea](https://github.com/scbrown/quipu/commit/126b7ea))
- **Context pipeline** — unified knowledge context for agent consumption with
  text search, link expansion, configurable depth and budget
  ([815e640](https://github.com/scbrown/quipu/commit/815e640))
- **EmbeddingProvider trait** — shared ONNX pipeline for auto-embedding queries
  in search endpoints
  ([95e18ee](https://github.com/scbrown/quipu/commit/95e18ee))

### Interfaces

- **CLI** — `quipu knot`, `quipu read`, `quipu cord`, `quipu unravel`,
  `quipu validate`, `quipu episode`, `quipu retract`, `quipu repl`, `quipu stats`
  ([89387ad](https://github.com/scbrown/quipu/commit/89387ad),
  [3ed26ea](https://github.com/scbrown/quipu/commit/3ed26ea),
  [fe0604f](https://github.com/scbrown/quipu/commit/fe0604f))
- **REST API** — axum server mirroring MCP tool surface with Graphiti-compatible
  `/search/nodes` and `/episodes/complete` endpoints
  ([a9eb8fa](https://github.com/scbrown/quipu/commit/a9eb8fa),
  [daef471](https://github.com/scbrown/quipu/commit/daef471))
- **Web UI** — standalone graph explorer with force-directed visualization,
  SPARQL workbench, episode timeline, and schema inspector
  ([32cf2ae](https://github.com/scbrown/quipu/commit/32cf2ae))
- **MCP tools** — 11 tools for agent integration including `quipu_context`,
  `quipu_episode`, `quipu_search_nodes`, `quipu_search_facts`, `quipu_retract`
  ([a53f5c0](https://github.com/scbrown/quipu/commit/a53f5c0),
  [3146322](https://github.com/scbrown/quipu/commit/3146322),
  [3b104fd](https://github.com/scbrown/quipu/commit/3b104fd))

### Infrastructure

- **Graph projection** — petgraph API with centrality, connected components,
  shortest path algorithms
  ([d270132](https://github.com/scbrown/quipu/commit/d270132))
- **Federation** — `GraphProvider` trait for multi-source queries
  ([0842816](https://github.com/scbrown/quipu/commit/0842816))
- **Configuration** — `QuipuConfig` with `.bobbin/config.toml` support
  ([c13baf2](https://github.com/scbrown/quipu/commit/c13baf2))
- **Bobbin integration** — namespace registration, code entity IRI patterns,
  external vector provider delegation, cross-repo import reconciliation,
  unified search results with source tagging
  ([dee600c](https://github.com/scbrown/quipu/commit/dee600c),
  [2fe48a7](https://github.com/scbrown/quipu/commit/2fe48a7),
  [a3b148d](https://github.com/scbrown/quipu/commit/a3b148d),
  [f1be2e0](https://github.com/scbrown/quipu/commit/f1be2e0))

### CI/CD

- GitHub Actions with fmt, clippy, test, and build jobs with caching
  ([c05d534](https://github.com/scbrown/quipu/commit/c05d534))
- release-plz for automated version bumps and changelog generation
  ([01b7808](https://github.com/scbrown/quipu/commit/01b7808))
- Pre-commit hooks for formatting, linting, and file size limits

### Documentation

- Comprehensive mdbook with persona-driven tutorials, SPARQL guide, and recipes
  ([d6504d2](https://github.com/scbrown/quipu/commit/d6504d2))
