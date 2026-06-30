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

### `POST /episode/retract`

Episode-scoped **logical** retraction. Retracts every currently-active fact an
episode's ingest contributed — its activity node, generated entities, the bare
relationship triples (edges), and any reified confidence statements — by closing
their `valid_to` via the bitemporal retract path. Facts are never physically
deleted, so time-travel queries (`/cord`, `/unravel`) still show them.

The retraction unit is the episode's ingest transaction(s), identified by their
`source = "episode:{name}"` tag. Because identical assertions are deduplicated to
a single owning transaction, retracting an episode only removes the facts *that
episode actually wrote* — entities and facts contributed by other episodes (even
about the same shared IRIs) survive untouched. This is the safe way to undo a
specific episode's contributions without SQL surgery on shared entities.

```bash
curl -s localhost:3030/episode/retract -X POST \
  -H "Content-Type: application/json" \
  -d '{"episode": "goldblum-deploy-verify-032"}'
```

Aliases for `episode`: `episode_id`, `name`. Optional: `timestamp`, `actor`.
**Idempotent** — retracting an already-retracted or unknown episode returns
`{"retracted": 0}` and changes nothing. Response includes `tx_id`, `retracted`
(count), and `statements` (the retracted facts).

> **Auth (hq-azs / hq-otm).** Retraction is a write — and a *more* sensitive one
> than assertion, since it removes facts from current views. The endpoint is in
> `http_auth::WRITE_ENDPOINTS`, so it already honours read-only mode and the
> bearer token like every other write. When per-principal scopes (hq-azs) and
> crew identity (hq-otm) land, retraction should be gated to an authorized
> principal, not merely the same token that permits assertion.

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

### `GET|POST /report`

Live graph report: top hubs (god-nodes), surprising cross-community connections,
and auto-suggested questions (see `quipu_report` in the
[MCP tools reference](./mcp-tools.md)). Read-only. `GET` returns the report with
defaults; `POST` accepts an options body (`type`, `predicate`, `hubs`,
`surprises`, `questions`).

```bash
curl -s localhost:3030/report
curl -s localhost:3030/report -X POST \
  -H "Content-Type: application/json" \
  -d '{"hubs": 5, "surprises": 5, "questions": 6}'
```

### `POST /context`

Knowledge context pipeline.

```bash
curl -s localhost:3030/context -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "traefik", "max_entities": 10}'
```

### `POST /unified_search`

Unified knowledge search (text + optional vector); results tagged
`source="knowledge"` with normalized 0–1 scores. Body: `query`, optional
`embedding`, `limit`, `expand_links`, `max_facts_per_entity`.

### `POST /ask`

Run a curated, parameterized named query by name (see `quipu_ask` in the
[MCP tools reference](./mcp-tools.md)). Body: `name` (omit or `"list"` to list
the catalog), optional `params` map. Parameters are validated and escaped by
type. Response: `query`, resolved `sparql`, `columns`, `rows`, `count`.

```bash
curl -s localhost:3030/ask -X POST \
  -d '{"name":"service_deps","params":{"entity":"http://example.org/traefik"}}'
```

### `POST /search_nodes`

Search entities by natural-language query (text matching). Body: `query`,
optional `group_ids`, `max_results`, `entity_type_filter`.

### `POST /search_facts`

Search relationships/edges by natural-language query. Body: `query`, optional
`group_ids`, `max_results`.

### `POST /search/nodes`

Graphiti-compatible node search (mirrors Graphiti's `search_nodes` shape).

### `POST /episodes/complete`

Graphiti-compatible flat episode ingestion. Body: `name`, optional
`episode_body`, `group_id`, `source_description`, `timestamp`.

### `POST /impact`

Impact analysis: walk downstream from an entity, optionally counterfactual.
Body: `entity`, optional `remove`, `hops`, `predicates`, `timestamp`.

### `POST /propose`

Submit a schema-evolution proposal. Body: `kind`, `target`, `diff`, `proposer`,
optional `rationale`, `trigger_ref`, `timestamp`.

### `POST /proposals`

List schema-evolution proposals. Body: optional `status`
(`pending`/`accepted`/`rejected`).

### `POST /proposal/accept`

Accept a pending proposal. Body: `id`, optional `decided_by`, `note`,
`timestamp`.

### `POST /proposal/reject`

Reject a pending proposal. Body: `id`, `note`, optional `decided_by`,
`timestamp`.

### `POST /entity_history`

Return the full fact history (across transactions) for an entity. Body: entity
IRI.

### `GET /transactions`

List transactions in the store.

### `POST /embed_backfill`

Backfill embeddings for entities that lack them.

### `GET /preview/{iri}`

Return a preview rendering of an entity by IRI.
