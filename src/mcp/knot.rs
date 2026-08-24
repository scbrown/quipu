//! MCP tool: `quipu_knot` -- bulk Turtle assertion with optional SHACL
//! validation, snapshot replacement, and (since the bobbin×quipu roadmap,
//! 2026-08-21) an optional named-graph target.
//!
//! ## Why `graph` exists here now, and why it is strict
//!
//! A `graph` parameter on `/knot` was previously a documented deliberate
//! refusal: an arbitrary write into a named committed graph would bypass the
//! committed/overlay class invariant. The camayoc ingress discipline made the
//! other side of that trade concrete — deterministic bulk loads must be able
//! to land in a registered trust plane, and the workaround (silently dropping
//! the key and writing to ROOT) produced facts that masqueraded at canonical
//! standing. The invariant survives by being strict about the target:
//!
//! - The graph IRI must already be interned AND registered `committed`
//!   (created via `graph_create` / `POST /graph/create`, which is where
//!   authority checks live). Unknown IRIs are an error, never interned —
//!   the permissive `/episode` idiom mints unregistered planes that can
//!   never be labelled (camayoc-s0h); this path refuses instead.
//! - Overlay-class graphs are refused: overlays are written through
//!   `overlay_write` only.
//! - Absent or empty `graph` targets ROOT, byte-identical to the old
//!   behavior.
//!
//! SHACL store-context repair is graph-aware (quipu-080): validation runs
//! against the resolved destination graph, so its repair context is the union
//! of that graph's committed type facts and ROOT's. A chunked write into a
//! named graph whose earlier chunks typed nodes in that graph sees those types
//! (the aegis-fp17f/aegis-sd5fj defect class stays fixed on plane-routed
//! writes), ROOT-held ontology types keep applying everywhere, and no third
//! graph's types leak in. See `docs/design/named-graphs.md`.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// Resolve an optional named-graph IRI to its `g` id for a bulk write.
///
/// `None`/empty → ROOT (0). Otherwise the IRI must already be interned and
/// registered as a `committed` graph; anything else is an error naming the
/// remedy.
fn resolve_committed_graph(store: &Store, graph: Option<&str>) -> Result<i64> {
    let Some(iri) = graph.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(0);
    };
    let Some(g) = store.lookup(iri)? else {
        return Err(Error::InvalidValue(format!(
            "unknown graph: {iri} — create and register it first via graph_create"
        )));
    };
    match store.graph_class(g)?.as_deref() {
        Some("committed") => Ok(g),
        Some("overlay") => Err(Error::InvalidValue(format!(
            "graph {iri} is an overlay — write through overlay_write, not knot"
        ))),
        Some(other) => Err(Error::InvalidValue(format!(
            "graph {iri} has unsupported class '{other}' for knot writes"
        ))),
        None => Err(Error::InvalidValue(format!(
            "graph {iri} is interned but not registered — register it via graph_create"
        ))),
    }
}

