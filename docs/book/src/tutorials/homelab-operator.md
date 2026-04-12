# The Homelab Operator

> "I want to know what breaks if koror goes down."

You run a homelab — a handful of machines with dozens of services, wired
together with reverse proxies, DNS, and hope. You need a way to track what
runs where, what depends on what, and what the blast radius is when a host
goes down.

This tutorial walks through modeling your infrastructure as a knowledge graph,
querying dependencies with SPARQL, and ingesting changes from monitoring
agents.

## Step 1: Model Your Infrastructure

Create `homelab.ttl` with your hosts and services:

```turtle
@prefix hw:   <http://example.org/homelab/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

# Type hierarchy
hw:WebApp     rdfs:subClassOf hw:Service .
hw:Database   rdfs:subClassOf hw:Service .

# Hosts
hw:koror a hw:Host ;
    rdfs:label   "koror" ;
    hw:hostname  "koror.lan" ;
    hw:cpuCores  "8"^^xsd:integer ;
    hw:memoryMB  "32768"^^xsd:integer .

hw:palau a hw:Host ;
    rdfs:label   "palau" ;
    hw:hostname  "palau.lan" ;
    hw:cpuCores  "4"^^xsd:integer ;
    hw:memoryMB  "16384"^^xsd:integer .

# Services
hw:traefik a hw:WebApp ;
    rdfs:label    "traefik" ;
    hw:runsOn     hw:koror ;
    hw:port       "443"^^xsd:integer ;
    hw:dependsOn  hw:pihole .

hw:pihole a hw:Service ;
    rdfs:label  "pihole" ;
    hw:runsOn   hw:koror ;
    hw:port     "53"^^xsd:integer .

hw:grafana a hw:WebApp ;
    rdfs:label    "grafana" ;
    hw:runsOn     hw:koror ;
    hw:dependsOn  hw:prometheus ;
    hw:dependsOn  hw:postgres .

hw:prometheus a hw:Service ;
    rdfs:label  "prometheus" ;
    hw:runsOn   hw:palau .

hw:postgres a hw:Database ;
    rdfs:label  "postgres" ;
    hw:runsOn   hw:palau ;
    hw:port     "5432"^^xsd:integer .

hw:nginx a hw:WebApp ;
    rdfs:label    "nginx" ;
    hw:runsOn     hw:palau ;
    hw:dependsOn  hw:postgres .
```

Load it:

```bash
quipu knot homelab.ttl --db homelab.db
```

## Step 2: Query — What Runs on Each Host?

```sparql
PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?host ?svc
WHERE {
  ?svc hw:runsOn ?host .
  ?host rdfs:label ?hostLabel .
  ?svc rdfs:label ?svcLabel .
}
ORDER BY ?hostLabel
```

```bash
quipu read "PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?hostLabel ?svcLabel WHERE {
  ?svc hw:runsOn ?host .
  ?host rdfs:label ?hostLabel .
  ?svc rdfs:label ?svcLabel .
} ORDER BY ?hostLabel ?svcLabel" --db homelab.db
```

| ?hostLabel | ?svcLabel |
|------------|-----------|
| koror | grafana |
| koror | pihole |
| koror | traefik |
| palau | nginx |
| palau | postgres |
| palau | prometheus |

## Step 3: Impact Analysis — What Breaks if Koror Goes Down?

This is the killer query. Find all services on koror, then find everything
that depends on them (directly or transitively):

```sparql
PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?affected ?label
WHERE {
  ?affected hw:dependsOn+/hw:runsOn hw:koror .
  ?affected rdfs:label ?label .
}
```

The property path `hw:dependsOn+/hw:runsOn` means: follow one or more
`dependsOn` edges, then one `runsOn` edge, and check if it lands on koror.

| ?affected | ?label |
|-----------|--------|
| `hw:traefik` | traefik |

Traefik depends on pihole, which runs on koror. But traefik itself also
runs on koror — so if koror goes down, you lose traefik, pihole, and grafana
(all locally hosted), plus anything that transitively depends on them.

A more complete impact query — services that run on koror OR depend on
services running on koror:

```sparql
PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT DISTINCT ?svc ?label
WHERE {
  {
    ?svc hw:runsOn hw:koror .
  }
  UNION
  {
    ?svc hw:dependsOn+/hw:runsOn hw:koror .
  }
  ?svc rdfs:label ?label .
}
```

## Step 4: Enforce Structure with SHACL

Prevent malformed data from entering the graph. Create `homelab.shapes.ttl`:

