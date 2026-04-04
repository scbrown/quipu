# Graph Projection

Quipu can materialize its fact store into an in-memory directed graph
(via [petgraph](https://crates.io/crates/petgraph)) for running graph
algorithms that aren't expressible in SPARQL.

## How It Works

The `project()` function scans entity-to-entity relationships in the
store and builds a `petgraph::DiGraph`:

- **Nodes** = entities (term IDs)
- **Edges** = relationships where the object is also an entity (`Value::Ref`)
- **Edge weight** = predicate ID

Optional filters narrow the projection:

| Filter | Description |
|--------|-------------|
| `type_filter` | Only include entities of a given `rdf:type` |
| `predicate_filter` | Only include edges with a given predicate |

## Available Algorithms

### Stats

Basic graph metrics: node count and edge count.

### In-Degree Centrality

Rank entities by how many incoming relationships they have.
Useful for finding "hub" entities that many things depend on.

```bash
quipu read "..." # Not expressible in SPARQL -- use the MCP tool instead
```

```json
{
  "tool": "quipu_project",
  "input": {
    "algorithm": "in_degree",
    "type": "http://example.org/Service",
    "limit": 10
  }
}
```

Returns:

```json
{
  "results": [
    { "entity": "http://example.org/traefik", "in_degree": 12 },
    { "entity": "http://example.org/postgres", "in_degree": 8 }
  ]
}
```

### Connected Components

Find clusters of entities that are connected to each other
(strongly connected components via Kosaraju's algorithm).

```json
{
  "tool": "quipu_project",
  "input": { "algorithm": "components" }
}
```

### Shortest Path

Find the shortest path between two entities (A* algorithm).

```json
{
  "tool": "quipu_project",
  "input": {
    "algorithm": "shortest_path",
    "from": "http://example.org/traefik",
    "to": "http://example.org/postgres"
  }
}
```

Returns the path as an ordered list of entity IRIs, or null if unreachable.

## Rust API

```rust
use quipu::graph::{project, in_degree, connected_components, shortest_path};

// Project all entities and relationships
let pg = project(&store, None, None).unwrap();
println!("Nodes: {}, Edges: {}", pg.node_count(), pg.edge_count());

// Find most-connected entities
let ranked = in_degree(&pg);
for (id, degree) in ranked.iter().take(5) {
    println!("{}: {} incoming", id, degree);
}

// Find clusters
let components = connected_components(&pg);
println!("Found {} connected components", components.len());

// Find a path
let path = shortest_path(&store, &pg, "http://ex.org/a", "http://ex.org/z").unwrap();
```
