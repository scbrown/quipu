# MCP Tools

Quipu exposes its API as MCP (Model Context Protocol) tools for agent
integration. These tools are available when Quipu runs as a Bobbin subsystem
or standalone MCP server.

The registry (`tool_definitions()`) exposes **23 tools** in a default build, or
**24** when built with the `owl` feature (which adds `quipu_load_ontology`).

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
| `group_id` | No | Provenance label for the episode (not an isolation boundary — see [Episodes](../architecture/episodes.md)) |
| `nodes` | No | Array of `{name, type, description, properties}` |
| `edges` | No | Array of `{source, target, relation}` |

### `quipu_search`

Semantic vector search over entity embeddings. Supply either a natural-language
`query` (auto-embedded when an `EmbeddingProvider` is attached) or a pre-computed
`embedding` vector. At least one is required.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | No | Natural-language query (auto-embedded; alternative to `embedding`) |
| `embedding` | No | Float array (query vector); takes precedence over `query` |
| `limit` | No | Max results (default: 10) |
| `valid_at` | No | Temporal filter |

### `quipu_hybrid_search`

Combined SPARQL filtering + vector ranking. Supply either a natural-language
`query` (auto-embedded) or a pre-computed `embedding`; the `sparql` pre-filter is
optional.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | No | Natural-language query (auto-embedded; alternative to `embedding`) |
| `embedding` | No | Float array (query vector); takes precedence over `query` |
| `sparql` | No | SPARQL pre-filter query (enables predicate pushdown) |
| `limit` | No | Max results (default: 10) |
| `valid_at` | No | Temporal filter |

### `quipu_project`

Graph projection and algorithms.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `algorithm` | No | `stats`, `in_degree`, `pagerank`, or `ppr` (default: `stats`) |
| `type` | No | Restrict projection to this rdf:type IRI |
| `predicate` | No | Restrict projection to edges with this predicate IRI |
| `limit` | No | Max results for in_degree/pagerank (default: 20) |
| `seeds` | No | Seed entity IRIs for personalized PageRank (non-empty switches pagerank to PPR) |
| `damping` | No | PageRank damping factor (default: 0.85) |
| `max_iters` | No | PageRank max iterations (default: 100) |
| `tolerance` | No | PageRank convergence tolerance (default: 1e-6) |

### `quipu_context`

Unified knowledge context pipeline.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Search query string |
| `max_entities` | No | Max entities (default from pipeline config) |
| `expand_links` | No | Follow relationships to linked entities |

### `quipu_search_nodes`

Search for entities by natural-language query (text matching on names, labels,
and values). Replaces Graphiti's `search_nodes`.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Natural-language search query |
| `group_ids` | No | Best-effort filter to entities from these provenance groups (episode-scoped label; `/knot` facts are ungrouped) |
| `max_results` | No | Max results (default: 10) |
| `entity_type_filter` | No | Filter by rdf:type IRI |

### `quipu_search_facts`

Search for relationships/edges by natural-language query (matches predicate or
value). Replaces Graphiti's `search_memory_facts`.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Natural-language search query |
| `group_ids` | No | Best-effort filter to facts from these provenance groups (episode-scoped label; `/knot` facts are ungrouped) |
| `max_results` | No | Max results (default: 10) |

### `quipu_episodes_complete`

Graphiti-compatible flat episode ingestion: accepts name, body text, group, and
source, then converts to a Quipu episode and ingests.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | Yes | Episode name/identifier |
| `episode_body` | No | Natural-language body of the episode |
| `group_id` | No | Provenance label for the episode (not an isolation boundary — see [Episodes](../architecture/episodes.md)) |
| `source_description` | No | Who/what produced this episode |
| `timestamp` | No | ISO-8601 timestamp |

### `quipu_impact`

Impact analysis: walk downstream from an entity. With `remove=true`,
speculatively retracts the entity first (counterfactual). The store is never
mutated.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `entity` | Yes | Entity IRI to analyse |
| `remove` | No | Speculatively retract before walking (default: false) |
| `hops` | No | Max edge hops to follow (default: 5) |
| `predicates` | No | Restrict walk to these predicate IRIs (empty = all) |
| `timestamp` | No | Timestamp for the speculative retraction (used when `remove=true`) |

