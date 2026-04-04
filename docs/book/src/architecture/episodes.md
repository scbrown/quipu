# Episode Ingestion

Episodes are the structured write path for agent-extracted knowledge.
When an agent observes something -- a deploy, an incident, a configuration
change -- it packages that knowledge as an episode with typed nodes and edges.

## Anatomy of an Episode

```json
{
  "name": "koror-rebuild-2026-03-29",
  "episode_body": "Koror was rebuilt on kota after disk failure",
  "source": "aegis/crew/ellie",
  "group_id": "aegis-ontology",
  "nodes": [
    {
      "name": "koror",
      "type": "ProxmoxNode",
      "description": "Proxmox host, rebuilt after failure",
      "properties": { "hostname": "koror.lan", "status": "recovered" }
    },
    {
      "name": "kota",
      "type": "ProxmoxNode",
      "description": "Primary Proxmox host"
    }
  ],
  "edges": [
    {
      "source": "koror",
      "target": "kota",
      "relation": "rebuilt_on"
    }
  ]
}
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Episode identifier (becomes IRI) |
| `episode_body` | No | Natural language description |
| `source` | No | Agent or system that produced this |
| `group_id` | No | Knowledge graph partition |
| `nodes` | No | Entities to create |
| `edges` | No | Relationships between entities |
| `shapes` | No | SHACL Turtle for validation gate |

### Node Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Entity identifier |
| `type` | No | rdf:type (e.g., "ProxmoxNode") |
| `description` | No | rdfs:comment |
| `properties` | No | Key-value pairs as typed literals |

### Edge Fields

| Field | Required | Description |
|-------|----------|-------------|
| `source` | Yes | Source entity name |
| `target` | Yes | Target entity name |
| `relation` | Yes | Relationship name (becomes predicate) |

## How Ingestion Works

1. Episode JSON is converted to RDF Turtle
2. If `shapes` is provided, SHACL validation runs first -- rejects on failure
3. Turtle is ingested via the standard RDF pipeline in a single transaction
4. Episode node gets `prov:Activity` type with provenance links
5. Each entity gets `prov:wasGeneratedBy` linking back to the episode

## Provenance Tracking

Every entity created by an episode carries a provenance link:

```sparql
SELECT ?entity WHERE {
  ?entity <http://www.w3.org/ns/prov#wasGeneratedBy>
          <http://aegis.gastown.local/ontology/episode/koror-rebuild-2026-03-29>
}
```

The `episode_provenance()` function returns all entities and their facts
for a given episode name.

## SHACL Validation Gate

Episodes can include a `shapes` field with SHACL constraints. If provided,
the episode data is validated before writing -- invalid episodes are rejected
with structured feedback explaining exactly what failed.

```json
{
  "name": "new-service",
  "shapes": "@prefix sh: ... (SHACL Turtle)",
  "nodes": [{ "name": "myapp", "type": "WebApplication" }]
}
```

## CLI Usage

```bash
# From file
quipu episode deploy.json --db my.db

# From stdin (pipe from another tool)
echo '{"name": "test", "nodes": [...]}' | quipu episode - --db my.db
```

## Batch Ingestion

Multiple episodes can be ingested sequentially. Processing stops on the
first error, so earlier episodes are committed while later ones may not be.

```rust
use quipu::episode::ingest_batch;

let results = ingest_batch(&mut store, &episodes, &timestamps)?;
for (tx_id, count) in results {
    println!("tx={tx_id}, triples={count}");
}
```
