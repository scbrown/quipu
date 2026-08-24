//! In-memory read model — three permutation indexes over one graph's current
//! facts.
//!
//! Design: `docs/design/in-memory-read-model.md`. Phase 2 (`quipu-d6x`) builds
//! the structure and its incremental-apply path; Phase 3 (`quipu-syt`) is what
//! routes `eval_bgp` through it. **Nothing consults this yet** — it is
//! deliberately inert until the scope guard in §5 of the design exists, because
//! a read model that answered outside its scope would be wrong in exactly the
//! dimensions Quipu exists to get right.
//!
//! # What it covers, and what it must never be asked
//!
//! [`ReadModel::build`] loads from `current_facts_in_graph`, which is
//! **currently-valid, asserted facts in ONE graph**. That is a strict subset of
//! what SPARQL can ask:
//!
//! | Dimension | In scope |
//! |---|---|
//! | Current facts in the built graph | yes |
//! | `valid_at` / `as_of_tx` time travel | **no** — no history is loaded |
//! | Overlay composition / tombstone resolution | **no** — a resolution rule, not a row filter |
//! | Attached databases | **no** — built from one database |
//!
//! The caller owns that check. This type cannot tell whether the question it
//! was handed is inside its scope, which is precisely why the guard belongs at
//! the call site and not here.
//!
//! # Why the term dictionary is not in here
//!
//! It lives on `Store` as [`crate::store::terms::TermCache`] (`quipu-yzf`),
//! shared by every read path rather than duplicated per model. The prototype in
//! `examples/mem_read_model.rs` carried its own because `Store` had none yet.

use crate::error::Result;
use crate::store::{Datum, Store};

