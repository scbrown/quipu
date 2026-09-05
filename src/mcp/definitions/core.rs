//! Core graph access: query, export, mutation, shapes, saved queries, datasets.
//!
//! Split out of `definitions.rs` under the file-size ratchet (aegis-gf3j7). The
//! blocks are MOVED VERBATIM and their order is preserved, so `tool_definitions()`
//! returns exactly the Vec it returned before — the split is provable, not argued.

use serde_json::Value as JsonValue;

pub(super) fn defs() -> Vec<JsonValue> {
    vec![
        serde_json::json!({
            "name": "quipu_query",
            "description": "Execute a SPARQL SELECT query against the knowledge graph (supports time-travel via valid_at/tx)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "SPARQL SELECT query" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601). Omit for current state." },
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider. Omit for all transactions." },
                    "verbose": { "type": "boolean", "description": "Return expanded full IRIs instead of default CURIE-compacted values." }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_export",
            "description": "Export deterministic RDF scoped to one named graph, provenance group, SPARQL CONSTRUCT, or ROOT when no scope is supplied. Scope fields are mutually exclusive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "Named-graph IRI to export. Omit for the ROOT/default graph. Unknown IRI is an error." },
                    "group_id": { "type": "string", "description": "Export ROOT entities attributed to episodes in this provenance group." },
                    "construct": { "type": "string", "description": "SPARQL CONSTRUCT or DESCRIBE query whose graph result is exported." },
                    "format": { "type": "string", "enum": ["turtle", "ntriples"], "description": "RDF serialization (default: turtle)." }
                }
            }
        }),
        // Alignment (aegis-5qmg3r). THREE tools, not one with a `mode`: codex
        // judges a tool by its annotation, a moded tool would need the
        // destructive one because it CAN write, and `propose` — the entry point
        // — would then hit the approval-never refusal. The operator's agent
        // could not start the operator's feature.
        serde_json::json!({
            "name": "quipu_align_propose",
            "description": "READ. Propose candidate cross-graph alignments between two named graphs, scored, as an SSSOM mapping set. Returns expected_version, which align_apply requires. Refuses an unknown graph IRI rather than returning 0 candidates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph_a": { "type": "string", "description": "First named-graph IRI. Must exist; an unknown IRI is refused, not treated as empty." },
                    "graph_b": { "type": "string", "description": "Second named-graph IRI." },
                    "mapping_set_id": { "type": "string", "description": "Identifier for the produced mapping set." }
                },
                "required": ["graph_a", "graph_b"]
            }
        }),
        serde_json::json!({
            "name": "quipu_align_decide",
            "description": "READ. Apply operator accept/negate decisions to a proposed mapping set. Touches no store. Returns the decided set and the expected_version to carry into align_apply.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "set_tsv": { "type": "string", "description": "SSSOM TSV from align_propose." },
                    "reviewer": { "type": "string", "description": "Who is deciding." },
                    "decisions": {
                        "type": "array",
                        "description": "Per-pair verdicts.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "subject_id": { "type": "string" },
                                "object_id": { "type": "string" },
                                "decision": { "type": "string", "enum": ["accept", "negate"] }
                            },
                            "required": ["subject_id", "object_id", "decision"]
                        }
                    }
                },
                "required": ["set_tsv", "reviewer", "decisions"]
            }
        }),
        serde_json::json!({
            "name": "quipu_align_apply",
            "description": "WRITE. Materialise decided alignments as owl:sameAs / quipu:distinctFrom in a derived alignment graph. expected_version is REQUIRED and must be carried from the decision: computing it here would hash the set being written, always match, and silently discard a concurrent operator's decision.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "set_tsv": { "type": "string", "description": "Decided SSSOM TSV from align_decide." },
                    "graph_a": { "type": "string", "description": "First source graph IRI (the pair given to align_propose)." },
                    "graph_b": { "type": "string", "description": "Second source graph IRI." },
                    "expected_version": { "type": "string", "description": "REQUIRED. From align_propose or align_decide. Never recomputed locally." },
                    "actor": { "type": "string", "description": "Who is applying." }
                },
                "required": ["set_tsv", "graph_a", "graph_b", "expected_version"]
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
            "name": "quipu_graph_freeze",
            "description": "Deep-freeze a named graph: export its FULL history (retracted rows and transactions included) into a read-only archive pack, verify the copy by content hash, delete the local rows, and re-attach the pack so the graph stays addressable at the same IRI. Compose frozen graphs back in with FROM <iri>, the urn:quipu:dataset:frozen dataset, or include_kinds:[\"archive\"]. Refuses ROOT, the meta-graph, overlays, attached graphs, and double-freezes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "IRI of the committed graph to freeze" },
                    "out_dir": { "type": "string", "description": "Directory for the archive pack (default: beside the store file)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" },
                    "actor": { "type": "string", "description": "Who is freezing" }
                },
                "required": ["graph", "timestamp"]
            }
        }),
        serde_json::json!({
            "name": "quipu_graph_thaw",
            "description": "Thaw a frozen graph: verify its archive pack, detach it, restore the full history into the local store under the same IRI, and reopen the graph for writes. The pack file is kept on disk; the freeze registry row is closed, never deleted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "IRI of the frozen graph" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" },
                    "actor": { "type": "string", "description": "Who is thawing" }
                },
                "required": ["graph", "timestamp"]
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
    ]
}
