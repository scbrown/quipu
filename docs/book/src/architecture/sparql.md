# SPARQL Engine

Quipu includes a custom SPARQL evaluator that compiles queries directly
against the SQLite fact log. No separate triple store or graph database
is needed.

## How It Works

1. **Parse**: SPARQL string -> AST via [spargebra](https://crates.io/crates/spargebra)
2. **Evaluate**: Walk the AST, executing each graph pattern against SQLite
3. **Return**: Variable bindings as `HashMap<String, Value>` rows

```rust
use quipu::store::Store;
use quipu::sparql;

let result = sparql::query(&store,
    "SELECT ?name WHERE { ?s <http://example.org/name> ?name }"
).unwrap();

for row in result.rows() {
    println!("{:?}", row.get("name"));
}
```

## Query Forms

| Form | Description | Example |
|------|-------------|---------|
| SELECT | Return variable bindings | `SELECT ?s ?p ?o WHERE { ... }` |
| ASK | Boolean existence check | `ASK { ?s a ex:Person }` |
| CONSTRUCT | Build new triples | `CONSTRUCT { ?s a ex:Result } WHERE { ... }` |
| DESCRIBE | Return all facts about an entity | `DESCRIBE <http://example.org/alice>` |

## Supported Features

### Graph Patterns

| Pattern | Status | Example |
|---------|--------|---------|
| Basic Graph Pattern (BGP) | Supported | `?s ?p ?o` |
| JOIN | Supported | Multiple BGP patterns |
| UNION | Supported | `{ ... } UNION { ... }` |
| FILTER | Supported | `FILTER(?age > 30)` |
| OPTIONAL (LeftJoin) | Supported | `OPTIONAL { ?s ex:email ?e }` |
| PROJECT | Supported | `SELECT ?name` |
| DISTINCT / REDUCED | Supported | `SELECT DISTINCT ?type` |
| LIMIT / OFFSET | Supported | `LIMIT 10 OFFSET 5` |
| ORDER BY | Supported | `ORDER BY DESC(?age)` |
| GROUP BY | Supported | `GROUP BY ?type` |
| HAVING | Supported | `HAVING(COUNT(?s) > 2)` |
| EXTEND (BIND) | Supported | Computed variables |
| Property paths | Not yet | Planned |

### Aggregates

| Function | Example |
|----------|---------|
| COUNT | `SELECT (COUNT(?s) AS ?n) WHERE { ... }` |
| SUM | `SELECT (SUM(?age) AS ?total) ...` |
| AVG | `SELECT (AVG(?age) AS ?mean) ...` |
| MIN / MAX | `SELECT (MIN(?age) AS ?youngest) ...` |

### FILTER Expressions

| Expression | Example |
|-----------|---------|
| Equality | `?name = "Alice"` |
| Comparison | `?age > 30`, `?age <= 50` |
| AND / OR / NOT | `?age > 20 && ?age < 40` |
| BOUND | `BOUND(?name)` |
| Regex | `regex(?name, "Ali")` |
| CONTAINS | `CONTAINS(STR(?s), "traefik")` |
| LCASE / STR | `LCASE(STR(?name))` |
| isIRI | `FILTER(isIRI(?o))` |

### RDFS Inference

Quipu supports RDFS subclass inference for `rdf:type` queries. If you define:

```turtle
ex:Engineer rdfs:subClassOf ex:Person .
ex:alice a ex:Engineer .
```

Then `SELECT ?s WHERE { ?s a ex:Person }` will return `ex:alice` through
transitive subclass reasoning.

## Temporal Awareness

The SPARQL engine automatically filters to current state
(`op = 1 AND valid_to IS NULL`). Time-travel is supported via the
`unravel` command:

```bash
# See the world as it was at a specific transaction
quipu unravel --tx 5 --db my.db

# See the world as it was at a specific time
quipu unravel --valid-at "2026-03-15T00:00:00Z" --db my.db
```

Via the MCP tool:

```json
{
  "tool": "quipu_query",
  "input": {
    "query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",
    "valid_at": "2026-03-15T00:00:00Z"
  }
}
```
