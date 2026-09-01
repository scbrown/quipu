//! Compact in-memory indexes over graph fact pointers.

use crate::error::Result;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};
use rusqlite::params;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

type FactId = u32;

#[derive(Debug)]
enum ValueSlot {
    Inline(Value),
    LazyRow(i64),
}

#[derive(Debug)]
struct FactPointer {
    entity: i64,
    attribute: i64,
    value_fingerprint: u64,
    value: ValueSlot,
}

impl FactPointer {
    fn value(&self, store: &Store) -> Result<Value> {
        match &self.value {
            ValueSlot::Inline(value) => Ok(value.clone()),
            ValueSlot::LazyRow(rowid) => {
                let mut stmt = store.prepare("SELECT v FROM facts WHERE rowid = ?1")?;
                let bytes: Vec<u8> = stmt.query_row(params![rowid], |row| row.get(0))?;
                Value::from_bytes(&bytes)
            }
        }
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn resident(value: &Value) -> bool {
    matches!(
        value,
        Value::Ref(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_)
    )
}

/// Four access indexes containing only compact pointers into one fact arena.
#[derive(Debug, Default)]
pub struct ReadModel {
    graph: i64,
    facts: Vec<Option<FactPointer>>,
    spo: HashMap<i64, Vec<FactId>>,
    pso: HashMap<i64, Vec<FactId>>,
    pos: HashMap<(i64, u64), Vec<FactId>>,
    osp: HashMap<u64, Vec<FactId>>,
    triples: usize,
    built_at_tx: i64,
}

impl ReadModel {
    /// Build from one graph's current facts. Heap-backed values are decoded
    /// once to classify them, then released; only `SQLite` row pointers remain.
    pub fn build(store: &Store, graph: i64) -> Result<Self> {
        let built_at_tx = store.latest_tx_id()?;
        let mut model = Self {
            graph,
            built_at_tx,
            ..Default::default()
        };
        let mut stmt = store.prepare(
            "SELECT MIN(rowid), e, a, v FROM facts \
             WHERE op = 1 AND valid_to IS NULL AND g = ?1 \
             GROUP BY e, a, v ORDER BY e, a",
        )?;
        let mut rows = stmt.query(params![graph])?;
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let entity: i64 = row.get(1)?;
            let attribute: i64 = row.get(2)?;
            let bytes: Vec<u8> = row.get(3)?;
            let value = Value::from_bytes(&bytes)?;
            let slot = if resident(&value) {
                ValueSlot::Inline(value)
            } else {
                ValueSlot::LazyRow(rowid)
            };
            model.insert_pointer(entity, attribute, fingerprint(&bytes), slot);
        }
        Ok(model)
    }

    #[must_use]
    pub fn built_at_tx(&self) -> i64 {
        self.built_at_tx
    }
    pub(crate) fn set_built_at_tx(&mut self, tx_id: i64) {
        self.built_at_tx = tx_id;
    }
    #[must_use]
    pub fn graph(&self) -> i64 {
        self.graph
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.triples
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triples == 0
    }

    /// Number of heap-backed values represented only by `SQLite` row pointers.
    #[must_use]
    pub fn lazy_value_count(&self) -> usize {
        self.facts
            .iter()
            .filter_map(Option::as_ref)
            .filter(|fact| matches!(fact.value, ValueSlot::LazyRow(_)))
            .count()
    }

    fn fact(&self, id: FactId) -> &FactPointer {
        self.facts[id as usize]
            .as_ref()
            .expect("indexes never retain removed fact ids")
    }

    pub fn by_subject(&self, store: &Store, entity: i64) -> Result<Vec<(i64, Value)>> {
        self.spo.get(&entity).map_or_else(
            || Ok(Vec::new()),
            |ids| {
                ids.iter()
                    .map(|id| {
                        let fact = self.fact(*id);
                        Ok((fact.attribute, fact.value(store)?))
                    })
                    .collect()
            },
        )
    }

    pub fn by_predicate(&self, store: &Store, attribute: i64) -> Result<Vec<(i64, Value)>> {
        self.pso.get(&attribute).map_or_else(
            || Ok(Vec::new()),
            |ids| {
                ids.iter()
                    .map(|id| {
                        let fact = self.fact(*id);
                        Ok((fact.entity, fact.value(store)?))
                    })
                    .collect()
            },
        )
    }

    pub fn by_predicate_object(
        &self,
        store: &Store,
        attribute: i64,
        value: &Value,
    ) -> Result<Vec<i64>> {
        let fp = fingerprint(&value.to_bytes());
        self.pos.get(&(attribute, fp)).map_or_else(
            || Ok(Vec::new()),
            |ids| {
                ids.iter().try_fold(Vec::new(), |mut out, id| {
                    let fact = self.fact(*id);
                    if fact.value(store)? == *value {
                        out.push(fact.entity);
                    }
                    Ok(out)
                })
            },
        )
    }

    pub fn by_object(&self, store: &Store, value: &Value) -> Result<Vec<(i64, i64)>> {
        let fp = fingerprint(&value.to_bytes());
        self.osp.get(&fp).map_or_else(
            || Ok(Vec::new()),
            |ids| {
                ids.iter().try_fold(Vec::new(), |mut out, id| {
                    let fact = self.fact(*id);
                    if fact.value(store)? == *value {
                        out.push((fact.entity, fact.attribute));
                    }
                    Ok(out)
                })
            },
        )
    }

    pub fn triples(&self, store: &Store) -> Result<Vec<(i64, i64, Value)>> {
        self.facts
            .iter()
            .filter_map(Option::as_ref)
            .map(|fact| Ok((fact.entity, fact.attribute, fact.value(store)?)))
            .collect()
    }

    pub fn contains(
        &self,
        store: &Store,
        entity: i64,
        attribute: i64,
        value: &Value,
    ) -> Result<bool> {
        Ok(self
            .by_predicate_object(store, attribute, value)?
            .contains(&entity))
    }

    pub fn apply(&mut self, store: &Store, datum: &Datum) {
        match datum.op {
            Op::Assert if datum.valid_to.is_none() => self.insert(store, datum),
            Op::Assert => {}
            Op::Retract | Op::Tombstone => self.remove(store, datum),
        }
    }

    pub fn apply_all(&mut self, store: &Store, datums: &[Datum]) {
        for datum in datums {
            self.apply(store, datum);
        }
    }

    fn insert(&mut self, store: &Store, datum: &Datum) {
        let bytes = datum.value.to_bytes();
        let fp = fingerprint(&bytes);
        if self.pos.get(&(datum.attribute, fp)).is_some_and(|ids| {
            ids.iter().any(|id| {
                let fact = self.fact(*id);
                fact.entity == datum.entity
                    && fact.value(store).is_ok_and(|value| value == datum.value)
            })
        }) {
            return;
        }
        let slot = if resident(&datum.value) {
            ValueSlot::Inline(datum.value.clone())
        } else {
            // Committed maintenance runs after the row exists. Keep the literal
            // out of memory just like a full build; the fallback preserves
            // correctness if a standalone caller applies an uncommitted datum.
            let rowid = store
                .prepare(
                    "SELECT rowid FROM facts WHERE e = ?1 AND a = ?2 AND v = ?3 \
                     AND g = ?4 AND op = 1 AND valid_to IS NULL \
                     ORDER BY rowid DESC LIMIT 1",
                )
                .ok()
                .and_then(|mut stmt| {
                    stmt.query_row(
                        params![datum.entity, datum.attribute, bytes, self.graph],
                        |row| row.get(0),
                    )
                    .ok()
                });
            rowid.map_or_else(
                || ValueSlot::Inline(datum.value.clone()),
                ValueSlot::LazyRow,
            )
        };
        self.insert_pointer(datum.entity, datum.attribute, fp, slot);
    }

    fn insert_pointer(&mut self, entity: i64, attribute: i64, fp: u64, value: ValueSlot) {
        let id = FactId::try_from(self.facts.len()).expect("read model exceeds u32 fact ids");
        self.facts.push(Some(FactPointer {
            entity,
            attribute,
            value_fingerprint: fp,
            value,
        }));
        self.spo.entry(entity).or_default().push(id);
        self.pso.entry(attribute).or_default().push(id);
        self.pos.entry((attribute, fp)).or_default().push(id);
        self.osp.entry(fp).or_default().push(id);
        self.triples += 1;
    }

    fn remove(&mut self, store: &Store, datum: &Datum) {
        let fp = fingerprint(&datum.value.to_bytes());
        let Some(id) = self.pos.get(&(datum.attribute, fp)).and_then(|ids| {
            ids.iter().copied().find(|id| {
                let fact = self.fact(*id);
                fact.entity == datum.entity
                    && fact.value(store).is_ok_and(|value| value == datum.value)
            })
        }) else {
            return;
        };
        let fact = self.fact(id);
        let (entity, attribute, value_fingerprint) =
            (fact.entity, fact.attribute, fact.value_fingerprint);
        Self::remove_id(&mut self.spo, &entity, id);
        Self::remove_id(&mut self.pso, &attribute, id);
        Self::remove_id(&mut self.pos, &(attribute, value_fingerprint), id);
        Self::remove_id(&mut self.osp, &value_fingerprint, id);
        self.facts[id as usize] = None;
        self.triples -= 1;
    }

    fn remove_id<K: Eq + Hash>(map: &mut HashMap<K, Vec<FactId>>, key: &K, id: FactId) {
        let empty = if let Some(ids) = map.get_mut(key) {
            if let Some(pos) = ids.iter().position(|candidate| *candidate == id) {
                ids.swap_remove(pos);
            }
            ids.is_empty()
        } else {
            false
        };
        if empty {
            map.remove(key);
        }
    }

    pub fn triples_sorted(&self, store: &Store) -> Result<Vec<(i64, i64, Vec<u8>)>> {
        let mut out: Vec<_> = self
            .triples(store)?
            .into_iter()
            .map(|(e, a, v)| (e, a, v.to_bytes()))
            .collect();
        out.sort_unstable();
        Ok(out)
    }
}
