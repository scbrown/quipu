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

use std::collections::HashMap;

use crate::error::Result;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// The three access patterns `facts`' SQL indexes serve, held in memory.
///
/// Keyed by term id throughout — resolution to IRIs goes through the store's
/// term cache, so this holds no strings.
#[derive(Debug, Default)]
pub struct ReadModel {
    /// The graph this model covers. `0` is ROOT.
    graph: i64,
    /// entity → (attribute, value). Serves `<s> ?p ?o`.
    spo: HashMap<i64, Vec<(i64, Value)>>,
    /// attribute → (entity, value). Serves `?s <p> ?o`.
    pso: HashMap<i64, Vec<(i64, Value)>>,
    /// (attribute, value-bytes) → entities. Serves `?s <p> <o>` and `?s a <T>`.
    pos: HashMap<(i64, Vec<u8>), Vec<i64>>,
    /// value-bytes → (entity, attribute). Serves `?s ?p <o>`, which the other
    /// three cannot without a scan — SQL reaches for `idx_vaet` here, so a model
    /// without this index would be a REGRESSION on that shape rather than a
    /// speedup.
    osp: HashMap<Vec<u8>, Vec<(i64, i64)>>,
    /// Distinct triples held, maintained incrementally so `len` is not a scan.
    triples: usize,
}

impl ReadModel {
    /// Build from one graph's currently-valid asserted facts.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the fact scan fails.
    pub fn build(store: &Store, graph: i64) -> Result<Self> {
        let mut model = Self {
            graph,
            ..Default::default()
        };
        for fact in store.current_facts_in_graph(graph)? {
            model.insert(fact.entity, fact.attribute, &fact.value);
        }
        Ok(model)
    }

    /// The graph this model covers.
    #[must_use]
    pub fn graph(&self) -> i64 {
        self.graph
    }

