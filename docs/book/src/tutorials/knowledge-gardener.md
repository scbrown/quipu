# The Knowledge Gardener

> "I want to curate and validate our ontology."

You're responsible for the quality of your knowledge graph. Entities drift,
agents write messy data, and over time the graph accumulates orphan nodes,
stale edges, and schema violations. Your job is to define what valid data
looks like and keep the garden tidy.

## Step 1: Define Your Ontology with SHACL

Start by declaring what types of entities exist and what properties they
must have. Create `ontology.shapes.ttl`:

```turtle
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix ont:  <http://aegis.gastown.local/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .

# Every entity must have a label
ont:LabeledShape a sh:NodeShape ;
    sh:targetSubjectsOf rdfs:label ;
    sh:property [
        sh:path rdfs:label ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .

# Hosts must have hostname and cpuCores
ont:HostShape a sh:NodeShape ;
    sh:targetClass ont:Host ;
    sh:property [
        sh:path ont:hostname ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] ;
    sh:property [
        sh:path ont:cpuCores ;
        sh:datatype xsd:integer ;
        sh:minCount 1 ;
    ] ;
    sh:property [
        sh:path ont:memoryMB ;
        sh:datatype xsd:integer ;
    ] .

# Services must reference a valid Host
ont:ServiceShape a sh:NodeShape ;
    sh:targetClass ont:Service ;
    sh:property [
        sh:path ont:runsOn ;
        sh:class ont:Host ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] .

# Dependencies must point to Services
ont:DependencyShape a sh:NodeShape ;
    sh:targetSubjectsOf ont:dependsOn ;
    sh:property [
        sh:path ont:dependsOn ;
        sh:class ont:Service ;
    ] .
```

Load the shapes:

```bash
quipu shapes load --name ontology --file ontology.shapes.ttl --db knowledge.db
```

## Step 2: Validate Existing Data

Run a dry-run validation against your current graph:

```bash
quipu validate --shapes ontology.shapes.ttl --data <(quipu export --db knowledge.db)
```

Or via REST:

```bash
# Export current data
DATA=$(curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"}' | jq -r '.triples')

# Validate
curl -s localhost:3030/validate -X POST \
  -H "Content-Type: application/json" \
  -d "{
    \"shapes\": \"$(cat ontology.shapes.ttl)\",
    \"data\": \"$DATA\"
  }"
```

The response tells you exactly what's wrong:

```json
{
  "conforms": false,
  "violations": 3,
  "warnings": 0,
  "issues": [
    {
      "severity": "Violation",
      "focus_node": "http://aegis.gastown.local/ontology/orphan-svc",
      "path": "http://aegis.gastown.local/ontology/runsOn",
      "component": "MinCountConstraintComponent",
      "message": "Less than 1 values for ont:runsOn"
    }
  ]
}
```

## Step 3: Find Orphan Nodes

Entities with no incoming or outgoing relationships are usually noise:

```sparql
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?entity ?label
WHERE {
  ?entity rdfs:label ?label .
  FILTER NOT EXISTS { ?entity ?anyPred ?anyObj . FILTER(?anyPred != rdfs:label && ?anyPred != <http://www.w3.org/1999/02/22-rdf-syntax-ns#type>) }
  FILTER NOT EXISTS { ?other ?rel ?entity }
}
```

## Step 4: Find Stale Edges

Edges to entities that no longer exist (were retracted):

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>

