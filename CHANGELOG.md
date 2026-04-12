# Changelog

All notable changes to this project will be documented in this file.

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
