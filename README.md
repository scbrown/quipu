<p align="center">
  <strong>QUIPU</strong><br/>
  <em>AI-native knowledge graph with strict ontology enforcement</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"/></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-1.85+-orange.svg" alt="Rust 1.85+"/></a>
  <a href="docs/book/src/SUMMARY.md"><img src="https://img.shields.io/badge/docs-mdbook-green.svg" alt="Documentation"/></a>
</p>

> *Cords are entities. Knots are facts. Colors are types. Agents are the readers.*

A [quipu](https://en.wikipedia.org/wiki/Quipu) is the Incan knotted-string recording system — a pre-Columbian knowledge graph encoded in textile. Trained readers (khipukamayuq) interpreted the structure. Quipu brings this philosophy to modern knowledge graphs: **strict structure, enforced by AI agents**.

## See It In Action

```text
$ quipu knot infrastructure.ttl --shapes aegis-schema.ttl --db ops.db
Ingested 847 triples in transaction 1 (SHACL: 0 violations)

$ quipu read "SELECT ?svc ?host WHERE {
    ?svc a <http://aegis.local/WebApplication> ;
         <http://aegis.local/runsOn> ?host .
  }" --db ops.db

| svc       | host   |
|-----------|--------|
| traefik   | kota   |
| forgejo   | koror  |
| grafana   | kota   |
3 results

$ quipu episode - --db ops.db <<'JSON'
{"name": "koror-rebuild", "source": "aegis/ellie",
 "nodes": [{"name": "koror", "type": "ProxmoxNode",
            "properties": {"status": "recovered"}}],
 "edges": [{"source": "koror", "target": "kota", "relation": "rebuilt_on"}]}
JSON
Ingested 6 triples in transaction 2
```

```text
$ quipu unravel --valid-at "2026-03-15T00:00:00Z" --db ops.db
# See the world as it was two weeks ago

$ quipu stats --db ops.db
Facts: 853 | Entities: 127 | Predicates: 34
```

## Why Quipu?

|  | **Jena/Stardog** | **Graphiti/Mem0** | **Quipu** |
|--|:----------------:|:-----------------:|:---------:|
| Strict schema (SHACL)       | ✅ | ❌ | ✅ |
| Bitemporal time-travel      | ❌ | ❌ | ✅ |
| SPARQL 1.1                  | ✅ | ❌ | ✅ |
| Vector similarity search    | ❌ | ✅ | ✅ |
| Agent-friendly feedback     | ❌ | ❌ | ✅ |
| Episode provenance          | ❌ | ✅ | ✅ |
| Graph algorithms            | ❌ | ❌ | ✅ |
| Embeddable (no server)      | ❌ | ❌ | ✅ |
| SQLite-backed               | ❌ | ❌ | ✅ |
| Rust / zero dependencies    | ❌ | ❌ | ✅ |

Traditional RDF stores demand too much ceremony. AI-native stores have no structure.
Quipu's thesis: **start strict, use agents to bear the cost of strictness.**

## Features

**Knowledge Graph Core**

- **Immutable bitemporal fact log** — every fact has transaction time and valid time. Time-travel to any point. Full audit trail. Contradiction detection.
- **RDF data model** — IRIs, blank nodes, typed literals via oxrdf. Import/export Turtle, N-Triples, JSON-LD, RDF/XML.
- **SPARQL 1.1** — SELECT, ASK, CONSTRUCT, DESCRIBE. BGP, JOIN, UNION, FILTER, OPTIONAL, ORDER BY, GROUP BY, aggregates, HAVING, RDFS subclass inference.
- **SHACL validation** — strict schema enforcement at write time. Structured feedback with severity, focus node, component, path, and message.

**AI-Native Features**

- **Episode ingestion** — structured write path for agent-extracted knowledge. Typed nodes, edges, and provenance tracking (`prov:wasGeneratedBy`).
- **Hybrid search** — SPARQL filters candidates, vector similarity ranks them. Combine structured queries with semantic meaning in one call.
- **Context pipeline** — unified knowledge context shaped for agent consumption. Text search + link expansion with configurable depth and budget.
- **Agent-friendly feedback** — validation errors include what failed, where, why, and what the valid alternatives are.

**Infrastructure**

- **Graph projection** — materialize subgraphs into petgraph for centrality, connected components, shortest path algorithms.
- **Federation** — `GraphProvider` trait for multi-source queries. Query local and remote Quipu instances in a single operation.
- **Three interfaces** — Rust crate (embed), CLI (`quipu`), REST API (`quipu-server`). Plus 11 MCP tools for agent integration.
- **"SQLite energy"** — single process, no server required, inspect with `sqlite3`, back up with `cp`.

## Quick Start

### As a Rust Library

```toml
[dependencies]
quipu = { git = "https://github.com/scbrown/quipu" }
```

```rust
use quipu::store::Store;
use quipu::rdf::ingest_rdf;
use quipu::sparql;
use oxrdfio::RdfFormat;

let mut store = Store::open_in_memory()?;

let turtle = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:name "Alice" ; ex:knows ex:bob .
ex:bob a ex:Person ; ex:name "Bob" .
"#;
ingest_rdf(&mut store, turtle.as_bytes(), RdfFormat::Turtle,
           None, "2026-04-04", None, None)?;

let result = sparql::query(&store,
    "SELECT ?name WHERE { ?s a <http://example.org/Person> . ?s <http://example.org/name> ?name }")?;
```

### From the Command Line

```bash
git clone https://github.com/scbrown/quipu && cd quipu
cargo build --release

# Load, query, explore
quipu knot data.ttl --db my.db
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10" --db my.db
quipu repl --db my.db
```

### REST API

```bash
quipu-server --db my.db --bind 0.0.0.0:3030

curl localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"}'
```

## Architecture

```
                    ┌──────────────────────────────┐
                    │         Agent / CLI           │
                    └──────────┬───────────────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
        ┌─────┴─────┐   ┌─────┴─────┐   ┌──────┴──────┐
        │ MCP Tools  │   │ REST API  │   │  Rust API   │
        │ (11 tools) │   │  (axum)   │   │  (crate)    │
        └─────┬─────┘   └─────┬─────┘   └──────┬──────┘
              └────────────────┼────────────────┘
                               │
        ┌──────────────────────┼──────────────────────┐
        │                      │                      │
  ┌─────┴─────┐         ┌─────┴─────┐         ┌──────┴──────┐
  │  SPARQL   │         │   SHACL   │         │   Vector    │
  │  Engine   │         │ Validator │         │   Search    │
  └─────┬─────┘         └─────┬─────┘         └──────┬──────┘
        └──────────────────────┼──────────────────────┘
                               │
                    ┌──────────┴───────────┐
                    │   EAVT Fact Log      │
                    │   (SQLite)           │
                    │                      │
                    │  facts + terms +     │
                    │  vectors + shapes    │
                    └──────────────────────┘
```

## Bobbin Integration

Quipu is designed as a [Bobbin](https://github.com/scbrown/bobbin) subsystem.
Bobbin holds the thread (code context); Quipu ties knots of structured meaning into it.

When integrated, Bobbin agents get two MCP tools: `knowledge_context` and `knowledge_query`,
blending code search results with knowledge graph facts in a single response.

## Documentation

Full documentation is available as an mdbook:

```bash
# Build and serve locally
cargo install mdbook
mdbook serve docs/book
```

See [docs/book/src/SUMMARY.md](docs/book/src/SUMMARY.md) for the table of contents.

## Development

```bash
cargo build              # Build
cargo test               # Run all tests (109 tests)
cargo clippy             # Lint (pedantic lints enabled)
cargo fmt                # Format
```

Pre-commit hooks enforce formatting, clippy, tests, and file size limits.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

[MIT](LICENSE)