mod index;
// Re-exported so `ReadModel` keeps its public path: this split is an internal
// tidy-up and must not move a type anyone imports.
pub use index::ReadModel;

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
    /// Build a [`ReadModel`] over one graph's current facts.
    ///
    /// Explicit and costs a full scan. For the resident one, see
    /// [`Self::read_model`].
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the fact scan fails.
    pub fn build_read_model(&self, graph: i64) -> Result<ReadModel> {
        ReadModel::build(self, graph)
    }

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

    /// The resident ROOT read model, built on first use.
    ///
    /// # Invalidation
    ///
    /// Dropped wholesale by [`Self::invalidate_read_model`] on every write,
    /// rather than updated in place with [`ReadModel::apply`]. That is
    /// deliberate for the first cut: a `transact` can close prior facts as a
    /// side effect of asserting new ones, and those closures are not in the
    /// caller's datum list, so an incremental path would need to observe what
    /// the write actually did rather than what it was asked to do. Dropping is
    /// correct by construction; incremental maintenance is `quipu-m9h`.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the build's fact scan fails.
    pub fn read_model(&self) -> Result<std::cell::Ref<'_, ReadModel>> {
        self.read_model_for(crate::schema::ROOT_GRAPH)
    }

    /// The resident read model for ONE graph, built on first use (quipu-nip).
    ///
    /// Each graph gets its own model; the combined size is bounded by
    /// [`Self::read_model_affordable`]'s budget check at the applicability
    /// guard, so a store with a large ROOT and a small derived graph holds
    /// only the derived graph resident.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the fact scan fails.
    pub fn read_model_for(&self, graph: i64) -> Result<std::cell::Ref<'_, ReadModel>> {
        // FRESHNESS IS CHECKED AGAINST THE DATABASE, not assumed from having
        // seen the writes (aegis-98gai). `maintain_read_model` keeps THIS
        // `Store`'s model current, which is sound for an embedder that writes
        // and reads through one handle — and the REST server is not that. Its
        // read pool is N SEPARATE `Store`s opened read-only on the same file
        // (`server.rs`, `Store::open_read_only` in a loop); a write goes through
        // the writer's handle, so the pooled models never hear about it and
        // nothing ever drops them. `invalidate_read_model` has exactly one call
        // site and it is `set_read_model_enabled(false)`.
        //
        // Only multi-pattern BGPs consult a model (`sparql/triple.rs`:
        // `patterns.len() >= 2`), so the symptom was silent and specific: a
        // freshly written node answered `<iri> a ?t` and `<iri> rdfs:label ?l`
        // with one row each and their CONJUNCTION with zero. Measured
        // 2026-08-24 on the deployed 0.3.24: exactly 4 of 264 `FailureMode`
        // nodes were join-invisible, all four written in the preceding ~40
        // minutes, deterministic over 8 repetitions — and 0 of 264 once the
        // process had cycled. A restart was the only thing clearing it.
        //
        // `latest_tx_id()` is a cheap indexed MAX on a rowid-keyed table and its
        // own doc comment already offers it for exactly this: "callers can use
        // it as a change-generation stamp for caches of derived read-side data:
        // rebuild when it moves, reuse when it hasn't." Checking the database
        // rather than trusting in-process bookkeeping makes correctness
        // independent of how many `Store` handles exist and who did the write.
        let latest = self.latest_tx_id()?;
        let since = match self.read_model.borrow().get(&graph) {
            None => None, // nothing resident
            Some(m) if m.built_at_tx() == latest => {
                return Ok(std::cell::Ref::map(self.read_model.borrow(), |m| {
                    m.get(&graph).expect("checked resident and current")
                }));
            }
            Some(m) => Some(m.built_at_tx()),
        };
        match since {
            // CATCH UP, DO NOT REBUILD. A rebuild is correct and costs a scan of
            // every current fact — 608 ms in release at 340k triples, measured —
            // and a pooled reader goes stale on EVERY write it did not perform.
            // Paying that per write on a store already over its request-second
            // ceiling (aegis-x5nr6) would trade a silent wrong answer for a
            // loud slow one. The delta is bounded by what actually changed.
            Some(built_at) => {
                let changes = self.facts_changed_since_in_graph(graph, built_at)?;
                let mut models = self.read_model.borrow_mut();
                if let Some(model) = models.get_mut(&graph) {
                    model.apply_all(&changes);
                    model.set_built_at_tx(latest);
                }
            }
            None => {
                let built = ReadModel::build(self, graph)?;
                self.read_model.borrow_mut().insert(graph, built);
            }
        }
        Ok(std::cell::Ref::map(self.read_model.borrow(), |m| {
            m.get(&graph).expect("just built")
        }))
    }

    /// Drop every resident read model.
    pub(crate) fn invalidate_read_model(&self) {
        self.read_model.borrow_mut().clear();
    }

    /// Whether a write currently holds an open savepoint, during which the
    /// model must not be consulted — see [`read_model_applicable`].
    #[must_use]
    pub(crate) fn write_in_progress(&self) -> bool {
        self.write_in_progress.get()
    }

    /// Mark a write as started or finished.
    pub(crate) fn set_write_in_progress(&self, on: bool) {
        self.write_in_progress.set(on);
    }

    /// Bring the resident model up to date after a committed write.
    ///
    /// Three outcomes, and the middle one is the point of `quipu-m9h`:
    ///
    /// - Nothing resident, or the write targeted a DIFFERENT graph: leave it.
    ///   A model over ROOT is genuinely unaffected by a write to graph 7,
    ///   because `current_facts_in_graph(0)` never sees those rows.
    /// - The write vouched for its complete change set: apply it. O(datums)
    ///   instead of a full rebuild.
    /// - It could not vouch (OWL inference, functional supersede): drop, and
    ///   let the next read rebuild. Correct by construction.
    pub(crate) fn maintain_read_model(&self, graph: i64, effective: Option<&[Datum]>, tx_id: i64) {
        let mut models = self.read_model.borrow_mut();
        // Only the WRITTEN graph's model is touched (quipu-nip): a model over
        // ROOT is genuinely unaffected by a write to graph 7, because
        // `current_facts_in_graph(0)` never sees those rows.
        let Some(model) = models.get_mut(&graph) else {
            return;
        };
        match effective {
            Some(datums) => {
                model.apply_all(datums);
                // Re-stamp, or the freshness check in `read_model_for` would
                // rebuild on the very next read and undo quipu-m9h.
                model.set_built_at_tx(tx_id);
            }
            None => {
                models.remove(&graph);
            }
        }
    }

    /// Whether any resident model is currently built. Test surface.
    #[must_use]
    pub fn read_model_is_resident(&self) -> bool {
        !self.read_model.borrow().is_empty()
    }

    /// Whether a resident model is built for this graph. Test surface.
    #[must_use]
    pub fn read_model_is_resident_for(&self, graph: i64) -> bool {
        self.read_model.borrow().contains_key(&graph)
    }

    /// Whether SPARQL may answer from the read model. **On by default.**
    #[must_use]
    pub fn read_model_enabled(&self) -> bool {
        self.read_model_enabled.get()
    }

    /// Turn the read-model fast path on or off for SPARQL evaluation.
    ///
    /// # Why this defaults to ON
    ///
    /// Measured on `examples/scale_bench.rs`, with no shape regressing:
    ///
    /// | Episodes | Point lookup | Type scan | 2-hop join |
    /// |---:|---|---|---|
    /// | 1,000 | 0.11 → 0.14 ms | 4.6 → 5.4 ms | 1,016 → **38 ms** |
    /// | 4,000 | 0.10 → 0.13 ms | 18.8 → 18.5 ms | 26,233 → **225 ms** |
    /// | 10,000 | 0.16 → 0.12 ms | 56.2 → 46.8 ms | 173,803 → **560 ms** |
    ///
    /// Three things had to be true before this could default on, and an earlier
    /// revision of this comment explained why it could not:
    ///
    /// - **Joins are hash joins now**, not a nested loop re-evaluating each
    ///   pattern per accumulated row. That loop was the O(n²), and making each
    ///   evaluation cheap only shrank its constant.
    /// - **Writes maintain the model** instead of dropping it, so the ordinary
    ///   write-then-read loop no longer pays a rebuild every time.
    /// - **Only multi-pattern BGPs use it.** A single pattern is what SQL is
    ///   already fast at, and routing it through the model made it pay to build
    ///   one — a 0.12 ms → 320 ms regression on the most common query shape.
    ///
    /// Size is bounded by [`Store::set_read_model_max_triples`]; past that
    /// ceiling queries keep the SQL path, which is slower on joins but is the
    /// behaviour they already had.
    pub fn set_read_model_enabled(&self, on: bool) {
        self.read_model_enabled.set(on);
        if !on {
            self.invalidate_read_model();
        }
    }
}

#[cfg(test)]
mod tests;
