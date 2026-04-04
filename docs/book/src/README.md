# Quipu

AI-native knowledge graph with strict ontology enforcement — structured
knowledge encoded in knotted strings.

A [quipu](https://en.wikipedia.org/wiki/Quipu) is the Incan knotted-string
recording system. Cords are entities, knots are facts, colors are types, and
trained readers interpret the structure. Quipu brings this philosophy to modern
knowledge graphs: strict structure, enforced by AI agents.

## What Is This?

An embeddable Rust library for building knowledge graphs with:

- **Immutable bitemporal fact log** -- time-travel, contradiction detection,
  full audit trail
- **RDF data model** -- IRIs, blank nodes, typed literals via oxrdf
- **SPARQL 1.1 query engine** -- parsed via spargebra, evaluated over SQLite
- **Agent-friendly validation** -- structured feedback, not just rejections
- **"SQLite energy"** -- single process, no server, inspect with `sqlite3`

Designed as a module for [Bobbin](https://github.com/scbrown/bobbin)
(semantic code search engine). Bobbin holds the thread; Quipu ties knots
of structured meaning into it.

## Status

**Early implementation.** Core storage, RDF model, and SPARQL engine are
functional. Schema enforcement (SHACL) and vector search are next.

## Quick Example

```rust
use quipu::{Store, ingest_rdf, sparql_query};
use oxrdfio::RdfFormat;

// Create an in-memory store
let mut store = Store::open_in_memory().unwrap();

// Ingest some Turtle data
let turtle = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:name "Alice" ; ex:age "30"^^xsd:integer .
ex:bob a ex:Person ; ex:name "Bob" .
"#;
ingest_rdf(&mut store, turtle.as_bytes(), RdfFormat::Turtle,
           None, "2026-04-04", None, None).unwrap();

// Query with SPARQL
let result = sparql_query(&store,
    "SELECT ?name WHERE { ?s <http://example.org/name> ?name }").unwrap();
// Returns: Alice, Bob
```
