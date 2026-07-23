# RDF Data Model

> **Implementation status (2026-07-23, kelly):** ✅ **Implemented** (a stale Language-Tags section was corrected in this commit). The capability is shipped: `ingest_rdf` / `export_rdf` for all 6 formats
> (Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, TriG) via oxrdfio, blank-node
> round-trip, and XSD→`Value` mapping (`src/rdf.rs`, `src/types.rs`). **Drift (corrected in this commit):** the
> "Language Tags" section (and its type-mapping row) is OBSOLETE — it claims
> `rdf:langString` is stored as `Value::Str("text@lang")` and re-split on `@`, but the
> code uses a dedicated `Value::Lang { lexical, lang }` variant (`src/types.rs`,
> tag 6); reconstructing a lang tag by splitting a `Str` on `@` was a fixed bug. The
> Language Tags section + type-map row are now corrected to match.

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
| rdf:langString | Language-tagged | `Value::Lang { lexical, lang }` |
| other `^^<datatype>` literals | Typed literal | `Value::Typed { lexical, datatype }` |

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

Language-tagged literals (`rdf:langString`) are stored as a dedicated
`Value::Lang { lexical, lang }` variant — the lexical form (`"hello"`) and the
language tag (`"en"`) held in **separate fields**, never concatenated. On export
the tag is reattached from the `lang` field. (A previous design concatenated them
as `"text@lang"` in a `Value::Str` and re-split on `@`; that is a fixed bug — a
lexical form may legitimately contain `@`, so the tag lives in its own field.)
Datatyped literals with a non-standard `^^<datatype>` use the parallel
`Value::Typed { lexical, datatype }`, preserving the lexical form byte-for-byte.
