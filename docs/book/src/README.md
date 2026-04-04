# Quipu

AI-native knowledge graph with strict ontology enforcement -- structured
knowledge encoded in knotted strings.

A [quipu](https://en.wikipedia.org/wiki/Quipu) is the Incan knotted-string
recording system. Cords are entities, knots are facts, colors are types, and
trained readers interpret the structure. Quipu brings this philosophy to modern
knowledge graphs: strict structure, enforced by AI agents.

## What Is This?

An embeddable Rust library and server for building knowledge graphs with:

- **Immutable bitemporal fact log** -- time-travel, contradiction detection,
  full audit trail
- **RDF data model** -- IRIs, blank nodes, typed literals via oxrdf
- **SPARQL 1.1 query engine** -- BGP, JOIN, UNION, FILTER, OPTIONAL,
  ORDER BY, GROUP BY, aggregates, HAVING, RDFS subclass inference
- **SHACL validation** -- strict schema enforcement at write time with
  structured agent-friendly feedback
- **Hybrid search** -- SPARQL + vector similarity in a single query
- **Episode ingestion** -- structured write path for agent-extracted knowledge
  (nodes, edges, provenance)
- **Graph projection** -- materialize subgraphs into petgraph for centrality,
  components, shortest-path algorithms
- **Federation** -- virtual graph provider trait for multi-source queries
- **Context pipeline** -- unified knowledge + code context for agent consumption
- **"SQLite energy"** -- single process, no server required, inspect with `sqlite3`

Three ways to use it:

| Interface | Use case |
|-----------|----------|
| **Rust crate** | Embed in your application |
| **CLI** (`quipu`) | Interactive queries and scripting |
| **REST API** (`quipu-server`) | Service deployment |

Designed as a module for [Bobbin](https://github.com/scbrown/bobbin)
(semantic code search engine). Bobbin holds the thread; Quipu ties knots
of structured meaning into it.

## Quick Example

```rust
use quipu::store::Store;
use quipu::rdf::ingest_rdf;
use quipu::sparql;
use oxrdfio::RdfFormat;

// Create an in-memory store
let mut store = Store::open_in_memory().unwrap();

// Ingest some Turtle data
let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:alice a ex:Person ; ex:name "Alice" ; ex:age "30"^^xsd:integer .
ex:bob a ex:Person ; ex:name "Bob" .
"#;
ingest_rdf(&mut store, turtle.as_bytes(), RdfFormat::Turtle,
           None, "2026-04-04", None, None).unwrap();

// Query with SPARQL
let result = sparql::query(&store,
    "SELECT ?name WHERE { ?s <http://example.org/name> ?name }").unwrap();
// Returns: Alice, Bob
```

## CLI Quick Start

```bash
# Load facts from Turtle
quipu knot data.ttl --db my.db

# Query
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10" --db my.db

# List entities of a type
quipu cord --type "http://example.org/Person" --db my.db

# Interactive REPL
quipu repl --db my.db
```

## REST API Quick Start

```bash
# Start the server
quipu-server --db my.db --bind 0.0.0.0:3030

# Query
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"}'

# Ingest an episode
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{"name": "deploy-v2", "nodes": [{"name": "traefik", "type": "WebApp"}],
       "edges": [{"source": "traefik", "target": "kota", "relation": "runs_on"}]}'
```
