//! When a read model may be used at all — the applicability and affordability
//! guard.
//!
//! Split out of `read_model.rs` (aegis-niuav/CI file-size gate). These belong
//! together and apart from the index itself: `ReadModel` is the data structure,
//! while everything here answers the prior question — *may* this query be
//! answered from a resident model, and can the store *afford* to hold one.
//! `read_model_applicable` is the load-bearing scope guard; the affordability
//! checks are what it consults to decide, so separating them would have split a
//! single decision across two files.

use crate::error::Result;
use crate::store::Store;
use crate::store::read_model::ReadModel;

/// Default ceiling on a resident read model, in triples.
///
/// At the measured ~320 bytes/triple this is roughly 320 MB — generous for a
/// working graph, and a bound rather than unlimited growth on a store holding a
/// million episodes.
pub const DEFAULT_READ_MODEL_MAX_TRIPLES: usize = 1_000_000;

/// Whether a read answered from a [`ReadModel`] would be identical to one
/// answered from SQL.
///
/// This is the guard, and it is the load-bearing half of Phase 3 — the hash
/// join is the easy part. A `ReadModel` holds **currently-valid asserted facts
/// in ONE graph**, so every dimension outside that must reach SQL instead:
///
/// - `valid_at` / `as_of_tx` — the model holds no history at all. It was built
///   from `valid_to IS NULL`, so a time-travelling query it answered would
///   return the present and call it the past.
/// - Anything but the plain ROOT default graph. A `FROM`-redefined default, a
///   named graph, or `GRAPH ?g` all read a different fact set;
///   `GraphScope::is_root_default` already exists for exactly this narrowing,
///   and carries the same warning that these paths would otherwise silently
///   read `g = 0`.
/// - `FROM NAMED` — restricts which named graphs are visible, which the model
///   does not model.
/// - A write in progress. The write-time policy guard runs queries INSIDE the
///   open savepoint, against rows that are staged but not committed. The model
///   holds the pre-write state and would answer without them, so the guard
///   would judge a write against a store missing the facts that make it valid.
///   Suspending here rather than dropping the model is what lets the write
///   MAINTAIN it afterwards instead of forcing a rebuild.
/// - Attachments. `facts_source()` becomes a `UNION ALL` over composed layers
///   and `canonical_id` starts rewriting ids; the model is built from one
///   database and knows neither. Checking attachments covers aliasing too,
///   because `canonical_id` is the identity when there are none.
///
/// Overlay composition and tombstone resolution need no separate check: an
/// overlay is a named graph, so `is_root_default` already excludes it.
///
/// **A `false` here is not a performance bug.** A query that time-travels is
/// slower because it is a different question, not because an optimization is
/// missing.
#[must_use]
pub fn read_model_applicable(store: &Store, ctx: &crate::sparql::TemporalContext) -> bool {
    store.read_model_enabled()
        && !store.write_in_progress()
        && ctx.valid_at.is_none()
        && ctx.as_of_tx.is_none()
        && ctx.named_dataset.is_none()
        // Any SINGLE graph, not just ROOT (quipu-nip): the model answers one
        // graph's own facts — the same scope the SQL path's `g IN (…)` filter
        // reads — so a query scoped to a small derived graph gets the fast
        // path even when ROOT is past the budget. Unions and `GRAPH ?g` keep
        // SQL: a model holds one graph and no per-row g.
        && ctx
            .graph
            .single_graph()
            .is_some_and(|g| store.read_model_affordable(g))
        && !store.has_attachments()
}

impl Store {
    /// Current fact count in a graph — the cheap check that decides whether
    /// building a read model is affordable, without building one to find out.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the count fails.
    pub fn current_fact_count(&self, graph: i64) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM facts WHERE op = 1 AND valid_to IS NULL AND g = ?1",
            rusqlite::params![graph],
            |r| r.get(0),
        )?;
        Ok(usize::try_from(n).unwrap_or(usize::MAX))
    }

    /// Whether a read model for `graph` fits the configured budget.
    ///
    /// Checked with a `COUNT`, not by building one and measuring — on the
    /// stores where the answer is "no", building to find out is precisely the
    /// cost being avoided.
    #[must_use]
    pub fn read_model_affordable(&self, graph: i64) -> bool {
        let models = self.read_model.borrow();
        if models.contains_key(&graph) {
            return true; // already paid for
        }
        // The budget bounds the COMBINED resident size (quipu-nip): a second
        // graph's model is affordable only if it fits alongside what is
        // already held, so per-graph models cannot multiply past the ceiling.
        let resident: usize = models.values().map(ReadModel::len).sum();
        self.current_fact_count(graph)
            .is_ok_and(|n| n.saturating_add(resident) <= self.read_model_max_triples.get())
    }

    /// Ceiling on how many triples a resident read model may hold.
    #[must_use]
    pub fn read_model_max_triples(&self) -> usize {
        self.read_model_max_triples.get()
    }

    /// Cap the resident read model's size. Above this, queries use SQL.
    ///
    /// At the measured ~320 bytes/triple, the default of
    /// [`DEFAULT_READ_MODEL_MAX_TRIPLES`] is roughly 320 MB. A store past it
    /// keeps the SQL path — slower on joins, but that is the behaviour it
    /// already had, not a regression. Scoping the model to a distilled graph
    /// rather than the whole episode log is the real answer at that size
    /// (`quipu-nip`).
    pub fn set_read_model_max_triples(&self, max: usize) {
        self.read_model_max_triples.set(max);
    }
}
