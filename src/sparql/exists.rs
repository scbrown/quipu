//! ASK evaluation: answer existence without materialising every solution.
//!
//! Split from `pattern.rs` to keep that file under the 500-line ceiling, and
//! because "does a solution exist" is a different question from "what are the
//! solutions" — the whole point of aegis-yzn4vp is that answering the first by
//! computing the second cost 4.36 s where 4.2 ms would do.

use spargebra::algebra::GraphPattern;

use super::TemporalContext;
use super::pattern::{eval_pattern, limit_pushdown_safe};
use crate::Result;
use crate::Store;

/// Answer ASK without materialising every solution (aegis-yzn4vp).
///
/// `ASK` asks whether AT LEAST ONE solution exists, so it may stop at the first
/// row. It did not: the arm evaluated the whole pattern and then tested
/// `!rows.is_empty()`, which on the deployed 5.7 GB store made
/// `ASK { ?s ?p ?o }` take **4.36 s** (6.65 s measured on the server host) while
/// the equivalent `SELECT ?s WHERE { ?s ?p ?o } LIMIT 1` took **4.2 ms** — the
/// control proving the engine could already stop at the first row on the
/// identical pattern.
///
/// That was not merely slow. The obvious liveness probe for this store is an
/// unbounded `ASK`, so the cheapest question anyone asks was the most expensive
/// one to answer, and it consumed 4.4 s of a 10 s health-check budget while the
/// store was completely idle.
///
/// The limit is pushed under exactly the same `limit_pushdown_safe` gate the
/// SELECT path uses, so no pattern gains a shortcut that was not already judged
/// prefix-safe there. Anything else (FILTER, MINUS, ORDER BY, ...) falls back to
/// full evaluation and is unchanged.
pub fn eval_pattern_exists(
    store: &Store,
    pattern: &GraphPattern,
    ctx: &TemporalContext,
) -> Result<bool> {
    let ctx = match exists_row_limit(pattern, ctx) {
        None => ctx.clone(),
        Some(cap) => TemporalContext {
            row_limit: Some(cap),
            ..ctx.clone()
        },
    };
    let (rows, _) = eval_pattern(store, pattern, &ctx)?;
    Ok(!rows.is_empty())
}

/// The row cap `ASK` may evaluate under, or `None` to evaluate in full.
///
/// Split out so the SHORT-CIRCUIT ITSELF is assertable. A test that only checks
/// the boolean answer passes whether or not the cap is applied — it would be a
/// test of correctness wearing the name of a test of cost, which is the shape
/// this codebase keeps getting caught by. Asserting this function pins the
/// mechanism: remove the cap and it returns `None`, and the test fails.
pub(crate) fn exists_row_limit(pattern: &GraphPattern, ctx: &TemporalContext) -> Option<usize> {
    if !limit_pushdown_safe(pattern) {
        return None;
    }
    Some(ctx.row_limit.map_or(1, |existing| existing.min(1)))
}
