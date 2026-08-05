# REST API

The `quipu-server` binary exposes all Quipu operations over HTTP (Axum).

## Starting the Server

```bash
quipu-server --db my.db --bind 0.0.0.0:3030
```

| Flag | Description |
|------|-------------|
| `--db <path>` | Store database path (default: `.bobbin/quipu/quipu.db`) |
| `--bind <addr>` | Bind address (default: `127.0.0.1:3030`) |

## Authentication

**Reads are open; writes need a bearer token.** When the server is started with an
auth token configured, every *write* endpoint requires:

```
Authorization: Bearer <token>
```

Reads — `/query`, `/search`, entity lookups, `/health`, `/version` — need no
credential and answer normally.

**The authoritative list is `http_auth::WRITE_ENDPOINTS` in `src/http_auth.rs`, not
this page.** It is enforced: `write_endpoints_cover_every_route` fails the build if
any registered route is unclassified, so the code cannot drift from itself — but
this page can drift from the code, so treat it as a summary and the constant as the
answer.

Two entries surprise people, and both are deliberate:

| Endpoint | Why it is a WRITE |
|---|---|
| `/project` | Looks read-only — `stats`, `pagerank`, `ppr`, `components` only read — but `louvain` with `persist: true` **writes** `quipu:memberOfCommunity` and supersedes any prior derivation. The route is gated as a whole. |
| `/shapes` | Gated even to *list*. Loading a shape set persists it, and a listed-but-unloaded set validates nothing while still reporting success. |

A refusal is never silent. Both refusals return a JSON body naming
the cause, so `curl -s` cannot render an auth failure as an empty result:

```json
{"endpoint":"/project","reason":"missing_or_invalid_bearer_token","error":"unauthorized: ..."}
{"endpoint":"/knot","reason":"server_is_read_only","error":"read-only mode: ..."}
```

`reason` is the stable field to branch on; `error` is prose and may be reworded.

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

Optional: `predicate` (only retract matching), `timestamp`, `actor`, and `value`
(retract only the one matching triple).

**The `value` shape matters.** An object that is an IRI reference — the target of
an edge such as `reports_to` or `rdf:type` — must be given as a tagged object, not
a bare string:

```bash
# retract exactly  <svc> reports_to <boss>
curl -s localhost:3030/retract -X POST -H "Content-Type: application/json" \
  -d '{"entity": "http://example.org/svc",
       "predicate": "http://example.org/reports_to",
       "value": {"iri": "http://example.org/boss"}}'
```

A **bare string** (`"value": "http://example.org/boss"`) is matched as a string
*literal*, which can never equal a stored IRI reference. Rather than silently
report `{"retracted": 0}` — indistinguishable from "the triple was already gone" —
the endpoint now **returns a 400 error** naming the `{"iri": ...}` form whenever a
bare string cannot match: either the predicate's stored objects are IRIs, or the
string itself parses as an IRI (has a `scheme://`). A correctly shaped `{"iri":
...}` (or a genuine string literal) for a triple that does not exist is still a
quiet, idempotent `{"retracted": 0}`.

### `POST /episode/retract`

Episode-scoped **logical** retraction. Retracts the facts an episode's ingest
contributed — its activity node, generated entities, the bare relationship
triples (edges), and any reified confidence statements — by closing their
`valid_to` via the bitemporal retract path. Facts are never physically deleted,
so time-travel queries (`/cord`, `/unravel`) still show them.

**Identity of surviving nodes is preserved by default.** Identity triples
(`rdfs:label`, `rdf:type`) are ordinary facts, so a naive scope retraction would
strip them from any node this episode *named* even when edges from other episodes
keep that node alive — leaving a "ghost": a node that answers predicate queries
but is invisible to every label scan and type query. The `on_orphan` parameter
decides that contract:

| `on_orphan` (alias `orphan_policy`) | Behaviour |
|-------------------------------------|-----------|
| `preserve` (**default**) | Keep `rdfs:label` / `rdf:type` alive for nodes that retain surviving references. So the default does **not** retract "every currently-active fact" — it spares the identity of nodes that would otherwise be orphaned. |
| `refuse` | If the retraction would orphan any node's identity, reject the whole operation (400) and change nothing. The safe mode when you do not want to strand entities. |
| `allow` | Legacy behaviour: retract every currently-active fact the episode wrote, orphaned identity included. |

Regardless of policy the response reports `identity_orphans` (a count) and names
the affected nodes, so a caller can tell a cleanup from a mutilation.

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

Aliases for `episode`: `episode_id`, `name`. Optional: `timestamp`, `actor`,
`on_orphan` (`preserve` | `refuse` | `allow`, default `preserve` — see the table
above). **Idempotent** — retracting an already-retracted or unknown episode
returns `{"retracted": 0}` and changes nothing.

Response fields: `tx_id`, `retracted` (count), `episode`, `statements` (the
retracted facts), and the identity accounting — `on_orphan` (the policy applied),
`identity_preserved` (count) with `identity_preserved_statements`, and
`identity_orphans` (count) with `identity_orphan_entities` (`entity`,
`lost_label`, `lost_type`).

