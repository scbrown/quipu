//! Retraction and episode ingest.
//!
//! Split out of `definitions.rs` under the file-size ratchet (aegis-gf3j7). The
//! blocks are MOVED VERBATIM and their order is preserved, so `tool_definitions()`
//! returns exactly the Vec it returned before — the split is provable, not argued.

use serde_json::Value as JsonValue;

pub(super) fn defs() -> Vec<JsonValue> {
    vec![
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
    ]
}
