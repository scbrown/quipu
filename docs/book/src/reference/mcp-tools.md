# MCP Tools

Quipu exposes its API as MCP (Model Context Protocol) tools for agent
integration. These tools are available when Quipu runs as a Bobbin subsystem
or standalone MCP server.

The registry (`tool_definitions()`) exposes **42 tools** in a default build, or
**43** when built with the `owl` feature (which adds `quipu_load_ontology`).
(The counts are pinned by tests in `src/mcp/tests.rs`, which also check this
page and the README against the manifest.)

## Tool Reference

### `quipu_query`

Execute a SPARQL SELECT query.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | SPARQL query string |
| `valid_at` | No | ISO-8601 timestamp for time-travel |
| `tx` | No | Transaction ID for time-travel |
| `graph` | No | Named-graph IRI or dataset name scoping the default graph (unknown IRI → empty default graph, never ROOT) |
| `fork` | No | Fork name to read (see `quipu fork`); unknown/dropped forks are refused; mutually exclusive with `graph` |
| `include_kinds` | No | `dataKind` tokens (e.g. `["archive"]`) that widen the default graph set with every graph declaring one of them. Absent/empty = unchanged scope; malformed tokens are refused; mutually exclusive with `fork`; a `FROM` in the query text still overrides |

### `quipu_export`

Export deterministic RDF from ROOT, a named graph, an episode provenance group,
or a SPARQL graph query. Scope parameters are mutually exclusive.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `graph` | No | Named-graph IRI to export (omit for ROOT; unknown IRI is an error) |
| `group_id` | No | Export ROOT entities attributed to this episode group |
| `construct` | No | SPARQL CONSTRUCT or DESCRIBE query whose graph is exported |
| `format` | No | `turtle` (default) or `ntriples` |

### `quipu_knot`

Assert facts from Turtle data, with optional SHACL validation.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `turtle` | Yes | RDF Turtle data |
| `timestamp` | No | Valid-time for the facts |
| `actor` | No | Who is asserting |
| `source` | No | Where the facts came from |
| `shapes` | No | SHACL Turtle for validation gate |
| `graph` | No | Registered committed-graph IRI to write into; unknown IRIs error, overlays refused; omit for ROOT |
| `replace_snapshot` | No | Replace this producer's prior facts (diffed), scoped to the target graph |
| `snapshot` | No | Stable producer key required by `replace_snapshot` |

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

### `quipu_set`

Atomically set `(entity, predicate)` to exactly one value: retracts every
current object on that predicate and asserts the new one in a single
transaction — the supersede primitive. Single-value semantics: to add without
removing, assert via `quipu_knot`.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `entity` | Yes | IRI of the entity (must exist) |
| `predicate` | Yes | Predicate IRI to set (may be new) |
| `value` | Yes | Bare string = literal; `{"iri": …}` for an edge; typed forms for int/float/bool/lang/datatype |
| `timestamp` | No | ISO-8601 valid-time for the supersede |
| `actor` | No | Who is performing the set |

### `quipu_retract_episode`

Episode-scoped **logical** retraction (`POST /episode/retract`). Retracts the
facts an episode's ingest contributed (activity node, entities, edges, reified
statements) by closing `valid_to` — logical, not physical, so time-travel history
is preserved. Entities and other episodes' facts (even about shared IRIs) are
untouched. Idempotent.

By default (`on_orphan: "preserve"`) it does **not** retract every currently-active
fact: it keeps `rdfs:label` / `rdf:type` alive for nodes that other episodes still
reference, so scope retraction cannot leave a node visible to predicate queries but
invisible to label/type scans.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `episode` | Yes | Episode name to retract (aliases: `episode_id`, `name`) |
| `timestamp` | No | Retraction timestamp |
| `actor` | No | Who is retracting |
| `on_orphan` | No | `preserve` (default) \| `refuse` (reject if it would orphan identity) \| `allow` (retract everything). Alias: `orphan_policy` |

Response: `tx_id`, `retracted`, `episode`, `statements`, plus identity accounting —
`on_orphan`, `identity_preserved` (+`identity_preserved_statements`), and
`identity_orphans` (+`identity_orphan_entities`).

Retraction is a more sensitive write than assertion. The endpoint honours
read-only mode and bearer auth today; when per-principal scopes (hq-azs) and
crew identity (hq-otm) land it should require an authorized principal.

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

