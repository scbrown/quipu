# Quick Start

## Rust Library

```rust
use quipu::store::Store;
use quipu::rdf::ingest_rdf;
use quipu::sparql;
use oxrdfio::RdfFormat;

fn main() -> quipu::error::Result<()> {
    // Open a persistent store (or use open_in_memory() for testing)
    let mut store = Store::open("my-knowledge.db")?;

    // Ingest Turtle data
    let data = r#"
    @prefix ex: <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

    ex:alice a ex:Person ;
        ex:name "Alice" ;
        ex:age "30"^^xsd:integer ;
        ex:knows ex:bob .

    ex:bob a ex:Person ;
        ex:name "Bob" ;
        ex:age "25"^^xsd:integer .
    "#;

    let (tx_id, count) = ingest_rdf(
        &mut store,
        data.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        Some("demo"),
        Some("quick-start"),
    )?;
    println!("Ingested {count} triples in transaction {tx_id}");

    // Query with SPARQL
    let result = sparql::query(
        &store,
        r#"SELECT ?name ?age WHERE {
            ?s a <http://example.org/Person> .
            ?s <http://example.org/name> ?name .
            ?s <http://example.org/age> ?age .
            FILTER(?age >= 28)
        }"#,
    )?;

    println!("People aged 28+:");
    for row in result.rows() {
        println!("  {:?} age {:?}", row.get("name"), row.get("age"));
    }

    Ok(())
}
```

## CLI

```bash
# Build
cargo build --release

# Load data
quipu knot data.ttl --db my.db

# Query
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10" --db my.db

# Interactive REPL
quipu repl --db my.db
```

## REST API Server

```bash
# Start
quipu-server --db my.db --bind 0.0.0.0:3030

# Health check
curl localhost:3030/health

# Query
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"}'

# Ingest an episode
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "test-episode",
    "nodes": [
      {"name": "alice", "type": "Person", "description": "Test user"}
    ],
    "edges": []
  }'
```

## MCP (Agent Integration)

When running as a Bobbin subsystem, Quipu tools are available to agents:

```json
{
  "tool": "quipu_query",
  "input": {
    "query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"
  }
}
```

See the [MCP Tools Reference](../reference/mcp-tools.md) for all available tools.
