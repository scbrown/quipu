//! The write gates `/knot` enforces, applied to what a SPARQL `/update` asserts
//! (aegis-1hfyk5).
//!
//! `/update` evaluated the request with Oxigraph and transacted the diff with no
//! gate at all. So on a path seeds uses for the shared board, the closed
//! vocabulary and every loaded shape were skipped. Measured on 674f3700 (ian,
//! aegis-bqgdr3 C2): an unknown `rdf:type` and a seed missing a required
//! property were refused by `/knot` and STORED by `/update`.
//!
//! The gates run on the ASSERTED quads, before any term is interned or any
//! transaction begins, so a refusal stores nothing. Same order and semantics as
//! `/knot`: the vocabulary first, then each destination graph's asserted triples
//! against the stored shapes with that graph's store context. The reject half
//! blocks; the emit half only queues `shacl.violation` events. Retractions are
//! not validated: `/knot` cannot retract, so there is no parity to keep, and a
//! deletion that leaves a node non-conforming is not caught here.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use oxigraph::model::{GraphName, Quad};

use super::super::base::AppError;

/// Refuse the update if what it asserts fails the vocabulary or the loaded
/// shapes. `Ok(())` means it may be transacted.
pub(super) fn enforce<'a>(
    store: &quipu::Store,
    asserted: impl Iterator<Item = &'a Quad>,
    actor: Option<&str>,
    source: &str,
) -> Result<(), AppError> {
    // Per destination graph: N-Triples of what lands there. N-Triples is valid
    // Turtle, which is what both gates take.
    let mut by_graph: BTreeMap<String, String> = BTreeMap::new();
    for quad in asserted {
        let key = match &quad.graph_name {
            GraphName::NamedNode(n) => n.as_str().to_owned(),
            _ => String::new(),
        };
        let _ = writeln!(
            by_graph.entry(key).or_default(),
            "{} {} {} .",
            quad.subject,
            quad.predicate,
            quad.object
        );
    }
    if by_graph.is_empty() {
        return Ok(());
    }
    let all: String = by_graph.values().map(String::as_str).collect();
    quipu::vocabulary::enforce_turtle(store, &all)?;
    shapes(store, &by_graph, actor, source)
}

#[cfg(feature = "shacl")]
fn shapes(
    store: &quipu::Store,
    by_graph: &BTreeMap<String, String>,
    actor: Option<&str>,
    source: &str,
) -> Result<(), AppError> {
    let Some(stored) = store.get_combined_shapes()? else {
        return Ok(());
    };
    let split = quipu::shacl::split_shapes_by_policy(&stored);
    let now = quipu::time::now_iso();
    for (iri, turtle) in by_graph {
        // A graph this update creates has no facts of its own yet; ROOT is its
        // whole context, as for a /knot into a fresh graph.
        let g = if iri.is_empty() {
            quipu::schema::ROOT_GRAPH
        } else {
            store.lookup(iri)?.unwrap_or(quipu::schema::ROOT_GRAPH)
        };
        let feedback = quipu::shacl_context::validate_with_store_context_in_graph(
            store,
            &split.reject,
            turtle,
            g,
        )?;
        if !feedback.conforms {
            let detail = feedback
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
                .join("; ");
            let reason = format!(
                "SHACL validation failed: {} violation(s): {detail}. No facts were written",
                feedback.violations
            );
            store.record_gate_refusal("shacl", &reason, g, actor, Some(source), &now);
            return Err(quipu::Error::InvalidValue(reason).into());
        }
        if split.has_emit {
            let observed = quipu::shacl_context::validate_with_store_context_in_graph(
                store,
                &split.emit,
                turtle,
                g,
            )?;
            quipu::shacl_context::queue_emit_violations(store, &observed);
        }
    }
    Ok(())
}

#[cfg(not(feature = "shacl"))]
fn shapes(
    _store: &quipu::Store,
    _by_graph: &BTreeMap<String, String>,
    _actor: Option<&str>,
    _source: &str,
) -> Result<(), AppError> {
    Ok(())
}
