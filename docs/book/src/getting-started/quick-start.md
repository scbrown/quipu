# Quick Start

## Create a Store and Add Data

```rust
use quipu::{Store, ingest_rdf, sparql_query};
use oxrdfio::RdfFormat;

fn main() -> quipu::Result<()> {
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
    let result = sparql_query(
        &store,
        r#"SELECT ?name ?age WHERE {
            ?s a <http://example.org/Person> .
            ?s <http://example.org/name> ?name .
            ?s <http://example.org/age> ?age .
            FILTER(?age >= 28)
        }"#,
    )?;

    println!("People aged 28+:");
    for row in &result.rows {
        println!("  {:?} age {:?}", row.get("name"), row.get("age"));
    }

    Ok(())
}
```

## CLI Demo

Run the built-in demo to explore interactively:

```bash
cargo run -- demo
```

This opens an interactive SPARQL prompt where you can load Turtle files
and run queries against the fact log.
