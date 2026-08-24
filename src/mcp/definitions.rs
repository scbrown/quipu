//! The MCP tool manifest — every tool definition quipu serves, split from
//! `mod.rs` to keep that module under the file-size ratchet.

use serde_json::Value as JsonValue;

/// MCP tool definitions as JSON schemas for registration with Bobbin.
pub fn tool_definitions() -> Vec<JsonValue> {
    #[allow(unused_mut)]
    let mut defs = vec![
        serde_json::json!({
            "name": "quipu_query",
            "description": "Execute a SPARQL SELECT query against the knowledge graph (supports time-travel via valid_at/tx)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "SPARQL SELECT query" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601). Omit for current state." },
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider. Omit for all transactions." }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_export",
            "description": "Export a scoped SUBSET of the graph as RDF: one named graph's facts (quipu #36), or the ROOT default graph when 'graph' is omitted. The 'pull a scoped slice' primitive for subset-export and federation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "Named-graph IRI to export. Omit for the ROOT/default graph. Unknown IRI is an error." },
                    "format": { "type": "string", "enum": ["turtle", "ntriples"], "description": "RDF serialization (default: turtle)." }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_knot",
            "description": "Assert facts into the knowledge graph (with optional SHACL validation)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turtle": { "type": "string", "description": "RDF data in Turtle format to assert" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the assertion" },
                    "actor": { "type": "string", "description": "Who is making the assertion" },
                    "source": { "type": "string", "description": "Provenance source (episode, file, etc.)" },
                    "shapes": { "type": "string", "description": "Optional SHACL shapes in Turtle for validation" },
                    "graph": { "type": "string", "description": "Named-graph IRI to write into. Must already be registered committed-class (graph_create). Unknown IRI is an error, never interned. Omit for ROOT." },
                    "replace_snapshot": { "type": "boolean", "description": "Replace the prior facts written under this snapshot key (diffed: unchanged facts stay live). Requires 'snapshot'." },
                    "snapshot": { "type": "string", "description": "Stable producer key scoping replace_snapshot (e.g. 'bobbin-chunks:myrepo'). Scoped to the target graph." }
                },
                "required": ["turtle"]
            }
        }),
        serde_json::json!({
            "name": "quipu_cord",
            "description": "List entities in the knowledge graph, optionally filtered by type or predicate",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Filter by rdf:type IRI" },
                    "predicate": { "type": "string", "description": "Filter by predicate IRI" },
                    "limit": { "type": "integer", "description": "Maximum number of entities (default: 100)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_unravel",
            "description": "Time-travel query: see facts as they were at a given point",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_validate",
            "description": "Validate RDF data against SHACL shapes without writing",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "shapes": { "type": "string", "description": "SHACL shapes in Turtle format" },
                    "data": { "type": "string", "description": "RDF data in Turtle format to validate" }
                },
                "required": ["shapes", "data"]
            }
        }),
        serde_json::json!({
            "name": "quipu_shapes",
            "description": "Manage persistent SHACL shapes (load, list, get, remove) and inspect the loaded class vocabulary. Loaded shapes auto-validate on writes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["load", "list", "get", "remove", "vocabulary"], "description": "Action to perform (default: list)" },
                    "name": { "type": "string", "description": "Shape graph name (required for load/remove)" },
                    "turtle": { "type": "string", "description": "SHACL shapes in Turtle format (required for load)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_queries",
            "description": "Manage STORED named queries — competency questions a consumer ships with its domain, callable through quipu_ask alongside the compiled-in catalog. Definitions are validated at LOAD (template must parse; every {placeholder} needs a spec; an optional param needs a default) and versioned: re-loading a name closes the prior version rather than overwriting it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["load", "list", "get", "remove"], "description": "Action to perform (default: list)" },
                    "name": { "type": "string", "description": "Query name (required for load/get/remove)" },
                    "description": { "type": "string", "description": "What the query answers" },
                    "template": { "type": "string", "description": "SPARQL template with {param} placeholders (required for load)" },
                    "dataset": { "type": "string", "description": "Optional dataset IRI this query is scoped to; activates it unless the caller passes `graph`" },
                    "params": { "type": "array", "description": "Ordered param specs: {name, type: iri|text|int, required, default, description}", "items": {} },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_graph_list",
            "description": "List registered named graphs with class, source, lifecycle and labels (freshness/durability/trust/policy/kind). Filter with `kind` (a dataKind token, e.g. operational | archive) and/or `lifecycle` (frozen). Also the capability probe for the graph-kinds surface: a store without this tool predates it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Only graphs declaring this dataKind token" },
                    "lifecycle": { "type": "string", "enum": ["frozen"], "description": "Only graphs in this storage lifecycle state" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_datasets",
            "description": "Manage named datasets — a reusable NAME for an arbitrary set of graphs, so it can be labelled, governed and handed to another agent. `FROM <dataset-iri>` then means FROM over its members. Datasets overlap freely and are never implicitly active.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["create", "list", "show", "remove"], "description": "Action to perform (default: list)" },
                    "name": { "type": "string", "description": "Dataset IRI (required for create/show/remove)" },
                    "members": { "type": "array", "description": "Graph IRIs, or {\"graph\": \"<iri>\", \"ord\": N} objects for a declared ordering. Duplicate ranks are refused.", "items": {} },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" },
                    "actor": { "type": "string", "description": "Who is creating the dataset" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_retract",
            "description": "Retract facts for an entity: all of them, or narrowed by predicate and/or value. Entity + predicate + value retracts exactly ONE (e,a,v) statement — use this to remove a stray edge instead of retracting a whole episode and rebuilding it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "IRI of the entity to retract" },
                    "predicate": { "type": "string", "description": "Optional: only retract facts with this predicate IRI" },
                    "value": { "description": "Optional: only retract facts with this object value. A bare string is a literal; use {\"iri\": \"...\"} for a reference, or {\"int\"|\"float\"|\"bool\": ...}. With entity + predicate this pins a single triple." },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the retraction" },
                    "actor": { "type": "string", "description": "Who is performing the retraction" }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "quipu_set",
            "description": "Atomically SET (entity, predicate) to exactly one value: retracts every current object on that predicate and asserts the new one in a single transaction. The supersede primitive — re-parenting (reports_to A -> B) is one call, with no empty-predicate window and no way to end up with two supervisors by forgetting the retract half. SINGLE-VALUE semantics: replaces ALL current objects; to add without removing, assert via quipu_knot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "IRI of the entity (must exist)" },
                    "predicate": { "type": "string", "description": "Predicate IRI to set (may be new)" },
                    "value": { "description": "New object value. A bare string is a literal; use {\"iri\": \"...\"} for an edge, or {\"int\"|\"float\"|\"bool\": ...} / {\"value\", \"lang\"|\"datatype\"}. A bare string aimed at an IRI-valued predicate is refused loudly." },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the supersede" },
                    "actor": { "type": "string", "description": "Who is performing the set" }
                },
                "required": ["entity", "predicate", "value"]
            }
        }),
        serde_json::json!({
            "name": "quipu_retract_episode",
            "description": "Episode-scoped logical retraction: retract all currently-active facts an episode's ingest contributed (activity node, entities, edges, reified statements), via the bitemporal valid_to close path. Logical, not physical — time-travel history is preserved. Entities and other episodes' facts are untouched. Idempotent. Node IDENTITY is protected: by default (on_orphan=preserve) rdfs:label/rdf:type survive for any node that other episodes still reference, so retraction never leaves an unlabelled, untyped ghost. The response always reports identity_orphans.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "episode": { "type": "string", "description": "Episode name/identifier to retract (aliases: episode_id, name)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the retraction" },
                    "actor": { "type": "string", "description": "Who is performing the retraction" },
                    "on_orphan": { "type": "string", "enum": ["preserve", "refuse", "allow"], "description": "What to do when retraction would strip rdfs:label/rdf:type from a node other episodes still reference. preserve (default): keep its identity alive. refuse: reject the whole retraction. allow: legacy strict scope — creates ghosts, reported in identity_orphans." }
                },
                "required": ["episode"]
            }
        }),
        serde_json::json!({
            "name": "quipu_episode",
            "description": "Ingest structured knowledge from an agent episode (nodes + edges)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Episode name/identifier" },
                    "episode_body": { "type": "string", "description": "Natural language description of the knowledge" },
                    "source": { "type": "string", "description": "Who/what produced this episode" },
                    "group_id": { "type": "string", "description": "Knowledge graph group (e.g. aegis-ontology)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the assertion" },
                    "nodes": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "type": { "type": "string" }, "description": { "type": "string" }, "properties": { "type": "object" } }, "required": ["name"] }, "description": "Entity nodes to create" },
                    "edges": { "type": "array", "items": { "type": "object", "properties": { "source": { "type": "string" }, "target": { "type": "string" }, "relation": { "type": "string" } }, "required": ["source", "target", "relation"] }, "description": "Relationship edges between nodes" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_search",
            "description": "Semantic vector search over entity embeddings. Accepts a pre-computed embedding vector or a natural-language query (auto-embedded when an EmbeddingProvider is configured).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query (auto-embedded when EmbeddingProvider is attached)" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Pre-computed query embedding vector (f32 array). Takes precedence over query." },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "valid_at": { "type": "string", "description": "Point-in-time for temporal filtering (ISO-8601)" },
                    "group_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: best-effort filter to entities from these provenance groups (episode-scoped label, NOT an isolation boundary; `/knot` facts are ungrouped and dropped from a group scope)" },
                    "entity_type": { "type": "string", "description": "Optional: restrict to entities of this rdf:type IRI" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_hybrid_search",
            "description": "Combined SPARQL + vector similarity search with predicate pushdown. Accepts a pre-computed embedding or natural-language query. Simple type constraints (e.g. ?s a <Type>) are pushed into the vector index for O(log n) filtered ANN.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query (auto-embedded when EmbeddingProvider is attached)" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Pre-computed query embedding vector (f32 array). Takes precedence over query." },
                    "sparql": { "type": "string", "description": "SPARQL SELECT query returning entity IRIs in the first variable. Simple type patterns (e.g. ?s a <Type>) enable predicate pushdown." },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "valid_at": { "type": "string", "description": "Point-in-time for temporal filtering (ISO-8601)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_search_nodes",
            "description": "Search for entities in the knowledge graph by natural language query. Uses text matching on entity names, labels, and values. Replaces Graphiti's search_nodes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "group_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: filter to entities from these knowledge graph groups" },
                    "max_results": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "entity_type_filter": { "type": "string", "description": "Optional: filter by rdf:type IRI" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_search_facts",
            "description": "Search for relationships/edges in the knowledge graph by natural language query. Finds facts where the predicate or value matches the query. Replaces Graphiti's search_memory_facts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "group_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: filter to facts from these knowledge graph groups" },
                    "max_results": { "type": "integer", "description": "Maximum results (default: 10)" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_episodes_complete",
            "description": "Graphiti-compatible flat episode ingestion. Accepts name, body text, group, and source — converts to Quipu episode and ingests.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Episode name/identifier" },
                    "episode_body": { "type": "string", "description": "Natural language body of the episode" },
                    "group_id": { "type": "string", "description": "Knowledge graph group (e.g. aegis-ontology)" },
                    "source_description": { "type": "string", "description": "Who/what produced this episode" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the assertion" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_impact",
            "description": "Impact analysis: walk downstream from an entity. With remove=true, speculatively retracts the entity first (counterfactual: 'what would break if I removed this?'). The store is never mutated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "IRI of the entity to analyse" },
                    "remove": { "type": "boolean", "description": "If true, speculatively retract the entity before walking (counterfactual mode). Default: false." },
                    "hops": { "type": "integer", "description": "Maximum edge hops to follow (default: 5)" },
                    "rank_by_ppr": { "type": "boolean", "description": "Order the reached set by Personalized PageRank seeded at the root (each entry gains a 'ppr' score) instead of BFS discovery order (default: false)" },
                    "predicates": { "type": "array", "items": { "type": "string" }, "description": "Restrict walk to these predicate IRIs. Empty = all edges." },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the speculative retraction (used only when remove=true)" }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "quipu_path_cone",
            "description": "Golden paths: compute the provenance cone of a trajectory — which steps did its falsifier-gated verified result depend on? Per-step verdicts are in-cone (load-bearing; pruning needs a human Decision), out-of-cone (mechanically prunable), or cannot-evaluate (no derivation edges recorded — never silently prunable). Refuses trajectories with no steps or no falsifier-gated verification.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "trajectory": { "type": "string", "description": "IRI of the Trajectory to analyse" },
                    "via": { "type": "array", "items": { "type": "string" }, "description": "Derivation predicate IRIs to walk, in addition to verifiedBy (always followed)." },
                    "hops": { "type": "integer", "description": "Depth bound for the derivation walk (default: 8)" },
                    "base_ns": { "type": "string", "description": "Vocabulary namespace override; defaults to the store's configured base_ns." }
                },
                "required": ["trajectory"]
            }
        }),
        serde_json::json!({
            "name": "quipu_path_backtest",
            "description": "Golden paths: backtest a pruned candidate (exemplar trajectory minus omitted steps) over recorded history — which past trajectories with a shared work-item topic would have conformed under gp-grammar/1, and how did their work items close? Distinguishes 0 matches from cannot-evaluate, and refuses a pattern it cannot compile.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exemplar": { "type": "string", "description": "IRI of the exemplar Trajectory" },
                    "omit": { "type": "array", "items": { "type": "string" }, "description": "Step IRIs the candidate omits" },
                    "base_ns": { "type": "string", "description": "Vocabulary namespace override; defaults to the store's configured base_ns." }
                },
                "required": ["exemplar"]
            }
        }),
        serde_json::json!({
            "name": "quipu_unified_search",
            "description": "Unified knowledge search for Bobbin integration. Combines text and optional vector search, returning results tagged with source='knowledge' and normalized 0-1 scores. When an EmbeddingProvider is attached, the query is auto-embedded for semantic search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Optional pre-computed query embedding. When omitted and EmbeddingProvider is attached, query is auto-embedded." },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "expand_links": { "type": "boolean", "description": "Expand results via graph links (default: true)" },
                    "max_facts_per_entity": { "type": "integer", "description": "Maximum facts per entity (default: 10)" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_ask",
            "description": "Run a curated, parameterized named query by name instead of hand-writing SPARQL. Call with no 'name' (or name='list') to discover the self-describing catalog of available queries and their parameters (e.g. service_deps, references_to, entity_facts, entities_of_type, labeled_like).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Named query to run; omit (or 'list') to list the catalog." },
                    "params": { "type": "object", "description": "Parameter map for the named query (see catalog for names/types)." }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_propose_schema_change",
            "description": "Submit a schema evolution proposal (new shape, class, property, or ontology change). Proposals require explicit acceptance before taking effect.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["shape", "ontology", "class", "property"], "description": "Kind of schema change" },
                    "target": { "type": "string", "description": "Shape name, class IRI, or property IRI being changed" },
                    "diff": { "type": "string", "description": "Turtle fragment or JSON patch describing the change" },
                    "rationale": { "type": "string", "description": "Why this change is needed" },
                    "proposer": { "type": "string", "description": "Identity of the proposing agent" },
                    "trigger_ref": { "type": "string", "description": "Validation failure ref or bead id that triggered this proposal" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                },
                "required": ["kind", "target", "diff", "proposer"]
            }
        }),
        serde_json::json!({
            "name": "quipu_list_proposals",
            "description": "List schema evolution proposals, optionally filtered by status (pending, accepted, rejected)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["pending", "accepted", "rejected"], "description": "Filter by proposal status (default: all)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_accept_proposal",
            "description": "Accept a pending schema proposal. For shape proposals, validates the Turtle before writing to the shapes table.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Proposal ID to accept" },
                    "decided_by": { "type": "string", "description": "Identity of the approver (default: aegis/crew/braino)" },
                    "note": { "type": "string", "description": "Optional acceptance note" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                },
                "required": ["id"]
            }
        }),
        serde_json::json!({
            "name": "quipu_reject_proposal",
            "description": "Reject a pending schema proposal with a reason",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Proposal ID to reject" },
                    "decided_by": { "type": "string", "description": "Identity of the rejector (default: aegis/crew/braino)" },
                    "note": { "type": "string", "description": "Reason for rejection" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                },
                "required": ["id", "note"]
            }
        }),
        serde_json::json!({
            "name": "quipu_resolve_entity",
            "description": "Check for existing near-duplicate entities before writing. Uses vector similarity (embedding) and canonical name matching (Jaro-Winkler) to find entities that may be duplicates. Returns candidates with similarity scores and match explanations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Canonical name of the proposed entity" },
                    "properties": { "type": "object", "description": "Optional key-value properties of the entity (used for embedding context)" },
                    "top_k": { "type": "integer", "description": "Maximum number of candidates to return (default: 3)" },
                    "threshold": { "type": "number", "description": "Similarity threshold 0.0-1.0 (default: 0.85)" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_graph",
            "description": "Project the knowledge graph into a render-ready node-link payload in ONE response: nodes (iri, label, rdf:type, degree), edges as [source_index, target_index, predicate] into that node array, and a type census ordered by prevalence. Excludes prov:Activity episodes and rdf/rdfs/prov scaffolding predicates by default so the domain graph is not buried in provenance. Nodes are ranked by degree and capped; the response states what was dropped.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max nodes to return, ranked by degree (default: 250, hard max: 2000)" },
                    "type": { "type": "string", "description": "Restrict to nodes of this rdf:type IRI (edges are scoped to the filtered set too)" },
                    "include_episodes": { "type": "boolean", "description": "Include prov:Activity episode nodes (default: false)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_project",
            "description": "Project the knowledge graph and run a graph algorithm over it: stats (node/edge counts), in_degree (most-referenced entities), pagerank/ppr (global or personalized PageRank from seed entities), components (weakly-connected components), louvain (modularity community detection), or shortest_path. Optionally restrict the projection to a node type or predicate. Read-only by default; louvain with persist:true writes quipu:memberOfCommunity facts (superseding any prior derivation).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "algorithm": { "type": "string", "enum": ["stats", "in_degree", "pagerank", "ppr", "components", "louvain", "shortest_path"], "description": "Algorithm to run (default: stats)" },
                    "type": { "type": "string", "description": "Restrict the projection to nodes of this rdf:type IRI" },
                    "predicate": { "type": "string", "description": "Restrict the projection to edges with this predicate IRI" },
                    "graph": { "type": "string", "description": "Project ONE named graph's own facts instead of ROOT (quipu-tz5) — cheap against a small derived layer even when the episode log is large" },
                    "limit": { "type": "integer", "description": "Max results for in_degree/pagerank (default: 20)" },
                    "seeds": { "type": "array", "items": { "type": "string" }, "description": "Seed entity IRIs (or raw term IDs) for personalized PageRank; non-empty switches pagerank to PPR" },
                    "damping": { "type": "number", "description": "PageRank damping factor (default: 0.85)" },
                    "max_iters": { "type": "integer", "description": "PageRank max iterations (default: 100)" },
                    "tolerance": { "type": "number", "description": "PageRank convergence tolerance (default: 1e-6)" },
                    "from": { "type": "string", "description": "Source entity IRI for shortest_path" },
                    "to": { "type": "string", "description": "Target entity IRI for shortest_path" },
                    "persist": { "type": "boolean", "description": "louvain only: persist quipu:memberOfCommunity facts (emergent clustering, NOT an access boundary), bitemporally superseding any prior derivation (default: false)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_context",
            "description": "Query the knowledge graph for context around a natural-language query: returns relevant entities and their facts, ready to prime an agent. Optionally expand to linked entities.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language query to find relevant knowledge context" },
                    "max_entities": { "type": "integer", "description": "Maximum entities to return (default from pipeline config)" },
                    "expand_links": { "type": "boolean", "description": "Whether to expand to entities linked from the matches" },
                    "ppr_rerank": { "type": "boolean", "description": "Re-order candidates by Personalized PageRank seeded at the direct hits before truncation (default: false)" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_report",
            "description": "Generate a live graph report (graphify's GRAPH_REPORT.md equivalent, but queryable): top hubs / 'god-nodes' (by PageRank with in-degree as a secondary signal), surprising connections (low-prior edges that bridge two otherwise-separate Louvain communities — rarer bridges rank first), and auto-suggested questions seeded by those hubs and bridges. Read-only; derived from current graph structure. Communities here are emergent clustering for surfacing, NOT an access boundary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Restrict the projection to nodes of this rdf:type IRI" },
                    "predicate": { "type": "string", "description": "Restrict the projection to edges with this predicate IRI" },
                    "hubs": { "type": "integer", "description": "Number of top hubs to return (default: 10)" },
                    "surprises": { "type": "integer", "description": "Number of surprising connections to return (default: 10)" },
                    "questions": { "type": "integer", "description": "Number of suggested questions to return (default: 8)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_policy_check",
            "description": "Committed-tier evaluation of a governance Policy over the graph of record: evaluates the policy's aegis:claim (a SPARQL ASK, optionally with a $target placeholder) against the committed graph and returns a Verdict — outcome ∈ {satisfied | unsatisfied | unknown} bound to a reproducible evidence_hash. Deterministic and reproducible: any verifier re-running the same ASK over the same committed evidence gets the same verdict (checked, not trusted). Returns the verdict UNSIGNED (signing + persistence is the Phase-0 verifier registry's job).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "policy": { "type": "string", "description": "Policy IRI whose aegis:claim to evaluate (alternative to inline 'claim')" },
                    "claim": { "type": "string", "description": "Inline SPARQL ASK claim (alternative to 'policy')" },
                    "target": { "type": "string", "description": "Target IRI bound to the $target placeholder" },
                    "predicate_id": { "type": "string", "description": "Predicate identifier recorded in the verdict (inline claims only; default: 'inline')" },
                    "evidence_probe": { "type": "string", "description": "Inline ASK for 'does the evidence exist?' — false yields outcome 'unknown' instead of 'unsatisfied'" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time evaluation (ISO-8601). Omit for current state." }
                },
                "required": ["target"]
            }
        }),
        serde_json::json!({
            "name": "quipu_verdict_verify",
            "description": "Verify a signed Verdict against the Phase-0 root of trust: the signature must be valid under the verifier's REGISTERED public key AND the verifier must be authorized to attest the predicate. 'trusted' is the conjunction — the property a consumer should gate on (checked, not trusted-by-assertion).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "predicate_id": { "type": "string", "description": "Predicate the verdict attests" },
                    "target_ref": { "type": "string", "description": "Target the verdict is about" },
                    "outcome": { "type": "string", "description": "Verdict outcome (satisfied | unsatisfied | unknown)" },
                    "evidence_hash": { "type": "string", "description": "Evidence hash the signature seals" },
                    "tier": { "type": "string", "description": "Evidence tier (default: committed)" },
                    "verifier": { "type": "string", "description": "Verifier IRI whose registered key verifies the signature" },
                    "signature": { "type": "string", "description": "Hex ed25519 signature over the verdict message" }
                },
                "required": ["predicate_id", "target_ref", "outcome", "evidence_hash", "verifier", "signature"]
            }
        }),
        serde_json::json!({
            "name": "quipu_verifier_authorized",
            "description": "Check the Phase-0 verifier registry: may this verifier attest this predicate? The discovery half of the governance gate — lets an agent learn who is authorized before trusting (or requesting) an attestation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "verifier": { "type": "string", "description": "Verifier IRI" },
                    "predicate": { "type": "string", "description": "Predicate IRI to attest" }
                },
                "required": ["verifier", "predicate"]
            }
        }),
        serde_json::json!({
            "name": "quipu_cooccurrence",
            "description": "Deterministic, auditable work-item co-occurrence: given a work-item (Bead) IRI, returns the other work-items that share at least one touched code entity via the provenance chain Bead <-implements- GitCommit -modifies-> entity. A graph query over typed provenance edges, not a statistical mine; ordered by overlap strength. Bitemporal: pass valid_at for 'which work co-occurred as of <date>'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "work_item": { "type": "string", "description": "Work-item (Bead) IRI" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601)" },
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider" }
                },
                "required": ["work_item"]
            }
        }),
        serde_json::json!({
            "name": "quipu_overlay_create",
            "description": "Register an overlay-class named graph bound (bind-once) to a committed parent branch. Overlays are scratch layers over the committed graph: write hypotheses into an overlay, read the composed view, and the committed layer stays untouched.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "overlay": { "type": "string", "description": "Overlay graph IRI to register" },
                    "parent_branch": { "type": "string", "description": "Committed parent-branch IRI (omit or null for ROOT)" }
                },
                "required": ["overlay"]
            }
        }),
        serde_json::json!({
            "name": "quipu_overlay_write",
            "description": "Write one overlay primitive: assert, retract, or tombstone a triple in an overlay graph. Tombstone masks the parent branch's fact in the composed view without touching the committed layer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "overlay": { "type": "string", "description": "Overlay graph IRI" },
                    "op": { "type": "string", "enum": ["assert", "retract", "tombstone"], "description": "Overlay primitive to apply" },
                    "subject": { "type": "string", "description": "Subject IRI" },
                    "predicate": { "type": "string", "description": "Predicate IRI" },
                    "object": { "description": "Object value (IRI string, literal, or typed JSON value)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 valid-time (default: now)" }
                },
                "required": ["overlay", "op", "subject", "predicate", "object"]
            }
        }),
        serde_json::json!({
            "name": "quipu_overlay_compose",
            "description": "Resolve an overlay's composed view over [overlay > parent-branch-root]: asserted-and-not-tombstoned, nearest wins. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "overlay": { "type": "string", "description": "Overlay graph IRI" }
                },
                "required": ["overlay"]
            }
        }),
    ];

    // OWL reasoning is gated behind the (non-default) `owl` feature; only
    // advertise the tool when its handler is actually compiled in, otherwise
    // agents see a tool whose call always fails (hq-8wd).
    #[cfg(feature = "owl")]
    defs.push(serde_json::json!({
        "name": "quipu_load_ontology",
        "description": "Manage OWL ontologies: load (parse + materialize entailments), list, or remove. Loaded ontologies enforce class hierarchy, disjoint-class constraints, and property characteristics (inverse, symmetric, functional).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["load", "list", "remove"], "description": "Action to perform (default: list)" },
                "name": { "type": "string", "description": "Ontology name (required for load/remove)" },
                "turtle": { "type": "string", "description": "OWL ontology in Turtle format (required for load)" },
                "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
            }
        }
    }));

    defs
}