### `quipu_unified_search`

Unified knowledge search for Bobbin integration: combines text and optional
vector search, returning results tagged `source="knowledge"` with normalized
0–1 scores.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Natural-language search query |
| `embedding` | No | Pre-computed query embedding (else auto-embedded when provider attached) |
| `limit` | No | Max results (default: 10) |
| `expand_links` | No | Expand results via graph links (default: true) |
| `max_facts_per_entity` | No | Max facts per entity (default: 10) |

### `quipu_ask`

Run a curated, parameterized **named query** by name instead of hand-writing
SPARQL. The catalog is self-describing: call with no `name` (or `name="list"`)
to list every query, its parameters, and their types.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | No | Named query to run; omit (or `"list"`) to list the catalog |
| `params` | No | Parameter map for the named query (names/types from the catalog) |

**Catalog:**

| Query | Parameters | Returns |
|-------|------------|---------|
| `entity_facts` | `entity` (iri), `limit` (int, 100) | All facts asserted about an entity |
| `service_deps` | `entity` (iri), `limit` (int, 50) | Outgoing entity references (dependencies / links) |
| `references_to` | `entity` (iri), `limit` (int, 50) | Entities that reference the given entity (incoming) |
| `entities_of_type` | `type` (iri), `limit` (int, 100) | All entities of a given `rdf:type` |
| `labeled_like` | `text` (text), `limit` (int, 50) | Entities whose `rdfs:label` contains `text` (case-insensitive) |

Parameters are validated and escaped by type before substitution, so values are
safe against SPARQL injection. The response includes the resolved `sparql`, the
result `columns`, and `rows`.

**Example** — service dependencies of an entity:

```json
{ "name": "service_deps", "params": { "entity": "http://example.org/traefik" } }
```

### `quipu_propose_schema_change`

Submit a schema-evolution proposal (shape, class, property, or ontology change).
Proposals require explicit acceptance before taking effect.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `kind` | Yes | `shape`, `ontology`, `class`, or `property` |
| `target` | Yes | Shape name, class IRI, or property IRI being changed |
| `diff` | Yes | Turtle fragment or JSON patch describing the change |
| `proposer` | Yes | Identity of the proposing agent |
| `rationale` | No | Why this change is needed |
| `trigger_ref` | No | Validation-failure ref or bead id that triggered this |
| `timestamp` | No | ISO-8601 timestamp |

### `quipu_list_proposals`

List schema-evolution proposals, optionally filtered by status.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `status` | No | `pending`, `accepted`, or `rejected` (default: all) |

### `quipu_accept_proposal`

Accept a pending schema proposal. Shape proposals are validated before writing.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `id` | Yes | Proposal ID to accept |
| `decided_by` | No | Identity of the approver |
| `note` | No | Optional acceptance note |
| `timestamp` | No | ISO-8601 timestamp |

### `quipu_reject_proposal`

Reject a pending schema proposal with a reason.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `id` | Yes | Proposal ID to reject |
| `note` | Yes | Reason for rejection |
| `decided_by` | No | Identity of the rejector |
| `timestamp` | No | ISO-8601 timestamp |

### `quipu_resolve_entity`

Check for existing near-duplicate entities before writing, using vector
similarity and canonical-name matching (Jaro-Winkler). Returns candidates with
similarity scores and match explanations.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | Yes | Canonical name of the proposed entity |
| `properties` | No | Key-value properties (used for embedding context) |
| `top_k` | No | Max candidates to return (default: 3) |
| `threshold` | No | Similarity threshold 0.0–1.0 (default: 0.85) |

### `quipu_load_ontology` (requires `owl` feature)

Manage OWL ontologies: `load` (parse + materialize entailments), `list`, or
`remove`. Only registered when Quipu is built with the `owl` feature.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `action` | No | `load`, `list`, or `remove` (default: list) |
| `name` | For load/remove | Ontology name |
| `turtle` | For load | OWL ontology in Turtle format |
| `timestamp` | No | ISO-8601 timestamp |
