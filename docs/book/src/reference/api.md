# Rust API

Quipu's public API is organized into modules. All types are re-exported
from the crate root via `quipu::*`.

## Store (`quipu::store::Store`)

The core fact log store backed by SQLite.

```rust
use quipu::store::Store;

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
use quipu::rdf::{ingest_rdf, export_rdf};
use oxrdfio::RdfFormat;

// Ingest from any RDF format
let (tx_id, count) = ingest_rdf(&mut store, reader, RdfFormat::Turtle,
    None, "2026-04-04", None, None)?;

// Export to any RDF format
let bytes = export_rdf(&store, RdfFormat::NTriples)?;
```

Supported formats: Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, TriG.

## SPARQL (`quipu::sparql`)

Execute SPARQL queries (SELECT, ASK, CONSTRUCT, DESCRIBE).

```rust
use quipu::sparql;

// SELECT
let result = sparql::query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }")?;
for row in result.rows() {
    println!("{:?}", row.get("name"));
}

// ASK
let result = sparql::query(&store, "ASK { ?s a <http://ex.org/Person> }")?;

// CONSTRUCT
let result = sparql::query(&store, "CONSTRUCT { ?s a <http://ex.org/Known> } WHERE { ?s ?p ?o }")?;
```

## Episode Ingestion (`quipu::episode`)

Structured write path for agent-extracted knowledge.

```rust
use quipu::episode::{Episode, ingest_episode, ingest_batch, episode_provenance};

let episode: Episode = serde_json::from_str(json_str)?;
let (tx_id, count) = ingest_episode(&mut store, &episode, "2026-04-04")?;

// Query provenance
let entities = episode_provenance(&store, "my-episode")?;
```

## Context Pipeline (`quipu::context`)

Unified knowledge context for agent consumption.

```rust
use quipu::context::{ContextPipeline, ContextPipelineConfig};

let pipeline = ContextPipeline::new(&store, ContextPipelineConfig::default());
let ctx = pipeline.query("traefik")?;
println!("{} entities, {} facts", ctx.summary.total_entities, ctx.summary.total_facts);
```

## Graph Projection (`quipu::graph`)

Materialize subgraphs for algorithms.

```rust
use quipu::graph::{project, in_degree, connected_components, shortest_path};

let pg = project(&store, None, None)?;
let ranked = in_degree(&pg);
let components = connected_components(&pg);
let path = shortest_path(&store, &pg, "http://ex.org/a", "http://ex.org/z")?;
```

## Federation (`quipu::provider`)

Virtual graph federation across multiple sources.

```rust
use quipu::provider::{FederatedProvider, LocalProvider};

let mut federation = FederatedProvider::new();
federation.add(Box::new(LocalProvider::new(&store, "local")));
let result = federation.query_all("SELECT ?s ?p ?o WHERE { ?s ?p ?o }")?;
```

## Vector Search (`quipu::store::Store`)

Embedding storage and similarity search.

```rust
// Store embedding
store.embed_entity(entity_id, "description text", &embedding_vec, "2026-04-04")?;

// Search
let matches = store.vector_search(&query_embedding, 10, None)?;
for m in &matches {
    println!("{} (score: {:.3})", m.text, m.score);
}
```

## Reasoner (`quipu::reasoner`)

Stratified Datalog engine that derives facts from rules over the EAVT log.

```rust
use quipu::reasoner::{parse_rules, evaluate, EvalReport, RuleSet};
use quipu::store::Store;

// Parse rules from Turtle
let turtle = std::fs::read_to_string("rules.ttl")?;
let ruleset: RuleSet = parse_rules(&turtle, None)?;

// Evaluate (full re-derivation)
let mut store = Store::open("quipu.db")?;
let report: EvalReport = evaluate(&mut store, &ruleset, "2026-04-04T12:00:00Z")?;
println!("{} asserted, {} retracted", report.asserted, report.retracted);

// Reactive evaluation (auto-derive on every transact)
#[cfg(feature = "reactive-reasoner")]
{
    use quipu::reasoner::reactive::ReactiveReasoner;
    use std::sync::Arc;

    let observer = Arc::new(ReactiveReasoner::new(ruleset));
    store.add_observer(observer.clone());
    // Derived facts now update automatically on every commit.
}

// Counterfactual queries
let result = store.speculate(&hypothetical_datums, timestamp, |s| {
    evaluate(s, &ruleset, timestamp)
})?;
// Store is unchanged — hypothetical was rolled back.
```

See [Reasoner Reference](reasoner.md) for rule syntax, error catalogue,
and supported rule shapes.

## SHACL Validation (`quipu::shacl`)

Schema enforcement at write time (requires `shacl` feature).

```rust
use quipu::shacl::Validator;

let validator = Validator::from_turtle(shapes_turtle)?;
let feedback = validator.validate(data_turtle)?;

if feedback.conforms {
    // Safe to write
} else {
    for issue in &feedback.issues {
        println!("{}: {} at {}", issue.severity, issue.message, issue.focus_node);
    }
}
```

## Types (`quipu::types`)

Core data structures used across modules:

- `Value` -- typed value (Ref, Str, Int, Float, Bool, Bytes)
- `Fact` -- a single EAVT fact entry
- `Op` -- Assert (1) or Retract (0)
- `Term` -- dictionary entry (id + IRI)
- `Transaction` -- recorded transaction metadata
- `VectorMatch` -- vector search result with score
