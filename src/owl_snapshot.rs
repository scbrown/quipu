//! Scheduled derivation over a private, current-state snapshot.
//!
//! Snapshot construction and derivation must run on a read connection, outside
//! both the live writer mutex and write admission. History, vectors and unrelated
//! named graphs are deliberately not copied. Applying a proposal is separate so
//! callers can release writer admission between bounded batches.

use rusqlite::params;

use crate::error::{Error, Result};
use crate::store::inferred::{
    DERIVED_AS_OF_TX, PLANE_SOURCE, ROOT_INFERRED_GRAPH_IRI, SOURCE_KIND,
};
use crate::{Datum, MaterializeReport, Op, Store, Value};

/// Maximum number of proposed assertions applied by one writer acquisition.
pub const APPLY_BATCH: usize = 64;
/// Refuse oversized inputs instead of copying the entire historical database.
const MAX_PREMISES: usize = 2_000_000;
const MAX_PROPOSALS: usize = 100_000;

/// Private derivation state. Its term ids are never used directly in the live store.
pub struct Snapshot {
    scratch: Store,
    identity: String,
    ontologies: Vec<(String, String, String)>,
    source_companion: Option<i64>,
    /// Transaction watermark of the consistent premise snapshot.
    pub premise_head: i64,
    timestamp: String,
    proposals: Vec<Datum>,
    next: usize,
    derived: bool,
    /// Derivation counts, before applying to the live store.
    pub report: MaterializeReport,
}

impl Snapshot {
    /// Copy only current ROOT and its inferred companion from a consistent read
    /// transaction. `scratch` must be an empty, private store owned by the caller.
    /// No live writes or derivation occur here.
    pub fn capture(source: &Store, scratch: Store, timestamp: &str) -> Result<Self> {
        if !scratch.current_facts()?.is_empty() || !scratch.list_ontologies()?.is_empty() {
            return Err(Error::InvalidValue(
                "OWL scratch store must be empty".into(),
            ));
        }
        let read = source.conn.unchecked_transaction()?;
        let premise_head = source.transaction_head()?;
        let identity = source.store_id()?;
        let ontologies = source.list_ontologies()?;
        let source_companion = source.lookup(ROOT_INFERRED_GRAPH_IRI)?;
        let companion = scratch.intern(ROOT_INFERRED_GRAPH_IRI)?;
        let write = scratch.conn.unchecked_transaction()?;
        scratch.conn.execute(
            "INSERT INTO transactions(timestamp, source) VALUES (?1, 'owl:snapshot')",
            params![timestamp],
        )?;
        let tx = scratch.conn.last_insert_rowid();
        let mut count = 0;
        for graph in std::iter::once(0).chain(source_companion) {
            let mut stmt = source.conn.prepare(
                "SELECT e, a, v, valid_from FROM facts \
                 WHERE g=?1 AND op=1 AND valid_to IS NULL",
            )?;
            let mut rows = stmt.query(params![graph])?;
            while let Some(row) = rows.next()? {
                count += 1;
                if count > MAX_PREMISES {
                    return Err(Error::InvalidValue(
                        "OWL snapshot premise budget exceeded".into(),
                    ));
                }
                let entity = remap_id(source, &scratch, row.get(0)?)?;
                let attribute = remap_id(source, &scratch, row.get(1)?)?;
                let value = remap_value(
                    source,
                    &scratch,
                    Value::from_bytes(&row.get::<_, Vec<u8>>(2)?)?,
                )?;
                scratch.conn.execute(
                    "INSERT OR IGNORE INTO facts(e,a,v,g,tx,valid_from,op) \
                     VALUES (?1,?2,?3,?4,?5,?6,1)",
                    params![
                        entity,
                        attribute,
                        value.to_bytes(),
                        if graph == 0 { 0 } else { companion },
                        tx,
                        row.get::<_, String>(3)?
                    ],
                )?;
            }
        }
        for (name, turtle, loaded_at) in &ontologies {
            scratch.load_ontology(name, turtle, loaded_at)?;
        }
        write.commit()?;
        read.commit()?;
        Ok(Self {
            scratch,
            identity,
            ontologies,
            source_companion,
            premise_head,
            timestamp: timestamp.to_owned(),
            proposals: Vec::new(),
            next: 0,
            derived: false,
            report: MaterializeReport::default(),
        })
    }

