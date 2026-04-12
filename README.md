<p align="center">
  <img src="assets/logo.svg" width="200" alt="Quipu logo — knotted strings forming a knowledge graph"/>
</p>

<h1 align="center">quipu</h1>

<p align="center">
  <em>🪢 AI-native knowledge graph with strict ontology enforcement</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"/></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-1.85+-orange.svg" alt="Rust 1.85+"/></a>
  <a href="docs/book/src/SUMMARY.md"><img src="https://img.shields.io/badge/docs-mdbook-green.svg" alt="Documentation"/></a>
</p>

> *Cords are entities. Knots are facts. Colors are types. Agents are the readers.* 🧶

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

## 🤔 Why Quipu?

|  | **Jena/Stardog** | **Graphiti/Mem0** | **Quipu** |
|--|:----------------:|:-----------------:|:---------:|
| Strict schema (SHACL)       | ✅ | ❌ | ✅ |
| Bitemporal time-travel      | ❌ | ❌ | ✅ |
| SPARQL 1.1                  | ✅ | ❌ | ✅ |
| Datalog reasoner            | ❌ | ❌ | ✅ |
| Counterfactual queries      | ❌ | ❌ | ✅ |
| Vector similarity search    | ❌ | ✅ | ✅ |
| LanceDB ANN + pushdown      | ❌ | ❌ | ✅ |
| Agent-friendly feedback     | ❌ | ❌ | ✅ |
| Episode provenance          | ❌ | ✅ | ✅ |
| Graph algorithms            | ❌ | ❌ | ✅ |
| Built-in web UI             | ❌ | ❌ | ✅ |
| Embeddable (no server)      | ❌ | ❌ | ✅ |
| SQLite-backed               | ❌ | ❌ | ✅ |
| Rust / zero dependencies    | ❌ | ❌ | ✅ |

Traditional RDF stores demand too much ceremony. AI-native stores have no structure.
Quipu's thesis: **start strict, use agents to bear the cost of strictness.**

## ✨ Features

**🏛️ Knowledge Graph Core**

- **Immutable bitemporal fact log** — every fact has transaction time and valid time. Time-travel to any point. Full audit trail. Contradiction detection.
- **RDF data model** — IRIs, blank nodes, typed literals via oxrdf. Import/export Turtle, N-Triples, JSON-LD, RDF/XML.
- **SPARQL 1.1** — SELECT, ASK, CONSTRUCT, DESCRIBE. BGP, JOIN, UNION, FILTER, OPTIONAL, ORDER BY, GROUP BY, aggregates, HAVING, RDFS subclass inference.
- **SHACL validation** — strict schema enforcement at write time. Structured feedback with severity, focus node, component, path, and message.

**🤖 AI-Native Features**

- **Episode ingestion** — structured write path for agent-extracted knowledge. Typed nodes, edges, and provenance tracking (`prov:wasGeneratedBy`).
- **Hybrid search** — SPARQL filters candidates, vector similarity ranks them. Combine structured queries with semantic meaning in one call. Type constraints are pushed down into the vector index for O(log n) filtered search with LanceDB.
- **Dual vector backends** — default SQLite (brute-force cosine similarity) or optional LanceDB (ANN with predicate pushdown, Arrow columnar storage). Enable with `--features lancedb`.
- **Context pipeline** — unified knowledge context shaped for agent consumption. Text search + link expansion with configurable depth and budget.
- **Agent-friendly feedback** — validation errors include what failed, where, why, and what the valid alternatives are.

**🧠 Reasoning Engine**

- **Datalog over EAVT** — forward-chaining rules in Turtle DSL, evaluated by `datafrog` with semi-naive fixpoint. Stratified negation-as-failure. Derived facts are first-class triples with provenance.
- **Reactive evaluation** — `TransactObserver` re-runs affected rules on every write. Delta-aware: only changed predicates trigger re-evaluation.
- **Counterfactual queries** — `Store::speculate()` forks a hypothetical view via SQLite SAVEPOINT. Answer "what if we remove X?" without mutation.
- **Impact analysis** — BFS walk over entity edges with configurable depth and predicate filters. CLI (`quipu impact`), REST (`POST /impact`), and MCP tool.

**⚙️ Infrastructure**

- **Graph projection** — materialize subgraphs into petgraph for centrality, connected components, shortest path algorithms.
- **Federation** — `GraphProvider` trait for multi-source queries. Query local and remote Quipu instances in a single operation.
- **Four interfaces** — Rust crate (embed), CLI (`quipu`), REST API (`quipu-server`), and built-in web UI with embeddable web components. Plus 11 MCP tools for agent integration.
- **"SQLite energy"** — single process, no server required, inspect with `sqlite3`, back up with `cp`.
- **Automated releases** — release-plz bumps versions from conventional commits, generates changelogs via git-cliff, and creates GitHub releases. CI runs fmt, clippy, tests, and markdown lint on every push.