Requires an embedding provider when called with `query` and no `embedding`;
without one it errors naming the missing `[quipu.embedding]` configuration.
The response carries an `embeddings` block (`configured`, `embedded_entities`)
so zero results are distinguishable from an unembedded store — see
[Embeddings and Semantic Search](../concepts/embeddings.md).
| `group_ids` | No | Best-effort filter to entities from these provenance groups (episode-scoped label, **not** an isolation boundary; `/knot` facts are ungrouped and dropped from a group scope) |
| `entity_type` | No | Restrict to entities of this rdf:type IRI |

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

Requires an embedding provider when called with `query` and no `embedding`;
without one it errors naming the missing `[quipu.embedding]` configuration.
The response carries an `embeddings` block (`configured`, `embedded_entities`)
so zero results are distinguishable from an unembedded store — see
[Embeddings and Semantic Search](../concepts/embeddings.md).

### `quipu_graph`

Project the knowledge graph into a render-ready node-link payload in one
response: nodes (IRI, label, type, degree), index-addressed edges, and a type
census. Episode/provenance scaffolding is excluded by default; nodes are
ranked by degree and capped, and the response states what was dropped.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `limit` | No | Max nodes, ranked by degree (default 250, hard max 2000) |
| `type` | No | Restrict to nodes of this rdf:type IRI |
| `include_episodes` | No | Include `prov:Activity` episode nodes (default false) |

### `quipu_project`

Graph projection and algorithms.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `algorithm` | No | `stats`, `in_degree`, `pagerank`/`ppr`, `components`, `louvain`, or `shortest_path` (default: `stats`) |
| `type` | No | Restrict projection to this rdf:type IRI |
| `predicate` | No | Restrict projection to edges with this predicate IRI |
| `graph` | No | Project one named graph's own facts instead of ROOT — cheap against a small derived layer even when the episode log is large |
| `limit` | No | Max results for in_degree/pagerank (default: 20) |
| `seeds` | No | Seed entity IRIs for personalized PageRank (non-empty switches pagerank to PPR) |
| `damping` | No | PageRank damping factor (default: 0.85) |
| `max_iters` | No | PageRank max iterations (default: 100) |
| `tolerance` | No | PageRank convergence tolerance (default: 1e-6) |
| `from` / `to` | No | Source/target entity IRIs for `shortest_path` |
| `persist` | No | `louvain`: persist `quipu:memberOfCommunity` facts; `pagerank` (global runs only — a seeded run refuses): persist `quipu:pageRank` scores. Both supersede any prior derivation (default: `false`). Communities are emergent clustering, **not** an access boundary. |

The `louvain` algorithm runs deterministic modularity-based community detection
and returns `{ communities: [{ community, entities, size }], modularity }`.
Read-only unless `persist: true`.

### `quipu_context`

Unified knowledge context pipeline.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Search query string |
| `max_entities` | No | Max entities (default from pipeline config) |
| `expand_links` | No | Follow relationships to linked entities |
| `ppr_rerank` | No | Re-order candidates by Personalized PageRank seeded at the direct hits before truncation (default: false) |

The `summary` includes an `embeddings` block (`configured`,
`embedded_entities`) reporting whether semantic retrieval was possible.

### `quipu_report`

Live graph report — graphify's `GRAPH_REPORT.md` equivalent, but queryable.
Read-only.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `type` | No | Restrict the projection to this rdf:type IRI |
| `predicate` | No | Restrict the projection to edges with this predicate IRI |
| `hubs` | No | Number of top hubs to return (default: 10) |
| `surprises` | No | Number of surprising connections to return (default: 10) |
| `questions` | No | Number of suggested questions to return (default: 8) |

Returns three sections:

- `hubs` — "god-nodes": the most central entities by PageRank, each with its
  `in_degree` as a secondary signal.
- `surprising_connections` — low-prior edges that bridge two otherwise-separate
  Louvain communities. Rarer bridges (fewer edges crossing between the same two
  communities — `bridge_rarity`) rank first; ties break toward bridges touching
  higher-PageRank endpoints.
- `suggested_questions` — deterministic, template-generated prompts seeded by the
  hubs and bridges above.

Plus a `graph` summary (`nodes`, `edges`, `communities`, `modularity`).
Communities here are emergent clustering for surfacing, **not** an access
boundary.

### `quipu_policy_check`