    /// Compute the closure on the private store. No live lock is needed.
    pub fn derive(&mut self) -> Result<()> {
        if self.derived {
            return Err(Error::InvalidValue("OWL snapshot already derived".into()));
        }
        self.scratch.ensure_owl_cache()?;
        let Some(ontology) = self.scratch.owl_cache.as_deref().cloned() else {
            self.derived = true;
            return Ok(());
        };
        let before = self.scratch.transaction_head()?;
        self.report =
            ontology.materialize_limited(&mut self.scratch, &self.timestamp, MAX_PROPOSALS)?;
        if self.report.pass_budget_exhausted || self.report.total > MAX_PROPOSALS {
            return Err(Error::InvalidValue(
                "OWL derivation budget exceeded; not applied".into(),
            ));
        }
        let mut stmt = self.scratch.conn.prepare(
            "SELECT f.e,f.a,f.v FROM facts f JOIN transactions t ON t.id=f.tx \
             WHERE f.tx>?1 AND t.source='owl:materialize' AND f.op=1 AND f.valid_to IS NULL",
        )?;
        let mut rows = stmt.query(params![before])?;
        while let Some(row) = rows.next()? {
            self.proposals.push(Datum {
                entity: row.get(0)?,
                attribute: row.get(1)?,
                value: Value::from_bytes(&row.get::<_, Vec<u8>>(2)?)?,
                valid_from: self.timestamp.clone(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        self.derived = true;
        Ok(())
    }

    /// Number of ontologies in the snapshot (zero is distinct from an empty closure).
    pub fn ontology_count(&self) -> usize {
        self.ontologies.len()
    }

    /// Number of proposals remaining to apply.
    pub fn remaining(&self) -> usize {
        self.proposals.len() - self.next
    }

    /// Publish freshness only after every batch succeeded and its premises are
    /// still valid. Uses indexed point reads, never a companion-wide scan.
    pub fn finish(&self, live: &mut Store) -> Result<()> {
        if !self.derived || self.remaining() != 0 {
            return Err(Error::InvalidValue("OWL snapshot is incomplete".into()));
        }
        self.validate(live)?;
        if self.ontologies.is_empty() {
            return Ok(());
        }
        let graph = live.intern(ROOT_INFERRED_GRAPH_IRI)?;
        let attr = live.intern(DERIVED_AS_OF_TX)?;
        let mut datums = Vec::new();
        {
            let mut stmt = live.conn.prepare(
                "SELECT v FROM facts WHERE g=?1 AND e=?1 AND a=?2 AND op=1 AND valid_to IS NULL",
            )?;
            let mut rows = stmt.query(params![graph, attr])?;
            while let Some(row) = rows.next()? {
                datums.push(Datum {
                    entity: graph,
                    attribute: attr,
                    value: Value::from_bytes(&row.get::<_, Vec<u8>>(0)?)?,
                    valid_from: self.timestamp.clone(),
                    valid_to: None,
                    op: Op::Retract,
                });
            }
        }
        datums.push(Datum {
            entity: graph,
            attribute: attr,
            value: Value::Int(self.premise_head),
            valid_from: self.timestamp.clone(),
            valid_to: None,
            op: Op::Assert,
        });
        datums.push(Datum {
            entity: graph,
            attribute: live.intern(SOURCE_KIND)?,
            value: Value::Str("inferred".into()),
            valid_from: self.timestamp.clone(),
            valid_to: None,
            op: Op::Assert,
        });
        live.transact_to_graph(
            &datums,
            &self.timestamp,
            Some("owl"),
            Some(PLANE_SOURCE),
            graph,
        )?;
        Ok(())
    }

    /// Validate monotone evolution, then apply at most [`APPLY_BATCH`] assertions.
    /// Concurrent additions are allowed: the result is explicitly as-of the
    /// snapshot, not a claim that new facts have been processed. Retractions or
    /// ontology changes stop publication. Already committed batches are partial
    /// historical entailments; callers must report failure and not freshness.
    pub fn apply_batch(&mut self, live: &mut Store) -> Result<usize> {
        if !self.derived {
            return Err(Error::InvalidValue(
                "OWL snapshot has not been derived".into(),
            ));
        }
        self.validate(live)?;
        let end = (self.next + APPLY_BATCH).min(self.proposals.len());
        let mut batch = Vec::with_capacity(end - self.next);
        for datum in &self.proposals[self.next..end] {
            batch.push(Datum {
                entity: remap_id(&self.scratch, live, datum.entity)?,
                attribute: remap_id(&self.scratch, live, datum.attribute)?,
                value: remap_value(&self.scratch, live, datum.value.clone())?,
                valid_from: datum.valid_from.clone(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        if !batch.is_empty() {
            let companion = live.intern(ROOT_INFERRED_GRAPH_IRI)?;
            live.transact_to_graph(
                &batch,
                &self.timestamp,
                Some("owl"),
                Some("owl:materialize"),
                companion,
            )?;
        }
        self.next = end;
        Ok(batch.len())
    }

    fn validate(&self, live: &Store) -> Result<()> {
        if live.store_id()? != self.identity || live.list_ontologies()? != self.ontologies {
            return Err(Error::InvalidValue(
                "OWL snapshot identity or ontology changed".into(),
            ));
        }
        let freshness = live.lookup(DERIVED_AS_OF_TX)?.unwrap_or(-1);
        for graph in std::iter::once(0).chain(self.source_companion) {
            let retracted: bool = live.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM facts INDEXED BY idx_retracted_tx \
                 WHERE g=?1 AND retracted_tx>?2 \
                 AND NOT (g!=0 AND e=?1 AND a=?3))",
                params![graph, self.premise_head, freshness],
                |row| row.get(0),
            )?;
            if retracted {
                return Err(Error::InvalidValue(
                    "OWL snapshot premises retracted; retry required".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "owl_snapshot_tests.rs"]
mod tests;

fn remap_id(source: &Store, target: &Store, id: i64) -> Result<i64> {
    target.intern(&source.resolve(id)?)
}

fn remap_value(source: &Store, target: &Store, value: Value) -> Result<Value> {
    match value {
        Value::Ref(id) => Ok(Value::Ref(remap_id(source, target, id)?)),
        value => Ok(value),
    }
}
