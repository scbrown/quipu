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

impl Store {
    /// Build a [`ReadModel`] over one graph's current facts.
    ///
    /// Not cached on the store: Phase 3 (`quipu-syt`) decides residency and
    /// invalidation policy alongside the scope guard that makes consulting it
    /// safe. Building one here is explicit and costs a full scan.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the fact scan fails.
    pub fn build_read_model(&self, graph: i64) -> Result<ReadModel> {
        ReadModel::build(self, graph)
    }
}

#[cfg(test)]
mod tests;
