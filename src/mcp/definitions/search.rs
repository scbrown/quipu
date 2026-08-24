//! Search, ranking and graph-walk tools.
//!
//! Split out of `definitions.rs` under the file-size ratchet (aegis-gf3j7). The
//! blocks are MOVED VERBATIM and their order is preserved, so `tool_definitions()`
//! returns exactly the Vec it returned before — the split is provable, not argued.

use serde_json::Value as JsonValue;

pub(super) fn defs() -> Vec<JsonValue> {
    vec![
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
    ]
}