Committed-tier evaluation of a governance Policy over the graph of record.
Evaluates the policy's `aegis:claim` (a SPARQL ASK, optionally with a `$target`
placeholder) and returns a Verdict — `outcome` ∈ `satisfied | unsatisfied |
unknown` bound to a reproducible `evidence_hash`. Deterministic: any verifier
re-running the same ASK over the same committed evidence gets the same verdict
(checked, not trusted). The verdict is returned **unsigned** unless the store
has a signing identity attached.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `policy` | One of policy/claim | Policy IRI whose `aegis:claim` to evaluate |
| `claim` | One of policy/claim | Inline SPARQL ASK claim |
| `target` | Yes | Target IRI bound to the `$target` placeholder |
| `predicate_id` | No | Predicate identifier recorded in the verdict (inline claims; default `inline`) |
| `evidence_probe` | No | Inline ASK for "does the evidence exist?" — false yields `unknown` |
| `valid_at` | No | ISO-8601 point-in-time for valid-time evaluation |

### `quipu_verdict_verify`

Verify a signed Verdict against the Phase-0 root of trust: the signature must
be valid under the verifier's **registered** public key, and the verifier must
be authorized to attest the predicate. `trusted` is the conjunction — the
property a consumer should gate on.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `predicate_id` | Yes | Predicate the verdict attests |
| `target_ref` | Yes | Target the verdict is about |
| `outcome` | Yes | Verdict outcome |
| `evidence_hash` | Yes | Evidence hash the signature seals |
| `tier` | No | Evidence tier (default: `committed`) |
| `verifier` | Yes | Verifier IRI whose registered key verifies the signature |
| `signature` | Yes | Hex ed25519 signature over the verdict message |

### `quipu_verifier_authorized`

Check the Phase-0 verifier registry: may this verifier attest this predicate?
The discovery half of the governance gate.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `verifier` | Yes | Verifier IRI |
| `predicate` | Yes | Predicate IRI to attest |

### `quipu_cooccurrence`

Deterministic, auditable work-item co-occurrence: given a work-item (`Bead`)
IRI, returns the other work-items that share at least one touched code entity
via the provenance chain `Bead ←implements− GitCommit −modifies→ entity`.
A graph query over typed provenance edges, ordered by overlap strength.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `work_item` | Yes | Work-item (Bead) IRI |
| `valid_at` | No | ISO-8601 point-in-time for valid-time filtering |
| `tx` | No | Maximum transaction ID to consider |

### `quipu_overlay_create`

Register an overlay-class named graph bound (bind-once) to a committed parent
branch. Overlays are scratch layers over the committed graph: write hypotheses
into an overlay, read the composed view, and the committed layer stays
untouched.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `overlay` | Yes | Overlay graph IRI to register |
| `parent_branch` | No | Committed parent-branch IRI (omit for ROOT) |

### `quipu_overlay_write`

Write one overlay primitive: `assert`, `retract`, or `tombstone` a triple in an
overlay graph. Tombstone masks the parent branch's fact in the composed view
without touching the committed layer.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `overlay` | Yes | Overlay graph IRI |
| `op` | Yes | `assert`, `retract`, or `tombstone` |
| `subject` | Yes | Subject IRI |
| `predicate` | Yes | Predicate IRI |
| `object` | Yes | Object value (IRI string, literal, or typed JSON value) |
| `timestamp` | No | ISO-8601 valid-time (default: now) |

### `quipu_overlay_compose`

Resolve an overlay's composed view over `[overlay > parent-branch-root]`.
Read-only. Two precedence modes: `nearest` (default, the scratch-layer read —
asserted-and-not-tombstoned, nearest wins) and `governed` (the
quarantine-plane read — the parent's facts always win: an overlay value on a
same-subject-same-predicate slot the parent claims is suppressed, and an
overlay tombstone cannot mask a parent fact; it only masks the overlay's own
contributions).

| Parameter | Required | Description |
|-----------|----------|-------------|
| `overlay` | Yes | Overlay graph IRI |
| `precedence` | No | `nearest` (default) or `governed` |

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
| `rank_by_ppr` | No | Order the reached set by Personalized PageRank seeded at the root — each entry gains a `ppr` score (default: false) |
| `timestamp` | No | Timestamp for the speculative retraction (used when `remove=true`) |

### `quipu_path_cone`

