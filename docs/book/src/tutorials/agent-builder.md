# The AI Agent Builder

> "I want my agents to share structured knowledge."

You're building AI agents that observe the world and need to share what they
learn. One agent monitors deployments, another reads incident reports, a third
answers questions. They need a shared knowledge layer — structured, validated,
and queryable.

Quipu gives agents three things:

1. **Episodes** — structured write path for agent observations
2. **MCP tools** — native integration for LLM tool-use
3. **Temporal queries** — "what did the system look like yesterday?"

## Step 1: Agent Writes an Episode

An agent observes a deployment and records it as an episode — a batch of
nodes and edges with provenance:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "deploy-v2.3",
    "source": "deploy-agent",
    "episode_body": "Deployed v2.3 of the API service to production",
    "group_id": "deployments",
    "nodes": [
      {
        "name": "api-v2.3",
        "type": "Deployment",
        "description": "API service version 2.3",
        "properties": {
          "version": "2.3.0",
          "environment": "production",
          "replicas": 3
        }
      },
      {
        "name": "api-service",
        "type": "Service",
        "description": "Core API service"
      }
    ],
    "edges": [
      {"source": "api-v2.3", "target": "api-service", "relation": "deploys"}
    ]
  }'
```

Response:

```json
{"tx_id": 1, "count": 12}
```

The episode created 12 triples in a single transaction: entities with types,
labels, descriptions, properties, relationships, and provenance metadata.

## Step 2: Another Agent Queries It

A Q&A agent needs to answer "what was deployed recently?" It uses the MCP
tool `quipu_query`:

```json
{
  "tool": "quipu_query",
  "input": {
    "query": "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> SELECT ?name ?desc WHERE { ?d a <http://aegis.gastown.local/ontology/Deployment> . ?d rdfs:label ?name . ?d rdfs:comment ?desc }"
  }
}
```

Response:

```json
{
  "variables": ["name", "desc"],
  "rows": [
    {"name": "api-v2.3", "desc": "API service version 2.3"}
  ],
  "count": 1
}
```

## Step 3: Search by Meaning, Not Just Structure

Agents don't always know the exact IRI to query. The `quipu_search_nodes`
tool does natural language entity search:

```json
{
  "tool": "quipu_search_nodes",
  "input": {
    "query": "API deployment",
    "max_results": 5
  }
}
```

Response:

```json
{
  "nodes": [
    {
      "name": "api-v2.3",
      "entity_type": "Deployment",
      "description": "API service version 2.3",
      "score": 0.87
    }
  ],
  "count": 1
}
```

For relationship search, use `quipu_search_facts`:

```json
{
  "tool": "quipu_search_facts",
  "input": {
    "query": "deploys",
    "max_results": 10
  }
}
```

## Step 4: Temporal Queries — What Changed?

An incident-response agent needs to see what the graph looked like before
a problem started. Add `valid_at` to any query:

```json
{
  "tool": "quipu_query",
  "input": {
    "query": "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> SELECT ?name WHERE { ?d a <http://aegis.gastown.local/ontology/Deployment> . ?d rdfs:label ?name }",
    "valid_at": "2026-04-03"
  }
}
```

This returns only deployments that existed as of April 3 — before today's
deploy. The agent can diff the two result sets to see what changed.

## Step 5: Validate Agent Output

Agents make mistakes. SHACL shapes catch them before they pollute the graph.

Define what a valid Deployment looks like:

```turtle
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix ont: <http://aegis.gastown.local/ontology/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ont:DeploymentShape a sh:NodeShape ;
    sh:targetClass ont:Deployment ;
    sh:property [
        sh:path <http://www.w3.org/2000/01/rdf-schema#label> ;
        sh:minCount 1 ;
        sh:datatype xsd:string ;
    ] ;
    sh:property [
        sh:path <http://www.w3.org/2000/01/rdf-schema#comment> ;
        sh:minCount 1 ;
    ] .
```

Load the shapes, then include them in episode ingestion:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "bad-deploy",
    "nodes": [{"name": "oops", "type": "Deployment"}],
    "edges": [],
    "shapes": "@prefix sh: <http://www.w3.org/ns/shacl#> ..."
  }'
```

If the episode data violates the shapes, the write is rejected with
structured feedback the agent can parse and fix.

