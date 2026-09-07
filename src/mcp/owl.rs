//! MCP tool implementation for OWL ontology management.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::owl::Ontology;
use crate::store::Store;

/// MCP tool: `quipu_load_ontology` -- Load, list, or remove OWL ontologies.
///
/// Actions:
/// - `load`: Parse and store an OWL ontology from Turtle, then materialize.
/// - `list`: List stored ontologies.
/// - `remove`: Remove a stored ontology by name.
/// - `materialize`: Re-derive entailments from the ALREADY-loaded ontologies,
///   without loading anything. The scheduled half of the aegis-2s6xpb
///   mitigation — see below.
pub fn tool_load_ontology(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    match action {
        "load" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;
            let turtle = input
                .get("turtle")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'turtle' parameter".into()))?;
            let now = crate::time::now_iso();
            let timestamp = input
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or(&now);

            // Parse and validate the ontology.
            let ontology = Ontology::from_turtle(turtle)?;
            let summary = ontology.axiom_summary();

            // Persist.
            store.load_ontology(name, turtle, timestamp)?;
            // The write gate caches the combined ontology; a newly loaded axiom
            // must bite on the NEXT write, not after a restart (aegis-bmqup).
            store.invalidate_owl_cache();

            // Materialize entailments.
            let report = ontology.materialize(store, timestamp)?;

            Ok(serde_json::json!({
                "action": "load",
                "name": name,
                "axioms": summary,
                "materialized": {
                    "subclass_inferences": report.subclass_inferences,
                    "sub_property_inferences": report.sub_property_inferences,
                    "inverse_inferences": report.inverse_inferences,
                    "symmetric_inferences": report.symmetric_inferences,
                    "equivalent_class_inferences": report.equivalent_class_inferences,
                    "domain_range_inferences": report.domain_range_inferences,
                    "total": report.total,
                }
            }))
        }
        "list" => {
            let ontologies = store.list_ontologies()?;
            let items: Vec<JsonValue> = ontologies
                .iter()
                .map(|(name, turtle, loaded_at)| {
                    let axiom_summary = Ontology::from_turtle(turtle).map_or_else(
                        |_| serde_json::json!({"error": "parse failed"}),
                        |o| o.axiom_summary(),
                    );
                    serde_json::json!({
                        "name": name,
                        "loaded_at": loaded_at,
                        "axioms": axiom_summary,
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "action": "list",
                "ontologies": items,
                "count": items.len(),
            }))
        }
        // ── THE SCHEDULED HALF OF THE aegis-2s6xpb MITIGATION (aegis-v3gf6u) ──
        //
        // The mitigation was "reactive OFF **plus** scheduled materialisation".
        // Only the first half shipped. `quipu_owl_reactive_materialize=false`
        // means `ReactiveOwl` is never registered, so `owl_materialize.rs` is
        // unreachable from the write path — and nothing replaced it. Measured
        // 2026-09-07 (muldoon, aegis-yro9m): no timer and no cron on the host
        // matching materiali/owl, and the "reactive OWL materialization enabled"
        // banner appears 0 times in server.log while its SIBLING banner three
        // lines above appears 4+13 times, which is what proves the channel works
        // and the branch never ran.
        //
        // So OWL entailment was DARK, not deferred, and for EVERY family — not
        // only `owl:sameAs`. sameAs was merely the family that surfaced, because
        // it is the one with no pre-existing materialised backlog from earlier
        // ontology loads to disguise the gap.
        //
        // WHY THIS ACTION RATHER THAN A NEW ROUTE OR A CLI. `/ontology` is
        // already `rw_handler!` and in WRITE_ENDPOINTS, so this inherits the
        // bearer gate instead of opening a second authenticated surface. And it
        // runs INSIDE the server process, which matters: a separate CLI process
        // would contend for the store's SQLite handle and, worse, leave the
        // server's own `owl_cache` and read-models describing a store that had
        // changed underneath them.
        //
        // WHY IT MATERIALISES THE COMBINED ONTOLOGY. `ensure_owl_cache` joins
        // every stored ontology into one document, which is what the write gate
        // already reasons over. Materialising them one at a time would derive
        // strictly less: an axiom in ontology A over a class declared in B is
        // invisible to either alone.
        //
        // Reactive stays OFF and is not up for reversal. The observer ran a full
        // -store scan per write — 641,803 facts, ~2.3 s each against ~29
        // writes/min — so the backlog presented as ~70 MiB/min of growth and
        // 134-137% CPU. It reads as a leak and is not one: it is unbounded queued
        // work, and it is the OOM the parent bead is about. Deriving the same
        // facts on a cadence is the same closure without the per-write scan.
        "materialize" => {
            let now = crate::time::now_iso();
            let timestamp = input
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or(&now);

            store.ensure_owl_cache()?;
            let Some(ontology) = store.owl_cache.as_deref().cloned() else {
                // NOT an error, and NOT silently reported as success either. A
                // store with no ontology loaded has nothing to entail, and a
                // scheduler must be able to tell that apart from "it ran and
                // derived nothing", which is the state this whole bead is about.
                return Ok(serde_json::json!({
                    "action": "materialize",
                    "ontologies": 0,
                    "materialized": null,
                    "note": "no ontologies are loaded; nothing to materialize"
                }));
            };
            let report = ontology.materialize(store, timestamp)?;
            Ok(serde_json::json!({
                "action": "materialize",
                "ontologies": store.list_ontologies()?.len(),
                "materialized": {
                    "subclass_inferences": report.subclass_inferences,
                    "sub_property_inferences": report.sub_property_inferences,
                    "inverse_inferences": report.inverse_inferences,
                    "symmetric_inferences": report.symmetric_inferences,
                    "equivalent_class_inferences": report.equivalent_class_inferences,
                    "same_as_inferences": report.same_as_inferences,
                    "domain_range_inferences": report.domain_range_inferences,
                    "total": report.total,
                }
            }))
        }
        "remove" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidValue("missing 'name' parameter".into()))?;
            let removed = store.remove_ontology(name)?;
            // Same reason as load: a retired axiom must stop rejecting writes
            // immediately, or the gate enforces an ontology nobody can see.
            store.invalidate_owl_cache();
            Ok(serde_json::json!({
                "action": "remove",
                "name": name,
                "removed": removed,
            }))
        }
        _ => Err(Error::InvalidValue(format!(
            "unknown action '{action}'; use 'load', 'materialize', 'list', or 'remove'"
        ))),
    }
}
