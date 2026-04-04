# API Overview

Quipu's public API is organized into four modules.

## Store (`quipu::Store`)

The core fact log store backed by SQLite.

```rust
// Open or create a store
let mut store = Store::open("quipu.db")?;
let mut store = Store::open_in_memory()?;

// Term dictionary
let id = store.intern("http://example.org/alice")?;
let iri = store.resolve(id)?;
let maybe_id = store.lookup("http://example.org/alice")?;

// Write facts
let tx_id = store.transact(&datums, "2026-04-04", Some("actor"), Some("source"))?;

// Read facts
let facts = store.current_facts()?;
let entity = store.entity_facts(entity_id)?;
let history = store.attribute_history(entity_id, attr_id)?;

// Time-travel
let past = store.facts_as_of(&AsOf { tx: Some(5), valid_at: Some("2026-01-01".into()) })?;

// Contradiction detection
let conflicts = store.detect_contradictions(entity_id, attr_id)?;
```

## RDF (`quipu::rdf`)

Parse and serialize standard RDF formats.

```rust
// Ingest from any RDF format
let (tx_id, count) = ingest_rdf(&mut store, reader, RdfFormat::Turtle,
    None, "2026-04-04", None, None)?;

// Export to any RDF format
let bytes = export_rdf(&store, RdfFormat::NTriples)?;
```

## SPARQL (`quipu::sparql`)

Execute SPARQL queries.

```rust
let result = sparql_query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }")?;
// result.variables: ["s", "p", "o"]
// result.rows: Vec<HashMap<String, Value>>
```

## Types (`quipu::types`)

Core data structures used across modules:

- `Value` -- typed value (Ref, Str, Int, Float, Bool, Bytes)
- `Fact` -- a single EAVT fact entry
- `Op` -- Assert (1) or Retract (0)
- `Term` -- dictionary entry (id + IRI)
- `Transaction` -- recorded transaction metadata