/// MCP tool: `quipu_knot` -- Assert facts with optional SHACL validation.
///
/// Input: `{ "turtle": "<data>", "timestamp": "...", "actor": "...",
///           "source": "...", "shapes": "<optional shapes turtle>",
///           "replace_snapshot": false, "snapshot": "<stable producer key>",
///           "graph": "<registered committed-graph IRI>" }`
/// Output: `{ "tx_id": N, "count": N }` or validation feedback on failure.
pub fn tool_knot(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let turtle = input
        .get("turtle")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'turtle' parameter".into()))?;

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(&now);

    let actor = input.get("actor").and_then(|v| v.as_str());
    let source = input.get("source").and_then(|v| v.as_str());
    let replace_snapshot = input
        .get("replace_snapshot")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let snapshot = input.get("snapshot").and_then(JsonValue::as_str);
    if replace_snapshot && snapshot.is_none_or(|s| s.trim().is_empty()) {
        return Err(Error::InvalidValue(
            "replace_snapshot requires a non-empty stable 'snapshot' producer key".into(),
        ));
    }
    let graph = resolve_committed_graph(store, input.get("graph").and_then(JsonValue::as_str))?;

    // SHACL validation: combine per-request shapes with stored shapes.
    let request_shapes = input.get("shapes").and_then(|v| v.as_str());
    let stored_shapes = store.get_combined_shapes()?;

    #[allow(unused_variables)]
    let combined_shapes = match (request_shapes, &stored_shapes) {
        (Some(req), Some(stored)) => Some(format!("{stored}\n\n{req}")),
        (Some(req), None) => Some(req.to_string()),
        (None, Some(stored)) => Some(stored.clone()),
        (None, None) => None,
    };

    // Validated WITH THE STORE AS CONTEXT (aegis-fp17f). `/knot` is the chunked
    // write path, so it is where payload-only `sh:class` did its damage: a
    // caller that splits one graph across several posts had every chunk judged
    // as if the others did not exist. See `shacl_context` for why the store
    // supplies only types, and only for nodes the payload already references.
    // Context is scoped to the RESOLVED DESTINATION graph unioned with ROOT
    // (quipu-080): earlier chunks of a plane-routed write typed their nodes in
    // that plane, not in ROOT.
    #[cfg(feature = "shacl")]
    if let Some(shapes) = &combined_shapes {
        let feedback = crate::shacl_context::validate_with_store_context_in_graph(
            store, shapes, turtle, graph,
        )?;
        if !feedback.conforms {
            // A gate refusal, even though this surface reports it as
            // `conforms: false` rather than an Err: record it on the audit
            // spine (camayoc-0d3). Metadata only — gate, destination graph,
            // actor/source, shape ids — never the refused Turtle body. The
            // datum count is unknowable without parsing (which would intern
            // terms for a write that is being refused), so it is 0 here.
            let reason = format!(
                "SHACL validation failed: {} violation(s): {}",
                feedback.violations,
                feedback
                    .results
                    .iter()
                    .take(3)
                    .map(|r| {
                        format!(
                            "{}: {} [{}] ({})",
                            r.severity,
                            r.message.as_deref().unwrap_or("no message"),
                            r.source_shape.as_deref().unwrap_or("?"),
                            r.focus_node
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            let refusal = crate::store::events::PendingRefusal {
                gate: "shacl",
                reason,
                refused_datums: 0,
            };
            store.record_refusal(
                &refusal,
                &store.graph_iri_of(graph),
                actor,
                source,
                timestamp,
            );
            let issues: Vec<JsonValue> = feedback
                .results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "severity": r.severity,
                        "focus_node": r.focus_node,
                        "component": r.component,
                        "path": r.path,
                        "value": r.value,
                        "source_shape": r.source_shape,
                        "message": r.message
                    })
                })
                .collect();
            return Ok(serde_json::json!({
                "conforms": false,
                "violations": feedback.violations,
                "warnings": feedback.warnings,
                "issues": issues,
                "hint": "propose a schema change via quipu_propose_schema_change"
            }));
        }
    }

    let (tx_id, count) = if replace_snapshot {
        let source_tag = format!("snapshot:{}", snapshot.unwrap());
        // Both the retraction plan and the transaction are scoped to the
        // resolved graph: a snapshot in graph G replaces only G's prior
        // facts under this producer key and leaves ROOT untouched.
        let mut datums = store.plan_source_retraction(&source_tag, graph)?;
        let mut assertions = crate::rdf::parse_rdf(
            store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            timestamp,
        )?;
        let count = assertions.len();
        datums.retain(|old| {
            !assertions.iter().any(|new| {
                old.entity == new.entity && old.attribute == new.attribute && old.value == new.value
            })
        });
        datums.append(&mut assertions);
        let tx_id = store.transact_to_graph(&datums, timestamp, actor, Some(&source_tag), graph)?;
        (tx_id, count)
    } else {
        crate::rdf::ingest_rdf_to_graph(
            store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            timestamp,
            actor,
            source,
            graph,
        )?
    };

    // VOCABULARY ADVISORY (aegis-7n1ya). `conforms: true` above means only that
    // no shape was VIOLATED — and a shape fires through sh:targetClass, so a type
    // no shape targets is untargeted and vacuously conformant. This response
    // therefore cannot otherwise distinguish "validated and fine" from "not
    // validated at all", and every caller reads it as the former. bobbin's chunk
    // snapshot is the case that proved it matters at machine scale (aegis-6noan):
    // a producer guarding on `conforms != false` would have minted the graph's
    // largest ungoverned class behind a clean success.
    //
    // Never blocks, never fails the write: the transaction is already committed
    // at this point, so any error here is swallowed by design and the field is
    // simply absent. An advisory that can break a successful write is worse than
    // no advisory.
    let vocabulary_hint = crate::vocabulary::sanctioned(store)
        .ok()
        .map(|v| crate::vocabulary::ungoverned_types_in_turtle(turtle, &v))
        .and_then(crate::vocabulary::hint_json);

    let mut response = serde_json::json!({
        "conforms": true,
        "tx_id": tx_id,
        "count": count,
        "snapshot": snapshot,
        "replaced": replace_snapshot
    });
    if let Some(hint) = vocabulary_hint {
        response["vocabulary_hint"] = hint;
    }
    Ok(response)
}
