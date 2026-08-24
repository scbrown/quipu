//! Schema proposals and the ask surface.
//!
//! Split out of `definitions.rs` under the file-size ratchet (aegis-gf3j7). The
//! blocks are MOVED VERBATIM and their order is preserved, so `tool_definitions()`
//! returns exactly the Vec it returned before — the split is provable, not argued.

use serde_json::Value as JsonValue;

pub(super) fn defs() -> Vec<JsonValue> {
    vec![
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
    ]
}