```turtle
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix hw:   <http://example.org/homelab/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .

hw:HostShape a sh:NodeShape ;
    sh:targetClass hw:Host ;
    sh:property [
        sh:path hw:hostname ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] ;
    sh:property [
        sh:path hw:cpuCores ;
        sh:datatype xsd:integer ;
        sh:minCount 1 ;
    ] .

hw:ServiceShape a sh:NodeShape ;
    sh:targetClass hw:Service ;
    sh:property [
        sh:path hw:runsOn ;
        sh:class hw:Host ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] .
```

Load shapes and validate:

```bash
quipu shapes load --name homelab --file homelab.shapes.ttl --db homelab.db
```

Now any new service without a `runsOn` edge is rejected.

## Step 5: Ingest from Monitoring Agents

When an agent discovers new infrastructure, it can push episodes:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "prometheus-discovery-2026-04-04",
    "source": "prometheus-sd",
    "nodes": [
      {"name": "redis", "type": "Service", "description": "Cache layer"},
      {"name": "yap", "type": "Host", "properties": {"hostname": "yap.lan"}}
    ],
    "edges": [
      {"source": "redis", "target": "yap", "relation": "runsOn"},
      {"source": "nginx", "target": "redis", "relation": "dependsOn"}
    ]
  }'
```

The episode creates entities and relationships in a single transaction,
with provenance tracking back to the discovery agent.

## Step 6: Time-Travel After Changes

After adding redis, query what the graph looked like before:

```bash
quipu read "PREFIX hw: <http://example.org/homelab/>
SELECT ?svc WHERE { ?svc a hw:Service }" --db homelab.db --tx 1
```

Transaction 1 only had the original six services. The current state includes
redis.

## Useful Queries for Operators

### Services with no dependencies (leaf nodes)

```sparql
PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?svc ?label
WHERE {
  ?svc a hw:Service .
  ?svc rdfs:label ?label .
  FILTER NOT EXISTS { ?svc hw:dependsOn ?dep }
}
```

### Hosts sorted by service count

```sparql
PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?hostLabel (COUNT(?svc) AS ?n)
WHERE {
  ?svc hw:runsOn ?host .
  ?host rdfs:label ?hostLabel .
}
GROUP BY ?hostLabel
ORDER BY DESC(?n)
```

### Resource utilization summary

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT (SUM(?cores) AS ?totalCores) (SUM(?mem) AS ?totalMB)
       (COUNT(?host) AS ?hostCount)
WHERE {
  ?host a hw:Host .
  ?host hw:cpuCores ?cores .
  ?host hw:memoryMB ?mem .
}
```

## Step 7: Materialise Dependencies with the Reasoner

The property path queries above (`dependsOn+`, `runsOn+`) re-derive
transitive chains every time you run them. For a graph you query often,
the reasoner can materialise those chains once and keep them fresh.

Create `infra-rules.ttl`:

```turtle
@prefix rule: <http://quipu.local/rule#> .
@prefix ex:   <http://example.org/rules/> .

ex:homelab a rule:RuleSet ;
    rule:defaultPrefix "http://example.org/homelab/" .

ex:depends_on_transitive a rule:Rule ;
    rule:id "depends_on_transitive" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .

ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runsOn(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?mid), runsOn(?mid, ?host)" .
```

Run it:

```bash
quipu reason --rules infra-rules.ttl --db homelab.db
```

Now the "what breaks if koror goes down?" query simplifies — no property
paths needed:

```sparql
PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT DISTINCT ?svc ?label
WHERE {
  ?svc hw:runsOn hw:koror .
  ?svc rdfs:label ?label .
}
```

This returns both directly-hosted services AND services that transitively
run on koror via containers, because the reasoner already computed the
transitive closure.

### Stay Fresh with Reactive Evaluation

Enable reactive mode so derived facts update whenever you add or remove
infrastructure:

```bash
quipu reason --reactive --rules infra-rules.ttl --db homelab.db
```

Now when a monitoring agent reports a new container on koror, the
transitive `runsOn` edges update automatically in the same transaction.

### "What If?" Before You Change

Before decommissioning a host, ask the reasoner what would break:

```rust
// Hypothetical: retract all runsOn edges to koror
let report = store.speculate(&koror_retractions, timestamp, |s| {
    evaluate(s, &ruleset, timestamp)
})?;
println!("Decommissioning koror would retract {} derived facts", report.retracted);
```

The store remains unchanged — you see the impact without making the change.
See [The Rule Builder](rule-builder.md) for a complete walkthrough.

## What's Next

- [The Rule Builder](rule-builder.md) — write custom rules step by step
- [Impact Analysis Recipe](../recipes/impact-analysis.md) — more impact patterns
- [SPARQL from Zero](sparql.md) — full SPARQL reference tutorial
- [Knowledge Gardener](knowledge-gardener.md) — maintain ontology quality
