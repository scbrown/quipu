//! Entity resolution, projection, context and reporting.
//!
//! Split out of `definitions.rs` under the file-size ratchet (aegis-gf3j7). The
//! blocks are MOVED VERBATIM and their order is preserved, so `tool_definitions()`
//! returns exactly the Vec it returned before — the split is provable, not argued.

use serde_json::Value as JsonValue;

pub(super) fn defs() -> Vec<JsonValue> {
    vec![
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
    ]
}
