# Context Pipeline

The context pipeline blends knowledge graph facts with code context,
producing unified results for agent consumption. It's the integration
surface between Quipu and Bobbin.

## How It Works

When an agent asks for context about a topic:

1. **Text search** -- SPARQL `FILTER(CONTAINS(...))` on entity IRIs and
   literal values to find direct hits
2. **Link expansion** -- follow outgoing and incoming relationships from
   direct hits to discover related entities
3. **Rank and truncate** -- sort by relevance score, trim to budget

The output is a `KnowledgeContext` shaped for Bobbin to merge with its
code search results.

## Output Shape

### KnowledgeContext

```json
{
  "query": "traefik",
  "entities": [ ... ],
  "summary": {
    "total_entities": 4,
    "total_facts": 18,
    "direct_hits": 1,
    "linked_additions": 3
  }
}
```

### KnowledgeEntity

Each entity includes its label, types, relevance, and all its facts:

```json
{
  "iri": "http://example.org/traefik",
  "label": "Traefik",
  "types": ["http://example.org/WebApplication"],
  "relevance": "Direct",
  "score": 1.0,
  "facts": [
    { "predicate": "http://example.org/runsOn", "value": "http://example.org/kota", "value_type": "Entity" },
    { "predicate": "http://example.org/port", "value": "443", "value_type": "Literal" }
  ]
}
```

### Relevance Types

| Relevance | Score | Description |
|-----------|-------|-------------|
| Direct | 1.0 | Found via text search match |
| Linked | 0.5 | Discovered by following relationships from direct hits |
| Semantic | varies | Found via vector similarity search |

## Configuration

| Option | Default | Description |
|--------|---------|-------------|
| `max_entities` | 20 | Maximum entities to return |
| `max_facts_per_entity` | 20 | Maximum facts per entity |
| `expand_links` | true | Follow relationships from direct hits |
| `link_depth` | 1 | How many hops to follow (1 = immediate neighbors) |

## MCP Tool

```json
{
  "tool": "quipu_context",
  "input": {
    "query": "traefik reverse proxy",
    "max_entities": 10,
    "expand_links": true
  }
}
```

## REST API

```bash
curl -s localhost:3030/context -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "traefik", "max_entities": 10}'
```

## Rust API

```rust
use quipu::context::{ContextPipeline, ContextPipelineConfig};

let config = ContextPipelineConfig {
    max_entities: 10,
    expand_links: true,
    ..Default::default()
};

let pipeline = ContextPipeline::new(&store, config);
let ctx = pipeline.query("traefik").unwrap();

for entity in &ctx.entities {
    println!("{} ({:?}): {} facts",
        entity.label.as_deref().unwrap_or(&entity.iri),
        entity.relevance,
        entity.facts.len());
}
```
