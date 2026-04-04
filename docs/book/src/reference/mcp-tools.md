# MCP Tools

Quipu exposes its API as MCP (Model Context Protocol) tools for agent
integration. These tools are available when Quipu runs as a Bobbin subsystem
or standalone MCP server.

## Tool Reference

### `quipu_query`

Execute a SPARQL SELECT query.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | SPARQL query string |
| `valid_at` | No | ISO-8601 timestamp for time-travel |
| `tx` | No | Transaction ID for time-travel |

### `quipu_knot`

Assert facts from Turtle data, with optional SHACL validation.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `turtle` | Yes | RDF Turtle data |
| `timestamp` | No | Valid-time for the facts |
| `actor` | No | Who is asserting |
| `source` | No | Where the facts came from |
| `shapes` | No | SHACL Turtle for validation gate |

Returns: transaction ID, fact count, and whether validation passed.

### `quipu_cord`

List entities with optional filtering.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `type` | No | Filter by rdf:type IRI |
| `predicate` | No | Filter by relationship |
| `limit` | No | Max results (default: 100) |

### `quipu_unravel`

Time-travel query: view facts at a past state.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `tx` | No | Transaction ID |
| `valid_at` | No | ISO-8601 timestamp |

At least one of `tx` or `valid_at` must be provided.

### `quipu_validate`

Dry-run SHACL validation without writing.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `shapes` | Yes | SHACL shapes as Turtle |
| `data` | Yes | Data to validate as Turtle |

Returns: `conforms` boolean, plus arrays of violations, warnings, and informational issues.

### `quipu_shapes`

Manage persistent SHACL shapes that auto-validate writes.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `action` | Yes | `load`, `list`, or `remove` |
| `name` | For load/remove | Shape set identifier |
| `turtle` | For load | SHACL Turtle content |
| `timestamp` | No | Timestamp for load |

### `quipu_retract`

Retract facts for an entity.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `entity` | Yes | Entity IRI to retract |
| `predicate` | No | Only retract this predicate |
| `timestamp` | No | Retraction timestamp |
| `actor` | No | Who is retracting |

### `quipu_episode`

Ingest structured agent knowledge as an episode.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | Yes | Episode identifier |
| `episode_body` | No | Natural language description |
| `source` | No | Source agent/system |
| `group_id` | No | Knowledge graph group |
| `nodes` | No | Array of `{name, type, description, properties}` |
| `edges` | No | Array of `{source, target, relation}` |

### `quipu_search`

Semantic vector search over entity embeddings.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `embedding` | Yes | Float array (query vector) |
| `limit` | No | Max results (default: 10) |
| `valid_at` | No | Temporal filter |

### `quipu_hybrid_search`

Combined SPARQL filtering + vector ranking.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `embedding` | Yes | Float array (query vector) |
| `sparql` | Yes | SPARQL pre-filter query |
| `limit` | No | Max results (default: 10) |
| `valid_at` | No | Temporal filter |

### `quipu_project`

Graph projection and algorithms.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `algorithm` | No | `stats`, `in_degree`, `components`, or `shortest_path` |
| `type` | No | Type filter for projection |
| `predicate` | No | Predicate filter for projection |
| `from` | For shortest_path | Source entity IRI |
| `to` | For shortest_path | Target entity IRI |
| `limit` | No | Max results for in_degree (default: 20) |

### `quipu_context`

Unified knowledge context pipeline.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Search query string |
| `max_entities` | No | Max entities (default: 20) |
| `expand_links` | No | Follow relationships (default: true) |
