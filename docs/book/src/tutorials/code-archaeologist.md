# The Code Archaeologist

> "I want to understand how this codebase evolved."

You're investigating a codebase — not just what the code does now, but what
decisions shaped it. Which commit caused that outage? What design rationale
lives only in someone's memory? Quipu, paired with Bobbin, links code symbols
to knowledge entities and lets you search across both.

## Step 1: Model Code as Knowledge

Code entities (modules, functions, types) become nodes in the graph:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "code-index-2026-04-04",
    "source": "bobbin-indexer",
    "group_id": "code-symbols",
    "nodes": [
      {
        "name": "sparql-engine",
        "type": "CodeModule",
        "description": "SPARQL 1.1 query evaluation engine",
        "properties": {"path": "src/sparql/mod.rs", "language": "rust"}
      },
      {
        "name": "property-path-eval",
        "type": "CodeSymbol",
        "description": "Evaluates SPARQL property path expressions",
        "properties": {"path": "src/sparql/property_path.rs", "symbol": "eval_path"}
      },
      {
        "name": "store-transact",
        "type": "CodeSymbol",
        "description": "Core transaction write path for the fact log",
        "properties": {"path": "src/store/ops.rs", "symbol": "transact"}
      }
    ],
    "edges": [
      {"source": "property-path-eval", "target": "sparql-engine", "relation": "partOf"},
      {"source": "store-transact", "target": "sparql-engine", "relation": "usedBy"}
    ]
  }'
```

## Step 2: Link Decisions to Code

Record architectural decisions as knowledge entities linked to code:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "adr-eavt-design",
    "source": "human",
    "group_id": "decisions",
    "nodes": [
      {
        "name": "adr-001-eavt",
        "type": "Decision",
        "description": "Chose Datomic-style EAVT fact log over traditional triple store for bitemporal support and append-only safety"
      },
      {
        "name": "adr-002-property-paths",
        "type": "Decision",
        "description": "Implemented custom SPARQL property path evaluator instead of using existing library for tighter integration with temporal model"
      }
    ],
    "edges": [
      {"source": "adr-001-eavt", "target": "store-transact", "relation": "influences"},
      {"source": "adr-002-property-paths", "target": "property-path-eval", "relation": "influences"}
    ]
  }'
```

## Step 3: Hybrid Search — Code AND Decisions

Search for entities by meaning, not just name:

```bash
curl -s localhost:3030/search/nodes -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "query": "how does the transaction write path work",
    "max_results": 5
  }'
```

This returns both the `store-transact` code symbol and the `adr-001-eavt`
decision that influenced it — answering "what" and "why" in one search.

### SPARQL for precise queries

```sparql
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX ont: <http://aegis.gastown.local/ontology/>

SELECT ?decision ?description
WHERE {
  ?decision a ont:Decision .
  ?decision ont:influences ?code .
  ?code rdfs:label "store-transact" .
  ?decision rdfs:comment ?description .
}
```

## Step 4: Incident Correlation

When something breaks, link the incident to code and time:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "incident-2026-04-02-query-timeout",
    "source": "incident-agent",
    "group_id": "incidents",
    "nodes": [
      {
        "name": "inc-2026-04-02",
        "type": "Incident",
        "description": "SPARQL queries timing out after property path merge",
        "properties": {
          "severity": "P2",
          "started": "2026-04-02T14:30:00Z",
          "resolved": "2026-04-02T16:00:00Z",
          "commit": "abc123"
        }
      }
    ],
    "edges": [
      {"source": "inc-2026-04-02", "target": "property-path-eval", "relation": "causedBy"},
      {"source": "inc-2026-04-02", "target": "sparql-engine", "relation": "affected"}
    ]
  }'
```

Now you can query: "What code has caused incidents?"

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?code ?codePath ?incident ?description
WHERE {
  ?incident a ont:Incident .
  ?incident ont:causedBy ?code .
  ?incident rdfs:comment ?description .
  ?code rdfs:label ?codePath .
}
```

