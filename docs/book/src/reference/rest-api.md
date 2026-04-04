# REST API

The `quipu-server` binary exposes all Quipu operations over HTTP (Axum).

## Starting the Server

```bash
quipu-server --db my.db --bind 0.0.0.0:3030
```

| Flag | Description |
|------|-------------|
| `--db <path>` | Store database path (default: `.bobbin/quipu/quipu.db`) |
| `--bind <addr>` | Bind address (default: `0.0.0.0:3030`) |

## Endpoints

All POST endpoints accept `Content-Type: application/json`.

### `GET /health`

Health check.

```bash
curl localhost:3030/health
```

Response: `{"status": "ok"}`

### `GET /stats`

Store statistics.

```bash
curl localhost:3030/stats
```

Response: `{"facts": 1234, "entities": 56, "predicates": 12}`

### `POST /query`

Execute a SPARQL query.

```bash
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"}'
```

Optional fields: `valid_at` (ISO-8601), `tx` (integer).

### `POST /knot`

Assert facts from Turtle data.

```bash
curl -s localhost:3030/knot -X POST \
  -H "Content-Type: application/json" \
  -d '{"turtle": "@prefix ex: <http://example.org/> . ex:alice a ex:Person ."}'
```

Optional fields: `shapes` (SHACL Turtle), `timestamp`, `actor`, `source`.

Response: `{"tx_id": 1, "count": 2, "conforms": true}`

### `POST /cord`

List entities.

```bash
curl -s localhost:3030/cord -X POST \
  -H "Content-Type: application/json" \
  -d '{"type": "http://example.org/Person", "limit": 50}'
```

### `POST /unravel`

Time-travel query.

```bash
curl -s localhost:3030/unravel -X POST \
  -H "Content-Type: application/json" \
  -d '{"tx": 5}'
```

### `POST /episode`

Ingest an episode.

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "deploy-v2",
    "nodes": [{"name": "myapp", "type": "WebApplication"}],
    "edges": [{"source": "myapp", "target": "kota", "relation": "runs_on"}]
  }'
```

### `POST /validate`

Dry-run SHACL validation.

```bash
curl -s localhost:3030/validate -X POST \
  -H "Content-Type: application/json" \
  -d '{"shapes": "@prefix sh: ...", "data": "@prefix ex: ..."}'
```

### `POST /retract`

Retract facts for an entity.

```bash
curl -s localhost:3030/retract -X POST \
  -H "Content-Type: application/json" \
  -d '{"entity": "http://example.org/old-service"}'
```

Optional: `predicate` (only retract matching), `timestamp`, `actor`.

### `POST /shapes`

Manage persistent SHACL shapes.

```bash
# Load
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "load", "name": "person", "turtle": "@prefix sh: ..."}'

# List
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "list"}'

# Remove
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "remove", "name": "person"}'
```

### `POST /search`

Vector similarity search.

```bash
curl -s localhost:3030/search -X POST \
  -H "Content-Type: application/json" \
  -d '{"embedding": [0.1, 0.2, ...], "limit": 10}'
```

### `POST /hybrid_search`

Combined SPARQL filter + vector ranking.

```bash
curl -s localhost:3030/hybrid_search -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Service> }",
    "embedding": [0.1, 0.2, ...],
    "limit": 5
  }'
```

### `POST /project`

Graph projection and algorithms.

```bash
curl -s localhost:3030/project -X POST \
  -H "Content-Type: application/json" \
  -d '{"algorithm": "in_degree", "limit": 10}'
```

### `POST /context`

Knowledge context pipeline.

```bash
curl -s localhost:3030/context -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "traefik", "max_entities": 10}'
```