## 🚀 Quick Start

### 📦 As a Rust Library

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

### 💻 From the Command Line

```bash
git clone https://github.com/scbrown/quipu && cd quipu
cargo build --release

# Load, query, explore
quipu knot data.ttl --db my.db
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10" --db my.db
quipu repl --db my.db
```

### 🌐 REST API & Web UI

```bash
quipu-server --db my.db --bind 0.0.0.0:3030

# Open the interactive graph explorer in your browser
open http://localhost:3030

# Or use the REST API directly
curl localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"}'
```

The built-in web UI provides:

- **Graph Explorer** — force-directed visualization with type-based coloring, entity search, and detail panel
- **SPARQL Workbench** — syntax-highlighted editor with time-travel parameters and tabular/JSON results
- **Episode Timeline** — chronological view of ingested episodes with extracted entities
- **Schema Inspector** — type distribution, SHACL shape browser, and validation runner

Embeddable web components (`<quipu-graph>`, `<quipu-sparql>`, `<quipu-entity>`, `<quipu-timeline>`, `<quipu-schema>`) let you drop Quipu panels into any page:

```html
<script src="http://localhost:3030/quipu-components.js"></script>
<quipu-graph endpoint="http://localhost:3030"></quipu-graph>
```

Semantic Web APIs for interoperability:

- **Spotlight** — entity recognition/disambiguation (`POST /spotlight`)
- **Triple Pattern Fragments** — LDF-compatible pagination (`GET /fragments`)
- **OpenRefine Reconciliation** — data cleaning integration (`POST /reconcile`)
- **Content Negotiation** — `GET /entity/{iri}` returns JSON-LD, Turtle, or HTML based on Accept header

### 🧠 Reasoner

```bash
# Impact analysis — what depends on this entity?
quipu impact http://aegis.local/traefik --db ops.db

# Counterfactual — what breaks if we remove it?
quipu impact http://aegis.local/traefik --remove --db ops.db

# Run Datalog rules over the fact log
quipu reason --rules rules.ttl --db ops.db
```

The reasoner adds forward-chaining inference over the EAVT fact log:

- **Datalog rule engine** — rules written in Turtle DSL, evaluated with semi-naive `datafrog`. Stratified negation-as-failure. Derived facts written back via `Store::transact()` with full provenance.
- **Reactive evaluation** — `TransactObserver` keeps derived facts fresh as base facts change. Delta-aware: only affected rules re-run.
- **Counterfactual queries** — `Store::speculate()` forks a view (SQLite SAVEPOINT) to answer "what if?" without mutation.
- **Impact analysis** — BFS walk over entity edges with configurable hop depth and predicate filters. Available as CLI, REST endpoint (`POST /impact`), and MCP tool.

## 🏗️ Architecture

```text
                    ┌──────────────────────────────┐
                    │    Agent / CLI / Bobbin       │
                    └──────────┬───────────────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
        ┌─────┴─────┐   ┌─────┴─────┐   ┌──────┴──────┐
        │ MCP Tools  │   │ REST API  │   │  Rust API   │
        │ (11 tools) │   │ + Web UI  │   │  (crate)    │
        └─────┬─────┘   └─────┬─────┘   └──────┬──────┘
              └────────────────┼────────────────┘
                               │
     ┌─────────────────────────┼─────────────────────────┐
     │                         │                         │
┌────┴────┐  ┌────┴─────┐  ┌──┴───────┐  ┌──────────────┴──────────────┐
│ SPARQL  │  │  SHACL   │  │ Reasoner │  │   KnowledgeVectorStore      │
│ Engine  │  │ Validator│  │ (Datalog)│  │         (trait)             │
└────┬────┘  └────┬─────┘  └──┬───────┘  └──────┬─────────┬───────────┘
     │             │           │                 │         │
     └─────┬───────┴───────────┘          ┌──────┴───┐ ┌───┴──────┐
           │                              │  SQLite  │ │ LanceDB  │
           │                              │ (default)│ │(optional)│
     ┌─────┴──────────────┐               └──────────┘ └──────────┘
     │   EAVT Fact Log    │
     │   (SQLite)         │
     │                    │
     │  facts + terms +   │
     │  shapes + rules    │
     └────────────────────┘
```

## 🧵 Bobbin Integration