## Step 5: Time-Travel for Context

Combine temporal queries with code knowledge:

"What did we know about the SPARQL engine before the incident?"

```bash
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "query": "PREFIX ont: <http://aegis.gastown.local/ontology/> PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> SELECT ?entity ?desc WHERE { ?entity ont:partOf <http://aegis.gastown.local/ontology/sparql-engine> . ?entity rdfs:comment ?desc }",
    "valid_at": "2026-04-01"
  }'
```

## Step 6: Graph Projection — Dependency Analysis

Visualize module dependencies using graph projection:

```bash
curl -s localhost:3030/project -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "type_filter": "http://aegis.gastown.local/ontology/CodeModule"
  }'
```

This returns a petgraph-compatible adjacency structure. Use it for:

- **Centrality**: Which modules are most depended upon?
- **Components**: Which modules form independent clusters?
- **Shortest path**: How are two modules connected?

```bash
curl -s localhost:3030/project -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "predicate_filter": "http://aegis.gastown.local/ontology/usedBy"
  }'
```

## Patterns for Code Archaeology

### Link commits to knowledge changes

When a significant commit lands, create an episode with the commit hash,
affected code symbols, and a description of intent. Over time, the graph
becomes a searchable record of *why* the code looks the way it does.

### Cross-reference documentation

Store doc sections as entities linked to the code they describe. When code
changes, search for linked docs that may need updating.

### Build a decision log

ADRs (Architecture Decision Records) stored as entities with `influences`
edges to code. When someone asks "why did we do it this way?", the graph
has the answer — queryable by code symbol, by date, or by topic.

## Step 7: Derive Influence Chains with the Reasoner

You've recorded `influences` edges between decisions and code symbols. But
influence is transitive — if decision A influences module M, and module M
is used by module N, then decision A indirectly influences module N. The
reasoner can materialise these chains.

Create `archaeology-rules.ttl`:

```turtle
@prefix rule: <http://quipu.local/rule#> .
@prefix ex:   <http://aegis.gastown.local/rules/> .

ex:archaeology a rule:RuleSet ;
    rule:defaultPrefix "http://aegis.gastown.local/ontology/" .

# Transitive influence: if A influences B and B is usedBy C, A influences C
ex:influence_through_usage a rule:Rule ;
    rule:id "influence_through_usage" ;
    rule:head "influences(?decision, ?downstream)" ;
    rule:body "influences(?decision, ?code), usedBy(?code, ?downstream)" .

# Transitive partOf: if A is partOf B and B is partOf C, A is partOf C
ex:part_of_transitive a rule:Rule ;
    rule:id "part_of_transitive" ;
    rule:head "partOf(?a, ?c)" ;
    rule:body "partOf(?a, ?b), partOf(?b, ?c)" .
```

Run it:

```bash
quipu reason --rules archaeology-rules.ttl --db knowledge.db
```

Now you can answer "which decisions influenced this module?" across any
depth of the dependency graph, with a flat query:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?decision ?description
WHERE {
  ?decision ont:influences <http://aegis.gastown.local/ontology/sparql-engine> .
  ?decision a ont:Decision .
  ?decision rdfs:comment ?description .
}
```

### Blast Radius for Code Changes

Before refactoring a module, use `speculate()` to see what derived
relationships would break:

```rust
// Hypothetical: remove the usedBy edge from store-transact to sparql-engine
let report = store.speculate(&retractions, timestamp, |s| {
    evaluate(s, &ruleset, timestamp)
})?;
println!("Decoupling these modules would affect {} derived influence chains",
    report.retracted);
```

This tells you which architectural decisions and incident correlations
would lose their path to downstream code — before you make the change.

## What's Next

- [The Rule Builder](rule-builder.md) — write custom rules step by step
- [Incident Correlation Recipe](../recipes/incident-correlation.md) — more patterns
- [SPARQL from Zero](sparql.md) — learn query patterns
- [Graph Projection](../reference/api.md) — API details
