# RDF Data Model

Quipu bridges standard RDF types with the EAVT fact log via the `rdf` module.
This layer handles conversion between oxrdf types and the integer-encoded
term dictionary.

## Type Mapping

RDF terms map to Quipu's `Value` type based on XSD datatype:

| RDF Type | XSD Datatype | Quipu Value |
|----------|-------------|-------------|
| Named node | -- | `Value::Ref(term_id)` |
| Blank node | -- | `Value::Ref(term_id)` (stored as `_:name`) |
| xsd:integer, xsd:long, xsd:int | Integer types | `Value::Int(i64)` |
| xsd:double, xsd:float, xsd:decimal | Float types | `Value::Float(f64)` |
| xsd:boolean | Boolean | `Value::Bool` |
| xsd:string | String | `Value::Str` |
| rdf:langString | Language-tagged | `Value::Str("text@lang")` |

## Ingestion

Parse any RDF format and write to the fact log in a single transaction:

```rust
use quipu::{Store, ingest_rdf};
use oxrdfio::RdfFormat;

let mut store = Store::open_in_memory().unwrap();
let turtle = r#"
@prefix ex: <http://example.org/> .
ex:alice ex:name "Alice" ; ex:age "30"^^xsd:integer .
"#;

let (tx_id, count) = ingest_rdf(
    &mut store,
    turtle.as_bytes(),
    RdfFormat::Turtle,
    None,                          // base IRI
    "2026-04-04T00:00:00Z",       // timestamp
    Some("crew/braino"),           // actor
    Some("entity-file.ttl"),       // source
).unwrap();
// tx_id: transaction ID, count: 2 triples ingested
```

Supported formats: Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, TriG.

## Export

Serialize current facts back to any RDF format:

```rust
use quipu::export_rdf;
use oxrdfio::RdfFormat;

let ntriples = export_rdf(&store, RdfFormat::NTriples).unwrap();
let turtle = export_rdf(&store, RdfFormat::Turtle).unwrap();
```

## Blank Nodes

Blank nodes are stored in the term dictionary with a `_:` prefix.
They round-trip correctly through ingestion and export.

## Language Tags

Language-tagged literals are stored as `"text@lang"` in a `Value::Str`.
On export, the `@` separator is detected and the proper RDF language tag
is restored.
