//! The MCP tool manifest — every tool definition quipu serves, split from
//! `mod.rs` to keep that module under the file-size ratchet.

use serde_json::Value as JsonValue;

mod core;
mod episodes;
mod policy_overlays;
mod projection;
mod schema;
mod search;

/// MCP tool definitions as JSON schemas for registration with Bobbin.
pub fn tool_definitions() -> Vec<JsonValue> {
    #[allow(unused_mut)]
    let mut defs = core::defs();
    defs.extend(episodes::defs());
    defs.extend(search::defs());
    defs.extend(schema::defs());
    defs.extend(projection::defs());
    defs.extend(policy_overlays::defs());

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