Quipu is designed as a [Bobbin](https://github.com/scbrown/bobbin) subsystem.
Bobbin holds the thread (code context); Quipu ties knots of structured meaning into it.

When running as a Bobbin subsystem, agents get 11 MCP tools. The two most
commonly used for knowledge-aware context:

**`quipu_context`** — unified knowledge discovery. Bobbin merges the result
with its own code search to give agents both code and knowledge in one response.

```json
{
  "tool": "quipu_context",
  "input": { "query": "traefik reverse proxy", "max_entities": 10 }
}
// Returns ranked entities with facts, types, and relevance scores
```

**`quipu_episode`** — save agent-extracted structured knowledge with full
provenance tracking.

```json
{
  "tool": "quipu_episode",
  "input": {
    "name": "deploy-v3",
    "source": "aegis/ellie",
    "nodes": [{"name": "traefik", "type": "WebApplication",
               "properties": {"version": "3.0"}}],
    "edges": [{"source": "traefik", "target": "kota", "relation": "runs_on"}]
  }
}
```

Embeddings are shared: Bobbin's ONNX pipeline (`all-MiniLM-L6-v2`) provides
384-dimensional vectors to both its code search and Quipu's knowledge search,
enabling hybrid queries that span both domains.

## 📖 Documentation

Full documentation is available as an mdbook:

```bash
# Build and serve locally
cargo install mdbook
mdbook serve docs/book
```

See [docs/book/src/SUMMARY.md](docs/book/src/SUMMARY.md) for the table of contents.

## 📋 Feature Matrix

| Feature | Status | Notes |
|---------|:------:|-------|
| **Core** | | |
| EAVT bitemporal fact log | ✅ | Immutable, time-travel queries |
| RDF data model (oxrdf) | ✅ | Turtle, N-Triples, JSON-LD, RDF/XML |
| SQLite storage | ✅ | Single-file, embeddable |
| Retraction with valid-time closure | ✅ | |
| **SPARQL 1.1** | | |
| SELECT / ASK / CONSTRUCT / DESCRIBE | ✅ | |
| BGP, JOIN, UNION, FILTER, OPTIONAL | ✅ | |
| ORDER BY, GROUP BY, HAVING | ✅ | |
| Aggregates (COUNT, SUM, AVG, MIN, MAX) | ✅ | |
| BIND / Extend | ✅ | |
| Property paths | ✅ | |
| Temporal queries (valid_at, as_of_tx) | ✅ | |
| RDFS subclass inference | ✅ | |
| SPARQL UPDATE | 🔜 | Planned |
| Named graphs | 🔜 | Planned |
| Full SPARQL federation (SERVICE) | 🔜 | Planned |
| **Schema & Validation** | | |
| SHACL write-time validation | ✅ | Optional `shacl` feature |
| Persistent shape storage | ✅ | |
| Aegis ontology shapes | ✅ | Infrastructure entities |
| Code entity shapes | ✅ | CodeModule, CodeSymbol, etc. |
| OWL reasoning | 🔜 | Planned |
| **AI-Native** | | |
| Episode ingestion (Graphiti-compatible) | ✅ | Typed nodes, edges, provenance |
| SQLite vector search (cosine) | ✅ | Default backend |
| LanceDB ANN + predicate pushdown | ✅ | Optional `lancedb` feature |
| LanceDB full-text search | ✅ | |
| Hybrid SPARQL + vector search | ✅ | |
| Auto-embed on write | ✅ | Knot/episode hooks |
| ONNX embedding pipeline | ✅ | Shared with Bobbin |
| Context pipeline | ✅ | Text search + link expansion |
| **Reasoner** | | |
| Impact analysis (BFS) | ✅ | CLI, REST, MCP tool |
| Datalog rule engine (datafrog) | ✅ | Turtle DSL, stratified negation |
| Reactive evaluation | ✅ | TransactObserver, delta-aware |
| Counterfactual queries | ✅ | `speculate()` via SQLite SAVEPOINT |
| Incremental truth maintenance | 🔜 | Planned (Phase 5) |
| **Interfaces** | | |
| Rust crate (embed) | ✅ | |
| CLI (`quipu`) | ✅ | knot, read, repl, episode, impact, reason |
| REST API (`quipu-server`) | ✅ | Axum-based |
| Web UI | ✅ | Explorer, workbench, timeline, schema |
| Web components | ✅ | Embeddable `<quipu-*>` elements |
| Semantic Web APIs | ✅ | Spotlight, TPF, OpenRefine reconciliation |
| MCP tools (11) | ✅ | Agent integration |
| Python bindings | 🔜 | Planned |
| **Infrastructure** | | |
| Graph projection (petgraph) | ✅ | Centrality, shortest path, etc. |
| GraphProvider federation trait | ✅ | Multi-source queries |
| Bobbin integration | ✅ | Namespace, IRI patterns, search |
| Automated releases (release-plz) | ✅ | |
| Clustering / replication | 🔜 | Planned |

## 🛠️ Development

```bash
just build               # Build
just test                # Run all tests
just lint                # Clippy with -D warnings
just fmt                 # Format
just check               # Full quality gate (all pre-commit hooks)
just docs check          # Markdown lint + mdbook build
```

Pre-commit hooks enforce formatting, clippy, tests, and file size limits.
CI runs the same checks on every push via GitHub Actions.

## 🤝 Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## 📄 License

[MIT](LICENSE)