    /// Distinct triples held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.triples
    }

    /// Whether the model holds no triples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triples == 0
    }

    /// `<s> ?p ?o` — every (predicate, object) for a subject.
    #[must_use]
    pub fn by_subject(&self, entity: i64) -> &[(i64, Value)] {
        self.spo.get(&entity).map_or(&[], Vec::as_slice)
    }

    /// `?s <p> ?o` — every (subject, object) for a predicate.
    #[must_use]
    pub fn by_predicate(&self, attribute: i64) -> &[(i64, Value)] {
        self.pso.get(&attribute).map_or(&[], Vec::as_slice)
    }

    /// `?s <p> <o>` — every subject with this exact predicate and object. The
    /// build side of a hash join, and what makes `?s a <Type>` a lookup rather
    /// than a scan.
    #[must_use]
    pub fn by_predicate_object(&self, attribute: i64, value: &Value) -> &[i64] {
        self.pos
            .get(&(attribute, value.to_bytes()))
            .map_or(&[], Vec::as_slice)
    }

    /// `?s ?p <o>` — every (subject, predicate) pointing at this object.
    #[must_use]
    pub fn by_object(&self, value: &Value) -> &[(i64, i64)] {
        self.osp.get(&value.to_bytes()).map_or(&[], Vec::as_slice)
    }

    /// Every triple held, as `(entity, attribute, value)`. The unbound-pattern
    /// fallback, and O(store) exactly as the SQL scan it replaces is.
    pub fn iter_triples(&self) -> impl Iterator<Item = (i64, i64, &Value)> {
        self.spo
            .iter()
            .flat_map(|(e, entries)| entries.iter().map(move |(a, v)| (*e, *a, v)))
    }

    /// Whether a specific triple is present.
    #[must_use]
    pub fn contains(&self, entity: i64, attribute: i64, value: &Value) -> bool {
        self.by_predicate_object(attribute, value).contains(&entity)
    }

    /// Apply one committed datum.
    ///
    /// [`Op::Assert`] inserts; [`Op::Retract`] and [`Op::Tombstone`] remove.
    /// **Removal is the half that earns the tests** — an index that only ever
    /// appends would answer with facts the store has retracted, and would do it
    /// silently.
    ///
    /// A datum carrying a `valid_to` is already closed and is therefore not
    /// current, so it is skipped rather than inserted: `build` loads
    /// `valid_to IS NULL`, and an incremental path that disagreed with the
    /// build would make the two diverge over time.
    pub fn apply(&mut self, datum: &Datum) {
        match datum.op {
            Op::Assert => {
                if datum.valid_to.is_none() {
                    self.insert(datum.entity, datum.attribute, &datum.value);
                }
            }
            Op::Retract | Op::Tombstone => {
                self.remove(datum.entity, datum.attribute, &datum.value);
            }
        }
    }

    /// Apply a batch of committed datums in order.
    pub fn apply_all(&mut self, datums: &[Datum]) {
        for datum in datums {
            self.apply(datum);
        }
    }

    /// Insert a triple into all three indexes, idempotently.
    ///
    /// Idempotence matters: the same `(e, a, v)` can be asserted twice (a
    /// re-ingest, an overlay echoing a root fact), and duplicate index entries
    /// would make a join emit duplicate rows that SQL's `SELECT DISTINCT` never
    /// produced.
    fn insert(&mut self, entity: i64, attribute: i64, value: &Value) {
        let key = (attribute, value.to_bytes());
        let subjects = self.pos.entry(key).or_default();
        if subjects.contains(&entity) {
            return;
        }
        subjects.push(entity);
        self.osp
            .entry(value.to_bytes())
            .or_default()
            .push((entity, attribute));
        self.spo
            .entry(entity)
            .or_default()
            .push((attribute, value.clone()));
        self.pso
            .entry(attribute)
            .or_default()
            .push((entity, value.clone()));
        self.triples += 1;
    }

    /// Remove a triple from all three indexes.
    ///
    /// Emptied buckets are dropped rather than left behind: a store that churns
    /// through predicates would otherwise accumulate empty `Vec`s forever, and
    /// an empty bucket is indistinguishable from an absent one to every reader.
    fn remove(&mut self, entity: i64, attribute: i64, value: &Value) {
        let key = (attribute, value.to_bytes());
        let Some(subjects) = self.pos.get_mut(&key) else {
            return;
        };
        let Some(pos) = subjects.iter().position(|s| *s == entity) else {
            return;
        };
        subjects.swap_remove(pos);
        if subjects.is_empty() {
            self.pos.remove(&key);
        }

        if let Some(entries) = self.spo.get_mut(&entity) {
            if let Some(i) = entries
                .iter()
                .position(|(a, v)| *a == attribute && v == value)
            {
                entries.swap_remove(i);
            }
            if entries.is_empty() {
                self.spo.remove(&entity);
            }
        }
        if let Some(entries) = self.pso.get_mut(&attribute) {
            if let Some(i) = entries.iter().position(|(e, v)| *e == entity && v == value) {
                entries.swap_remove(i);
            }
            if entries.is_empty() {
                self.pso.remove(&attribute);
            }
        }
        let obj_key = value.to_bytes();
        if let Some(entries) = self.osp.get_mut(&obj_key) {
            if let Some(i) = entries
                .iter()
                .position(|(e, a)| *e == entity && *a == attribute)
            {
                entries.swap_remove(i);
            }
            if entries.is_empty() {
                self.osp.remove(&obj_key);
            }
        }
        self.triples -= 1;
    }

    /// Every triple held, sorted — the comparison surface for the differential
    /// tests that prove an incrementally-updated model equals a rebuilt one.
    #[must_use]
    pub fn triples_sorted(&self) -> Vec<(i64, i64, Vec<u8>)> {
        let mut out: Vec<(i64, i64, Vec<u8>)> = self
            .spo
            .iter()
            .flat_map(|(e, entries)| entries.iter().map(move |(a, v)| (*e, *a, v.to_bytes())))
            .collect();
        out.sort_unstable();
        out
    }
}

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
        && store.read_model_affordable(crate::schema::ROOT_GRAPH)
        && ctx.valid_at.is_none()
        && ctx.as_of_tx.is_none()
        && ctx.named_dataset.is_none()
        && ctx.graph.is_root_default()
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
        if self.read_model.borrow().is_some() {
            return true; // already paid for
        }
        self.current_fact_count(graph)
            .is_ok_and(|n| n <= self.read_model_max_triples.get())
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
        if self.read_model.borrow().is_none() {
            let built = ReadModel::build(self, crate::schema::ROOT_GRAPH)?;
            *self.read_model.borrow_mut() = Some(built);
        }
        Ok(std::cell::Ref::map(self.read_model.borrow(), |m| {
            m.as_ref().expect("just built")
        }))
    }

    /// Drop the resident read model.
    pub(crate) fn invalidate_read_model(&self) {
        *self.read_model.borrow_mut() = None;
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
    pub(crate) fn maintain_read_model(&self, graph: i64, effective: Option<&[Datum]>) {
        let mut slot = self.read_model.borrow_mut();
        let Some(model) = slot.as_mut() else {
            return;
        };
        if model.graph() != graph {
            return;
        }
        match effective {
            Some(datums) => model.apply_all(datums),
            None => *slot = None,
        }
    }

    /// Whether a resident model is currently built. Test surface.
    #[must_use]
    pub fn read_model_is_resident(&self) -> bool {
        self.read_model.borrow().is_some()
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
