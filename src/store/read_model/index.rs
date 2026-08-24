//! The in-memory index itself: `ReadModel` and nothing else.
//!
//! Split out of `read_model.rs` because that file passed the repo's size guard
//! (aegis-98gai grew it to 601 lines against a 522 baseline). The seam is not
//! arbitrary — the two halves answer different questions and have different
//! reasons to change:
//!
//!   * HERE: what an in-memory triple index holds and how it is maintained.
//!     Pure data structure; knows about `Datum` and `Value`, not about SQL,
//!     transactions or freshness.
//!   * `read_model.rs`: WHEN a resident model may be trusted — the
//!     applicability guard, the size budget, and the freshness check against
//!     the database.
//!
//! Raising the baseline instead would have been the easier move and the wrong
//! one: the guard exists to stop exactly the growth that produced it.

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
    /// The store's `latest_tx_id()` when this model was built or last
    /// maintained. A model whose stamp no longer matches the database is STALE
    /// and must be rebuilt before it answers anything (aegis-98gai).
    built_at_tx: i64,
}

impl ReadModel {
    /// Build from one graph's currently-valid asserted facts.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the fact scan fails.
    pub fn build(store: &Store, graph: i64) -> Result<Self> {
        // STAMP BEFORE SCANNING, never after. Between the two there is no lock
        // held over the database, so a write that lands mid-scan would other-
        // wise be stamped as included when the scan may have missed it. Taking
        // the stamp first can only make the model look staler than it is, which
        // costs a rebuild; the other order silently serves a gap.
        let built_at_tx = store.latest_tx_id()?;
        let mut model = Self {
            graph,
            built_at_tx,
            ..Default::default()
        };
        for fact in store.current_facts_in_graph(graph)? {
            model.insert(fact.entity, fact.attribute, &fact.value);
        }
        Ok(model)
    }

    /// The `latest_tx_id()` this model is current as of.
    #[must_use]
    pub fn built_at_tx(&self) -> i64 {
        self.built_at_tx
    }

    /// Re-stamp after an incremental maintain, so a write the model DID absorb
    /// does not force the next reader to rebuild it.
    pub(crate) fn set_built_at_tx(&mut self, tx_id: i64) {
        self.built_at_tx = tx_id;
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
