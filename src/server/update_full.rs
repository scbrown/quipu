//! The whole-store copy an unsliceable `/update` needs, bounded (aegis-11rwfs).
//!
//! `update_slice::plan` returns `Plan::Full` when it cannot name the facts an
//! update reads: an open subject AND an open predicate, e.g.
//! `DELETE WHERE { GRAPH <g> { ?s ?p ?o } }`. Evaluation then copies every
//! current fact of every graph into an in-memory store, so its memory grows
//! with the whole store, not with the update. On 2026-10-08 one such cleanup
//! of 29 triples reached the 8 GiB cap and force-recycled the production
//! server. Past [`max_facts`] the copy now stops and the update is refused
//! with advice, instead.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use oxigraph::{model::GraphName, store::Store as OxStore};

use super::super::base::AppError;

/// Default ceiling on facts copied for one full-path update. No production
/// update completed a full copy in the 24 h measured before this bound, so
/// legitimate traffic stays far below it; 250k quads is a few hundred MB.
pub(crate) const DEFAULT_MAX_FACTS: usize = 250_000;

static MAX_FACTS: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_FACTS);

/// Full-path updates STARTED. Counted before the copy, so an update that dies
/// in it is still visible (the old counter ran after the copy and missed the
/// one that killed the server).
pub(super) static STARTED: AtomicU64 = AtomicU64::new(0);
/// Full-path updates refused at the ceiling.
pub(super) static REFUSED: AtomicU64 = AtomicU64::new(0);

/// Set the ceiling (0 = unbounded). Called once at startup from config.
pub(crate) fn set_max_facts(limit: usize) {
    MAX_FACTS.store(limit, Ordering::Relaxed);
}

pub(crate) fn max_facts() -> usize {
    MAX_FACTS.load(Ordering::Relaxed)
}

/// `quipu_sparql_update_full_copy_total{outcome}`: copies started (counted
/// before the copy, so one that dies in it still shows) and refused.
pub(super) fn render(out: &mut String) {
    use std::fmt::Write as _;
    out.push_str(
        "# HELP quipu_sparql_update_full_copy_total Whole-store copies for unsliceable updates, by outcome.\n\
         # TYPE quipu_sparql_update_full_copy_total counter\n",
    );
    for (outcome, counter) in [("started", &STARTED), ("refused", &REFUSED)] {
        let _ = writeln!(
            out,
            "quipu_sparql_update_full_copy_total{{outcome=\"{outcome}\"}} {}",
            counter.load(Ordering::Relaxed)
        );
    }
}

/// Copy every current fact of `graphs` into `ox`, refusing past `limit`
/// (0 = unbounded). Production passes [`max_facts`].
pub(super) fn copy(
    store: &quipu::Store,
    ox: &OxStore,
    graphs: &[(i64, GraphName)],
    reason: &str,
    limit: usize,
) -> Result<(), AppError> {
    STARTED.fetch_add(1, Ordering::Relaxed);
    // Count FIRST: loading one graph's facts already materialises the whole
    // graph, so a check during the copy fires only after that cost is paid
    // (measured: +481 MB on a 773k-fact board before refusing). The per-graph
    // COUNT is the store's documented affordability check.
    let mut total = 0usize;
    if limit > 0 {
        for (graph_id, _) in graphs {
            total = total.saturating_add(store.current_fact_count(*graph_id)?);
        }
    }
    let mut copied = 0usize;
    for (graph_id, graph) in graphs {
        if limit > 0 && total > limit {
            break;
        }
        for fact in store.current_facts_in_graph(*graph_id)? {
            copied += 1;
            if limit > 0 && copied > limit {
                total = copied;
                break;
            }
            super::insert_fact(store, ox, fact.entity, fact.attribute, &fact.value, graph)?;
        }
    }
    if limit > 0 && total > limit {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return Err(quipu::Error::InvalidValue(format!(
            "this update cannot be sliced ({reason}): it reads an open subject \
                     with an open predicate, which needs a copy of the whole store, and \
                     the store holds more than {limit} facts. Rewrite it with constant \
                     subjects (for example DELETE DATA with the explicit triples, or one \
                     subject per pattern), or raise server.update_full_copy_max_facts"
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "update_full_tests.rs"]
mod tests;
