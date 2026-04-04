# SPARQL Engine

Quipu includes a custom SPARQL evaluator that compiles queries directly
against the SQLite fact log. No separate triple store or graph database
is needed.

## How It Works

1. **Parse**: SPARQL string -> AST via [spargebra](https://crates.io/crates/spargebra)
2. **Evaluate**: Walk the AST, executing each graph pattern against SQLite
3. **Return**: Variable bindings as `HashMap<String, Value>` rows

```rust
use quipu::{Store, sparql_query};

let result = sparql_query(&store,
    "SELECT ?name WHERE { ?s <http://example.org/name> ?name }"
).unwrap();

for row in &result.rows {
    println!("{:?}", row.get("name"));
}
```

## Supported Features

### Graph Patterns

| Pattern | Status | Example |
|---------|--------|---------|
| Basic Graph Pattern (BGP) | Supported | `?s ?p ?o` |
| JOIN | Supported | Multiple BGP patterns |
| UNION | Supported | `{ ... } UNION { ... }` |
| FILTER | Supported | `FILTER(?age > 30)` |
| PROJECT | Supported | `SELECT ?name` |
| DISTINCT | Supported | `SELECT DISTINCT ?type` |
| LIMIT/OFFSET | Supported | `LIMIT 10 OFFSET 5` |
| OPTIONAL | Not yet | Planned |
| Property paths | Not yet | Planned |
| Aggregates | Not yet | Planned |
| ORDER BY | Not yet | Planned |

### FILTER Expressions

| Expression | Example |
|-----------|---------|
| Equality | `?name = "Alice"` |
| Comparison | `?age > 30`, `?age <= 50` |
| AND/OR/NOT | `?age > 20 && ?age < 40` |
| BOUND | `BOUND(?name)` |
| Regex | `regex(?name, "Ali")` |

## Temporal Awareness

The SPARQL engine automatically filters to current state
(`op = 1 AND valid_to IS NULL`). Time-travel queries via SPARQL
extension functions are planned.

## Integration with Bobbin

The SPARQL engine will be exposed through Bobbin's MCP server, allowing
agents to query the knowledge graph alongside code context. The planned
tool surface:

- `sparql_query`: Execute a SPARQL SELECT and return bindings
- `knot`: Assert a fact (with validation)
- `cord`: List entities matching a pattern
- `unravel`: Time-travel query