> **Auth (hq-azs / hq-otm).** Retraction is a write — and a *more* sensitive one
> than assertion, since it removes facts from current views. The endpoint is in
> `http_auth::WRITE_ENDPOINTS`, so it already honours read-only mode and the
> bearer token like every other write. When per-principal scopes (hq-azs) and
> crew identity (hq-otm) land, retraction should be gated to an authorized
> principal, not merely the same token that permits assertion.

### `POST /resolve`

Ask what entity resolution *would* say about a name, without writing anything.

Returns the same candidate list the ingest path computes, so "is this a duplicate
of something we already have?" can be answered **before** minting the entity.

```bash
curl -s localhost:3030/resolve -X POST \
  -H "Content-Type: application/json" \
  -d '{"name": "example-service", "properties": {"type": "DatabaseService"}}'

# {"candidates":[{"iri":"http://example.org/ontology/example-service",
#                "score":0.9,"matched_on":"canonical_name:jaro_winkler:0.90"}],
#  "count":1,"has_matches":true}
```

`name` is required. `properties` (object), `top_k` and `threshold` are optional;
`top_k` and `threshold` default to `[quipu.resolution]` config, so this route and
the ingest path agree by construction rather than by convention.

Both matchers run: Jaro-Winkler over `rdfs:label` (`matched_on:
canonical_name:jaro_winkler:<score>`) **and** vector similarity when an embedding
provider is configured (`matched_on: embedding:<score>`). The embedding half is
the reason a client-side name check is not a substitute.

Notes:

- **It does not write — and that is guaranteed by a test, not by the type.** The
  handler takes a `&Store`, but do not read that as a read-only capability:
  `Store` writes through `&self` methods via interior mutability, so a `&Store`
  handler *can* commit. Several routes registered the same way do write, which is
  why `/overlay/create` sits in `WRITE_ENDPOINTS` despite its signature. What
  actually holds this route read-only is the explicit assertion in
  `tool_resolve_entity_is_a_genuine_read_commits_nothing`.
- **It is not a write endpoint** (absent from `http_auth::WRITE_ENDPOINTS`), so it
  needs no bearer token and answers normally on a `read_only = true` server —
  where `POST /episode` returns 403.
- **It does not require `[quipu.resolution].enabled`.** Resolution being off
  disables the *ingest-time* hints; this route still answers.
- Not to be confused with `POST /reconcile` — the W3C Reconciliation API, which
  has its own substring scoring on a 0-100 scale and does not consult embeddings.
  (It is routed but undocumented here, as are `/spotlight` and `/fragments`.)

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

Vector similarity search. Body: `embedding` (or `query`), optional `limit`,
`valid_at`, and best-effort scoping by `group_ids` / `entity_type`.

```bash
curl -s localhost:3030/search -X POST \
  -H "Content-Type: application/json" \
  -d '{"embedding": [0.1, 0.2, ...], "limit": 10}'
```

`group_ids` is a best-effort **provenance** filter, not an isolation boundary:
it narrows to entities whose facts trace (via `prov:wasGeneratedBy → episode →
groupId`) to a listed group, and it **drops** ungrouped `/knot` facts (they have
no episode to trace). `entity_type` restricts to an rdf:type IRI. See
[group-isolation](../../design/group-isolation.md).

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

### `POST /graph`

Render-ready node-link projection — the single payload the web UI draws from.

```bash
curl -s localhost:3030/graph -X POST \
  -H "Content-Type: application/json" \
  -d '{"limit": 250}'
```

Body: optional `limit` (nodes, ranked by degree; default 250, max 2000),
`type` (restrict to one `rdf:type` IRI), `include_episodes` (default `false`).

```json
{
  "nodes": [{"iri": "…", "label": "kota", "type": "…/ProxmoxNode", "deg": 8}],
  "edges": [[0, 10, "managed_by"]],
  "types": [{"iri": "…", "label": "SystemdService", "count": 15}],
  "truncated": {"shown": 250, "of": 1180},
  "stats": {"nodes": 250, "edges": 612}
}
```

`edges` address nodes by **index** into `nodes`, not by IRI — an IRI averages
~45 bytes and would otherwise repeat at both ends of every edge. `prov:Activity`
episodes and `rdf`/`rdfs`/`prov` scaffolding predicates are excluded by default
so the domain graph is not buried in provenance. `truncated` always states what
was dropped rather than silently capping.

### `POST /context`

Knowledge context pipeline.

```bash
curl -s localhost:3030/context -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "traefik", "max_entities": 10}'
```

The `summary` carries an `embeddings` block reporting whether semantic
retrieval was possible at all, so an empty `entities` list is not ambiguous:

```json
"embeddings": { "configured": true, "embedded_entities": 2579 }
```

`configured: false` means no embedding provider is attached;
`embedded_entities: 0` with `configured: true` means the store was never
embedded (`quipu knot` does not embed — run a backfill). See
[Embeddings and Semantic Search](../concepts/embeddings.md).

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

Backfill embeddings for entities that lack them. Returns
`{"status": "error", ...}` when no embedding provider is configured; the
`--embed-backfill` startup flag instead exits non-zero rather than serving
without the capability it was asked for.

### `GET /preview/{iri}`

Return a preview rendering of an entity by IRI.
