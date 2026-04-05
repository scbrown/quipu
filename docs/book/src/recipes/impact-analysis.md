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