## MCP Tool Reference

| Tool | Purpose |
|------|---------|
| `quipu_query` | Run SPARQL queries (SELECT, ASK, CONSTRUCT, DESCRIBE) |
| `quipu_knot` | Assert Turtle facts with optional SHACL validation |
| `quipu_cord` | List entities, optionally filtered by type |
| `quipu_unravel` | Time-travel query (by transaction or valid time) |
| `quipu_episode` | Ingest a structured episode |
| `quipu_search` | Vector similarity search |
| `quipu_hybrid_search` | Combined SPARQL filter + vector search |
| `quipu_search_nodes` | Natural language entity search |
| `quipu_search_facts` | Natural language relationship search |
| `quipu_validate` | Dry-run SHACL validation |
| `quipu_shapes` | Load, list, or remove SHACL shapes |
| `quipu_retract` | Retract facts about an entity |

See [MCP Tools Reference](../reference/mcp-tools.md) for full parameter details.

## Patterns for Multi-Agent Systems

### Shared ontology, independent episodes

Each agent writes episodes with its own `source` and `group_id`. The shared
ontology (types, relationships) is defined once via SHACL shapes. Any agent
can query the full graph.

### Agent as knowledge gardener

One agent periodically validates the graph against shapes, finds violations,
and either fixes them or files issues. See [The Knowledge Gardener](knowledge-gardener.md).

### Provenance tracking

Every episode records which agent wrote it. Query provenance:

```sparql
PREFIX prov: <http://www.w3.org/ns/prov#>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?entity ?episode
WHERE {
  ?entity prov:wasGeneratedBy ?ep .
  ?ep rdfs:label ?episode .
}
```

## Step 6: Derived Knowledge with the Reasoner

Agents write raw facts — "traefik runs on webproxy", "webproxy runs on
koror". But other agents need to query *derived* facts — "traefik runs
on koror" (transitively). Instead of making every consuming agent write
property path queries, use the reasoner to materialise derived facts that
every agent can query directly.

### Rules as Shared Infrastructure

Define rules once, and every agent benefits:

```turtle
@prefix rule: <http://quipu.local/rule#> .
@prefix ex:   <http://aegis.gastown.local/rules/> .

ex:agent_rules a rule:RuleSet ;
    rule:defaultPrefix "http://aegis.gastown.local/ontology/" .

# If A depends on B and B depends on C, then A depends on C
ex:depends_on_transitive a rule:Rule ;
    rule:id "depends_on_transitive" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .
```

### Reactive: Derive on Write

With reactive evaluation enabled, derived facts update every time an agent
writes an episode:

```bash
quipu reason --reactive --rules agent-rules.ttl --db knowledge.db
```

Now when the deploy agent writes a new `dependsOn` edge, the transitive
closure updates in the same transaction. The Q&A agent's next query sees
the full dependency chain without any property paths.

### Pre-Flight Checks with Speculate

Before a deploy agent pushes a change, it can ask "what would this break?"
without actually modifying the graph:

```rust
// Hypothetical: remove the old service version
let report = store.speculate(&removal_datums, timestamp, |s| {
    evaluate(s, &ruleset, timestamp)
})?;

if report.retracted > 0 {
    println!("WARNING: removing old version would retract {} derived facts", report.retracted);
    // Agent can decide to proceed or alert a human
}
// Store is unchanged — safe to inspect before committing
```

This is especially powerful in multi-agent systems: one agent proposes a
change, the reasoner evaluates the impact, and a separate agent decides
whether to approve it.

### Provenance for Derived Facts

Derived facts carry source tags like `reasoner:depends_on_transitive`.
Agents can distinguish raw observations from derived knowledge:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>

SELECT ?a ?b
WHERE {
  ?a ont:dependsOn ?b .
  # This returns BOTH direct and transitively-derived dependencies
}
```

The provenance is in the fact metadata — agents that need to distinguish
can filter on the `source` field.

## What's Next

- [The Rule Builder](rule-builder.md) — write custom rules step by step
- [MCP Tools Reference](../reference/mcp-tools.md) — full tool docs
- [Knowledge Ingestion Recipe](../recipes/knowledge-ingestion.md) — batch patterns
- [REST API Reference](../reference/rest-api.md) — all HTTP endpoints
