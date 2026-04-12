# Impact Analysis

Recipes for answering "what breaks if X goes down?" using SPARQL property
paths and graph projection.

## Direct Dependencies

Find everything that directly depends on a specific service:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?dependent ?label
WHERE {
  ?dependent ont:dependsOn <http://aegis.gastown.local/ontology/postgres> .
  ?dependent rdfs:label ?label .
}
```

## Transitive Blast Radius

Follow the full dependency chain with property path `+`:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT DISTINCT ?affected ?label
WHERE {
  ?affected ont:dependsOn+ <http://aegis.gastown.local/ontology/postgres> .
  ?affected rdfs:label ?label .
}
```

This traverses one or more `dependsOn` hops — if A depends on B, and B
depends on postgres, then A appears in the results.

## Host-Level Impact

"Everything that breaks if koror goes down" — services running on koror
plus anything that depends on them:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT DISTINCT ?affected ?label ?reason
WHERE {
  {
    ?affected ont:runsOn <http://aegis.gastown.local/ontology/koror> .
    BIND("runs on koror" AS ?reason)
  }
  UNION
  {
    ?affected ont:dependsOn+/ont:runsOn <http://aegis.gastown.local/ontology/koror> .
    BIND("depends on service on koror" AS ?reason)
  }
  ?affected rdfs:label ?label .
}
```

| ?label | ?reason |
|--------|---------|
| traefik | runs on koror |
| pihole | runs on koror |
| grafana | runs on koror |

The `BIND` clause annotates each row with the reason it appears.

## Impact Count by Host

Which hosts are single points of failure?

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?hostLabel (COUNT(DISTINCT ?affected) AS ?blastRadius)
WHERE {
  ?host a ont:Host .
  ?host rdfs:label ?hostLabel .
  {
    ?affected ont:runsOn ?host .
  }
  UNION
  {
    ?affected ont:dependsOn+/ont:runsOn ?host .
  }
}
GROUP BY ?hostLabel
ORDER BY DESC(?blastRadius)
```

## Graph Projection for Visual Analysis

For more complex analysis, project the dependency graph and run algorithms:

```bash
curl -s localhost:3030/project -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "predicate_filter": "http://aegis.gastown.local/ontology/dependsOn"
  }'
```

The projection returns nodes and edges suitable for:

- **In-degree centrality**: Most-depended-upon services
- **Connected components**: Independent failure domains
- **Shortest path**: How are two services connected?

### In-Degree: Most Critical Services

```json
{
  "tool": "quipu_project",
  "input": {
    "predicate_filter": "http://aegis.gastown.local/ontology/dependsOn"
  }
}
```

Services with the highest in-degree are your most critical dependencies.

## Materialised Impact via the Reasoner

The SPARQL property path approach above re-derives transitive chains at
query time. For graphs that change infrequently but are queried often,
you can **materialise** the transitive closure using the reasoner — derived
facts sit in the store alongside raw facts and are queryable without
property paths.

### Set Up Rules

Create `impact-rules.ttl`:

```turtle
@prefix rule:  <http://quipu.local/rule#> .
@prefix ex:    <http://aegis.gastown.local/rules/> .

ex:impact a rule:RuleSet ;
    rule:defaultPrefix "http://aegis.gastown.local/ontology/" .

ex:depends_on_transitive a rule:Rule ;
    rule:id "depends_on_transitive" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .

ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runsOn(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?container), runsOn(?container, ?host)" .
```

### Run the Reasoner

```bash
quipu reason --rules impact-rules.ttl --db homelab.db
```

Now transitive edges are first-class facts. The blast radius query
simplifies to a flat lookup:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT DISTINCT ?affected ?label
WHERE {
  ?affected ont:dependsOn <http://aegis.gastown.local/ontology/postgres> .
  ?affected rdfs:label ?label .
}
```

No `+` operator, no property paths — the reasoner has already closed the
chain. This is faster for repeated queries and simpler for agents to
consume (they don't need to understand property path syntax).

### Keep It Fresh

Enable reactive evaluation so derived facts update automatically when
base facts change:

```bash
quipu reason --reactive --rules impact-rules.ttl --db homelab.db
```

Now every `transact()` that touches `dependsOn` or `runsOn` triggers
re-derivation of the affected transitive edges.

### Property Paths vs Reasoner: When to Use Which

| Approach | Best for |
|----------|----------|
| **Property paths** (`dependsOn+`) | Ad-hoc exploration, one-off queries, small graphs |
| **Reasoner rules** | Repeated queries, agent consumption, cross-predicate joins, counterfactual analysis |

The two approaches are complementary. Property paths work on any graph
without setup. The reasoner requires writing rules up front but pays back
on every subsequent query.

### Counterfactual Impact

The reasoner's `speculate()` API lets you test hypothetical changes
without committing them:

```rust
// "What if postgres goes down?"
let report = store.speculate(&retractions, timestamp, |s| {
    evaluate(s, &ruleset, timestamp)
})?;
println!("{} derived facts would be retracted", report.retracted);
```

See [The Rule Builder tutorial](../tutorials/rule-builder.md) for a
complete worked example.

## Temporal Impact: What Changed?

Compare the dependency graph before and after a change:

```bash
# Snapshot before (transaction 5)
quipu read "PREFIX ont: <http://aegis.gastown.local/ontology/>
SELECT ?svc ?dep WHERE { ?svc ont:dependsOn ?dep }" --db my.db --tx 5

# Current state
quipu read "PREFIX ont: <http://aegis.gastown.local/ontology/>
SELECT ?svc ?dep WHERE { ?svc ont:dependsOn ?dep }" --db my.db
```

Diff the two result sets to see which dependencies were added or removed.