SELECT ?source ?rel ?target
WHERE {
  ?source ?rel ?target .
  FILTER(isIRI(?target))
  FILTER NOT EXISTS { ?target a ?type }
  FILTER(?rel != <http://www.w3.org/1999/02/22-rdf-syntax-ns#type>)
}
```

This finds triples where the object is an IRI but has no type — likely
a reference to a retracted or never-created entity.

## Step 5: Progressive Schema Tightening

Start with loose shapes and tighten as your ontology stabilizes:

### Phase 1: Required types and labels

```turtle
ont:BasicShape a sh:NodeShape ;
    sh:targetSubjectsOf <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ;
    sh:property [
        sh:path rdfs:label ;
        sh:minCount 1 ;
    ] .
```

### Phase 2: Add datatype constraints

```turtle
ont:HostShape a sh:NodeShape ;
    sh:targetClass ont:Host ;
    sh:property [
        sh:path ont:cpuCores ;
        sh:datatype xsd:integer ;    # Was accepting any literal
        sh:minCount 1 ;
    ] .
```

### Phase 3: Add referential integrity

```turtle
ont:ServiceShape a sh:NodeShape ;
    sh:targetClass ont:Service ;
    sh:property [
        sh:path ont:runsOn ;
        sh:class ont:Host ;          # Must reference a Host entity
        sh:minCount 1 ;
    ] .
```

### Phase 4: Add logical constraints

```turtle
ont:ServiceShape
    sh:property [
        sh:path ont:dependsOn ;
        sh:not [
            sh:equals ont:runsOn ;    # Can't depend on your own host
        ] ;
    ] .
```

## Step 6: Automated Gardening

Set up an agent to periodically validate and report:

```json
{
  "tool": "quipu_validate",
  "input": {
    "shapes": "@prefix sh: ... your shapes ...",
    "data": "@prefix ont: ... current data ..."
  }
}
```

The structured feedback is machine-readable — an agent can:

1. Parse violations
2. Attempt automated fixes (e.g., add missing labels from entity names)
3. File issues for violations it can't fix
4. Report on graph health trends over time

## Step 7: Manage Multiple Shape Sets

Different domains can have different shapes:

```bash
# Infrastructure shapes
quipu shapes load --name infra --file infra.shapes.ttl --db knowledge.db

# Code entity shapes
quipu shapes load --name code --file code.shapes.ttl --db knowledge.db

# List all loaded shapes
quipu shapes list --db knowledge.db

# Remove outdated shapes
quipu shapes remove --name old-shapes --db knowledge.db
```

All loaded shapes are combined during validation — a write must satisfy
all applicable shapes.

## Gardening Queries Cheat Sheet

| Goal | Query Pattern |
|------|---------------|
| Orphan nodes | Entities with no relationships beyond type/label |
| Missing types | `FILTER NOT EXISTS { ?x a ?type }` |
| Duplicate labels | `GROUP BY ?label HAVING(COUNT(?x) > 1)` |
| Stale references | Object IRI with no type assertion |
| Type distribution | `SELECT ?type (COUNT(?x) AS ?n) GROUP BY ?type` |
| Most connected | `SELECT ?x (COUNT(?rel) AS ?n) GROUP BY ?x ORDER BY DESC(?n)` |
| Recent additions | Time-travel with `--tx` to compare states |

## Step 8: Derived Relationships as Garden Health

The reasoner doesn't just derive facts for operators — it's a gardening
tool. Materialised transitive closures reveal structural properties of your
graph that are hard to see from raw facts alone.

### Completeness Checks via Derived Facts

Define a rule that derives "this service is reachable from at least one
host" by closing the `runsOn` chain:

```turtle
@prefix rule: <http://quipu.local/rule#> .
@prefix ex:   <http://aegis.gastown.local/rules/> .

ex:garden a rule:RuleSet ;
    rule:defaultPrefix "http://aegis.gastown.local/ontology/" .

ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runsOn(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?mid), runsOn(?mid, ?host)" .
```

After running the reasoner, query for services that *don't* transitively
reach any host:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?svc ?label
WHERE {
  ?svc a ont:Service .
  ?svc rdfs:label ?label .
  FILTER NOT EXISTS {
    ?svc ont:runsOn ?host .
    ?host a ont:Host .
  }
}
```

These are services with incomplete `runsOn` chains — either they reference
a container that doesn't exist, or the chain breaks somewhere. This is a
data quality signal: the garden needs tending.

### Reactive Gardening

With reactive evaluation enabled, derived facts update as agents write
new data. You can run validation checks after the reasoner fires to catch
problems immediately:

1. Agent writes a new service with `runsOn` pointing to a container
2. Reactive reasoner fires, tries to derive the transitive `runsOn` to a host
3. If the container doesn't have its own `runsOn` edge, no transitive fact
   is derived
4. Your gardening query finds the gap

This turns the reasoner into an early warning system: gaps in the
transitive closure signal incomplete data at the source.

### Monitoring Derived Fact Counts

Track the health of your derived facts over time. After each reasoner run,
the `EvalReport` tells you how many facts were asserted and retracted. A
sudden spike in retractions might mean an agent is writing bad data that
broke a dependency chain. A plateau in assertions might mean your rules
have converged and the ontology is stable.

```bash
quipu reason --rules garden-rules.ttl --db knowledge.db
# reasoner: 3 rules across 2 strata — asserted 0, retracted 0
# ^ All derived facts are up to date — the garden is healthy
```

## What's Next

- [The Rule Builder](rule-builder.md) — write custom rules step by step
- [SHACL Validation](../concepts/shacl-validation.md) — constraint reference
- [SPARQL from Zero](sparql.md) — query patterns
- [Knowledge Ingestion Recipe](../recipes/knowledge-ingestion.md) — bulk loading