Golden paths: compute the provenance cone of a trajectory — which steps did
its falsifier-gated verified result depend on? Per-step verdicts are
`in-cone` (load-bearing; pruning needs a human Decision), `out-of-cone`
(mechanically prunable), or `cannot-evaluate` (no derivation edges recorded —
never silently prunable). Refuses trajectories with no steps or no
falsifier-gated verification. See the
[golden-paths design](https://github.com/scbrown/quipu/blob/main/docs/design/golden-paths-blessing.md).

| Parameter | Required | Description |
|-----------|----------|-------------|
| `trajectory` | Yes | IRI of the Trajectory to analyse |
| `via` | No | Derivation predicate IRIs to walk, in addition to `verifiedBy` (always followed) |
| `hops` | No | Depth bound for the derivation walk (default: 8) |
| `base_ns` | No | Vocabulary namespace override (default: the store's `base_ns`) |

### `quipu_path_backtest`

Golden paths: backtest a pruned candidate (exemplar trajectory minus omitted
steps) over recorded history — which past trajectories with a shared
work-item topic would have conformed under `gp-grammar/1`, and how did their
work items close? Distinguishes 0 matches from cannot-evaluate, and refuses a
pattern it cannot compile.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `exemplar` | Yes | IRI of the exemplar Trajectory |
| `omit` | No | Step IRIs the candidate omits |
| `base_ns` | No | Vocabulary namespace override (default: the store's `base_ns`) |

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

### `quipu_queries`

Manage stored named queries — competency questions a consumer ships with its
domain, callable through `quipu_ask` alongside the compiled-in catalog.
Definitions are validated at load and versioned (re-loading a name closes the
prior version rather than overwriting it).

| Parameter | Required | Description |
|-----------|----------|-------------|
| `action` | No | `load`, `list` (default), `get`, or `remove` |
| `name` | For load/get/remove | Query name |
| `description` | For load | What the query answers |
| `template` | For load | SPARQL template with `{param}` placeholders |
| `dataset` | No | Dataset IRI this query is scoped to |
| `params` | No | Ordered param specs `{name, type, required, default, description}` |
| `timestamp` | No | ISO-8601 timestamp |

### `quipu_graph_list`

List registered named graphs with class, source, storage lifecycle, and labels
(freshness / durability / trust / policy / kind). The read half of the
graph-kinds surface, and the consumer **capability probe**: a store that does
not serve this tool (or `GET /graphs`) predates the kind axis, which a
consumer must treat as "cannot tell" — never as "no graphs".

| Parameter | Required | Description |
|-----------|----------|-------------|
| `kind` | No | Only graphs declaring this `dataKind` token (e.g. `operational`, `archive`) |
| `lifecycle` | No | Only graphs in this storage lifecycle state (`frozen`) |

### `quipu_graph_freeze`

Deep-freeze a named graph: export its **full history** (retracted rows and
transactions included) into a read-only archive pack, verify the copy by
content hash, delete the local rows, and re-attach the pack — the graph stays
addressable at the same IRI. Compose frozen graphs back in with `FROM <iri>`,
`FROM <urn:quipu:dataset:frozen>`, or `include_kinds: ["archive"]`. Known
cost: `as_of_tx` time travel is refused while any archive is attached
(pre-existing rule for attachments); valid-time queries survive.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `graph` | Yes | IRI of the committed graph to freeze |
| `out_dir` | No | Directory for the archive pack (default: beside the store file) |
| `timestamp` | Yes | ISO-8601 timestamp |
| `actor` | No | Who is freezing |

### `quipu_graph_thaw`

Thaw a frozen graph: verify its archive pack, detach it, restore the full
history into the local store under the same IRI, and reopen the graph for
writes. The pack file is kept on disk; the freeze registry row is closed,
never deleted.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `graph` | Yes | IRI of the frozen graph |
| `timestamp` | Yes | ISO-8601 timestamp |
| `actor` | No | Who is thawing |

### `quipu_datasets`

Manage named datasets — a reusable name for an arbitrary set of graphs, so it
can be labelled, governed and handed to another agent. `FROM <dataset-iri>`
then means `FROM` over its members.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `action` | No | `create`, `list` (default), `show`, or `remove` |
| `name` | For create/show/remove | Dataset IRI |
| `members` | For create | Graph IRIs, or `{"graph": …, "ord": N}` for a declared ordering |
| `timestamp` | No | ISO-8601 timestamp |
| `actor` | No | Who is creating the dataset |

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
