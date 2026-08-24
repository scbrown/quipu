//! Policy, verdicts, co-occurrence and overlays.
//!
//! Split out of `definitions.rs` under the file-size ratchet (aegis-gf3j7). The
//! blocks are MOVED VERBATIM and their order is preserved, so `tool_definitions()`
//! returns exactly the Vec it returned before — the split is provable, not argued.

use serde_json::Value as JsonValue;

pub(super) fn defs() -> Vec<JsonValue> {
    vec![
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
    ]
}
