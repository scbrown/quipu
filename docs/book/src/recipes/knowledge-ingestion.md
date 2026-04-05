# Knowledge Ingestion

Recipes for loading data into Quipu — from single triples to batch imports.

## Turtle Files (Bulk Load)

The fastest way to load structured data:

```bash
quipu knot infrastructure.ttl --db knowledge.db
```

With SHACL validation:

```bash
quipu knot infrastructure.ttl --db knowledge.db --shapes shapes/infra.shapes.ttl
```

With a specific timestamp (for valid-time):

```bash
quipu knot infrastructure.ttl --db knowledge.db --timestamp 2026-04-01
```

Via REST:

```bash
curl -s localhost:3030/knot -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "turtle": "@prefix hw: <http://example.org/homelab/> .\nhw:koror a hw:Host ; hw:hostname \"koror.lan\" .",
    "timestamp": "2026-04-01",
    "actor": "bulk-import"
  }'
```

## Episodes (Agent Observations)

Episodes are the structured write path for agents. Each episode is a
transaction with nodes, edges, and provenance:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "discovery-run-42",
    "source": "prometheus-sd",
    "group_id": "monitoring",
    "episode_body": "Periodic service discovery sweep",
    "nodes": [
      {"name": "redis", "type": "Service", "description": "Cache layer"},
      {"name": "memcached", "type": "Service"}
    ],
    "edges": [
      {"source": "redis", "target": "koror", "relation": "runsOn"},
      {"source": "memcached", "target": "palau", "relation": "runsOn"}
    ]
  }'
```

### Episode Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Episode identifier (becomes rdfs:label) |
| `nodes` | Yes | Array of entities to create |
| `edges` | Yes | Array of relationships |
| `source` | No | Agent/system that produced this |
| `group_id` | No | Logical grouping (e.g., "monitoring") |
| `episode_body` | No | Human-readable description |
| `shapes` | No | Inline SHACL shapes for validation |

### Node Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Entity name (used to generate IRI) |
| `type` | No | RDF type (e.g., "Service", "Host") |
| `description` | No | Human-readable description (rdfs:comment) |
| `properties` | No | Key-value map of additional properties |

### Edge Fields

| Field | Required | Description |
|-------|----------|-------------|
| `source` | Yes | Source entity name |
| `target` | Yes | Target entity name |
| `relation` | Yes | Predicate name (e.g., "runsOn") |

## Graphiti-Compatible Ingestion

For systems already using the Graphiti API format:

```bash
curl -s localhost:3030/episodes/complete -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "flat-episode",
    "episode_body": "Ingested via Graphiti compat endpoint",
    "entity_nodes": [
      {"name": "svc-1", "entity_type": "Service", "summary": "A service"}
    ],
    "episodic_edges": [
      {"source_node_name": "svc-1", "target_node_name": "host-1", "relation_type": "runsOn"}
    ]
  }'
```

## Batch Patterns

### Multiple Turtle files

```bash
for f in data/*.ttl; do
  echo "Loading $f..."
  quipu knot "$f" --db knowledge.db
done
```

### Episodes from a JSON array

```bash
# episodes.json contains an array of episode objects
cat episodes.json | jq -c '.[]' | while read -r episode; do
  curl -s localhost:3030/episode -X POST \
    -H "Content-Type: application/json" \
    -d "$episode"
done
```

### Idempotent ingestion

Episodes with the same entity names update existing entities rather than
creating duplicates. The entity IRI is derived from the name, so repeated
ingestion is safe.

## Validated Ingestion Pipeline

For production use, always validate:

1. **Load shapes first**:

    ```bash
    quipu shapes load --name infra --file infra.shapes.ttl --db knowledge.db
    ```

2. **Dry-run validate**:

    ```bash
    quipu validate --shapes infra.shapes.ttl --data new-data.ttl
    ```

3. **Ingest with shapes**:

    ```bash
    quipu knot new-data.ttl --db knowledge.db --shapes infra.shapes.ttl
    ```

If validation fails, the write is rejected and no facts enter the log.

## MCP Tool Ingestion

For agents using MCP tools:

### Assert triples

```json
{
  "tool": "quipu_knot",
  "input": {
    "turtle": "@prefix hw: <http://example.org/homelab/> .\nhw:koror a hw:Host .",
    "shapes": "@prefix sh: ... optional validation ..."
  }
}
```

### Ingest episode

```json
{
  "tool": "quipu_episode",
  "input": {
    "name": "agent-observation",
    "source": "my-agent",
    "nodes": [{"name": "x", "type": "Thing"}],
    "edges": []
  }
}
```

## Retraction (Removing Facts)

Retract all facts about an entity:

```bash
quipu retract "http://example.org/homelab/old-host" --db knowledge.db
```

Retract a specific predicate:

```bash
quipu retract "http://example.org/homelab/koror" \
  --predicate "http://example.org/homelab/cpuCores" --db knowledge.db
```

Via REST:

```bash
curl -s localhost:3030/retract -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "entity": "http://example.org/homelab/old-host",
    "timestamp": "2026-04-04",
    "actor": "cleanup-agent"
  }'
```

Retractions don't delete data — they close the valid-time window. The
original facts remain in the log for audit and time-travel.
