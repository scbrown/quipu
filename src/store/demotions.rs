//! Preserved, unsupported derivations are evidence, not asserted premises.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;
use sha2::{Digest, Sha256};

use super::inferred::PLANE_SOURCE;
use super::{Datum, Store};
use crate::error::Result;
use crate::namespace::RDF_TYPE;
#[cfg(any(feature = "owl", test))]
use crate::types::Fact;
use crate::types::{Op, Value};

/// Product vocabulary for plane bookkeeping.
pub const NS: &str = "http://quipu.local/graph#";
/// Class of a preserved demotion record.
pub const DEMOTED: &str = "http://quipu.local/graph#DemotedDerivation";
/// Current disposition; resolved records are retained.
pub const STATE: &str = "http://quipu.local/graph#derivationState";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Stored query exposed through `/ask` and `quipu demotions`.
pub const QUERY_NAME: &str = "unsupported_demotions";

impl Store {
    /// Install the product's enumeration query once, without replacing user data.
    pub fn ensure_demotion_query(&self, timestamp: &str) -> Result<()> {
        use super::queries::{StoredParam, StoredQuery};
        if self.query_get(QUERY_NAME)?.is_some() {
            return Ok(());
        }
        self.query_load(
            &StoredQuery {
                name: QUERY_NAME.into(),
                description: "Unsupported demotions retained as evidence in a companion graph"
                    .into(),
                template: format!(
                    "SELECT ?record ?subject ?predicate ?object ?premiseGraph ?deriverSource ?promotionTx ?invalidationTx \
                FROM <{{graph}}> WHERE {{ ?record <{STATE}> 'unsupported' ; \
                <{RDF}subject> ?subject ; <{RDF}predicate> ?predicate ; <{RDF}object> ?object ; \
                <{NS}premiseGraph> ?premiseGraph ; <{NS}deriverSource> ?deriverSource ; \
                <{NS}promotionTx> ?promotionTx ; <{NS}invalidationTx> ?invalidationTx }}"
                ),
                dataset: None,
                params: vec![StoredParam {
                    name: "graph".into(),
                    kind: "iri".into(),
                    required: false,
                    default: Some(super::inferred::ROOT_INFERRED_GRAPH_IRI.into()),
                    description: "Companion graph to inspect".into(),
                }],
            },
            timestamp,
        )
    }
}

fn datum(entity: i64, attribute: i64, value: Value, timestamp: &str, op: Op) -> Datum {
    Datum {
        entity,
        attribute,
        value,
        valid_from: timestamp.into(),
        valid_to: None,
        op,
    }
}

impl Store {
    /// Remove plane bookkeeping from engine inputs, including incremental seeds.
    /// Queries and exports still see these records in their explicit graph.
    #[cfg(any(feature = "owl", test))]
    pub(crate) fn without_plane_metadata(&self, facts: Vec<Fact>) -> Result<Vec<Fact>> {
        let mut metadata = BTreeSet::new();
        let txs: Vec<i64> = facts
            .iter()
            .map(|f| f.tx)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for chunk in txs.chunks(256) {
            let slots = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT id FROM transactions WHERE source = ? AND id IN ({slots})");
            let mut params: Vec<&dyn rusqlite::ToSql> = vec![&PLANE_SOURCE];
            params.extend(chunk.iter().map(|tx| tx as &dyn rusqlite::ToSql));
            let mut statement = self.conn.prepare(&sql)?;
            let rows = statement.query_map(params.as_slice(), |r| r.get::<_, i64>(0))?;
            for tx in rows {
                metadata.insert(tx?);
            }
        }
        Ok(facts
            .into_iter()
            .filter(|f| !metadata.contains(&f.tx))
            .collect())
    }

    /// Reconcile a rule's promoted facts and preserved demotion records.
    /// Only this rule's own source is eligible; ordinary base facts are untouched.
    pub(crate) fn reconcile_promoted_derivations(
        &mut self,
        attribute: i64,
        source: &str,
        premise: i64,
        companion: i64,
        supported: &BTreeSet<(i64, i64)>,
        timestamp: &str,
    ) -> Result<usize> {
        self.conn.execute_batch("SAVEPOINT quipu_demotion")?;
        let result = self.reconcile_demotion_snapshot(
            attribute, source, premise, companion, supported, timestamp,
        );
        match result {
            Ok(count) => {
                self.conn.execute_batch("RELEASE quipu_demotion")?;
                Ok(count)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO quipu_demotion; RELEASE quipu_demotion")?;
                self.read_model.borrow_mut().clear();
                Err(error)
            }
        }
    }

    fn reconcile_demotion_snapshot(
        &mut self,
        attribute: i64,
        source: &str,
        premise: i64,
        companion: i64,
        supported: &BTreeSet<(i64, i64)>,
        timestamp: &str,
    ) -> Result<usize> {
        let promoted = {
            let mut stmt = self.conn.prepare(
                "SELECT f.e, f.v, f.tx FROM facts f JOIN transactions t ON f.tx=t.id \
                 WHERE f.g=?1 AND f.a=?2 AND f.op=1 AND f.valid_to IS NULL AND t.source=?3",
            )?;
            let rows = stmt.query_map(params![premise, attribute, source], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut retracts = Vec::new();
        let mut records = Vec::new();
        // The read snapshot and both batches share a savepoint. INTEGER PRIMARY
        // KEY allocates this id; a competing writer causes a snapshot refusal,
        // never a silently misattributed invalidation.

        let invalidation_tx = self.transaction_head()? + 1;
        for (entity, bytes, promotion_tx) in promoted {
            let Value::Ref(object) = Value::from_bytes(&bytes)? else {
                continue;
            };
            if supported.contains(&(entity, object)) {
                continue;
            }
            let identity = [
                self.resolve(entity)?,
                self.resolve(attribute)?,
                self.resolve(object)?,
                self.graph_iri_of(premise),
                source.to_string(),
                promotion_tx.to_string(),
            ];
            let mut hash = Sha256::new();
            for part in identity {
                hash.update((part.len() as u64).to_be_bytes());
                hash.update(part.as_bytes());
            }
            let record = self.intern(&format!("urn:quipu:demotion:{:x}", hash.finalize()))?;
            retracts.push(datum(
                entity,
                attribute,
                Value::Ref(object),
                timestamp,
                Op::Retract,
            ));
            let fields = [
                (RDF_TYPE.to_string(), Value::Ref(self.intern(DEMOTED)?)),
                (format!("{RDF}subject"), Value::Ref(entity)),
                (format!("{RDF}predicate"), Value::Ref(attribute)),
                (format!("{RDF}object"), Value::Ref(object)),
                (STATE.to_string(), Value::Str("unsupported".into())),
                (
                    format!("{NS}premiseGraph"),
                    Value::Ref(self.intern(&self.graph_iri_of(premise))?),
                ),
                (format!("{NS}deriverSource"), Value::Str(source.into())),
                (format!("{NS}promotionTx"), Value::Int(promotion_tx)),
                (format!("{NS}invalidationTx"), Value::Int(invalidation_tx)),
                (
                    "http://www.w3.org/ns/prov#invalidatedAtTime".into(),
                    Value::Typed {
                        lexical: timestamp.into(),
                        datatype: "http://www.w3.org/2001/XMLSchema#dateTime".into(),
                    },
                ),
            ];
            for (predicate, value) in fields {
                records.push(datum(
                    record,
                    self.intern(&predicate)?,
                    value,
                    timestamp,
                    Op::Assert,
                ));
            }
        }
        let demoted = retracts.len();
        if demoted > 0 {
            records.push(datum(
                self.intern(DEMOTED)?,
                self.intern(crate::namespace::RDFS_SUBCLASS_OF)?,
                Value::Ref(self.intern(&format!("{RDF}Statement"))?),
                timestamp,
                Op::Assert,
            ));
            self.transact_graph_batches(
                &[(premise, retracts), (companion, records)],
                timestamp,
                Some("reasoner"),
                Some(PLANE_SOURCE),
            )?;
        }

        // Resolve retained evidence only when this same rule derives the same
        // tuple again. This does not restore first-class standing or promote it.
        let state = self.intern(STATE)?;
        let origin = self.intern(&format!("{NS}deriverSource"))?;
        let predicate = self.intern(&format!("{RDF}predicate"))?;
        let subject = self.intern(&format!("{RDF}subject"))?;
        let object = self.intern(&format!("{RDF}object"))?;
        let mut by_entity: BTreeMap<i64, BTreeMap<i64, Value>> = BTreeMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT f.e, f.a, f.v FROM facts marker JOIN facts f ON f.e=marker.e AND f.g=marker.g \
                 WHERE marker.g=?1 AND marker.a=?2 AND marker.v=?3 \
                   AND marker.op=1 AND marker.valid_to IS NULL AND f.op=1 AND f.valid_to IS NULL"
            )?;
            let mut rows = stmt.query(params![
                companion,
                origin,
                Value::Str(source.into()).to_bytes()
            ])?;
            while let Some(row) = rows.next()? {
                by_entity
                    .entry(row.get(0)?)
                    .or_default()
                    .insert(row.get(1)?, Value::from_bytes(&row.get::<_, Vec<u8>>(2)?)?);
            }
        }
        let mut changes = Vec::new();
        for (record, fields) in by_entity {
            if fields.get(&state) != Some(&Value::Str("unsupported".into()))
                || fields.get(&origin) != Some(&Value::Str(source.into()))
                || fields.get(&predicate) != Some(&Value::Ref(attribute))
            {
                continue;
            }
            if let (Some(Value::Ref(s)), Some(Value::Ref(o))) =
                (fields.get(&subject), fields.get(&object))
                && supported.contains(&(*s, *o))
            {
                changes.push(datum(
                    record,
                    state,
                    Value::Str("unsupported".into()),
                    timestamp,
                    Op::Retract,
                ));
                changes.push(datum(
                    record,
                    state,
                    Value::Str("resolved".into()),
                    timestamp,
                    Op::Assert,
                ));
            }
        }
        if !changes.is_empty() {
            self.transact_to_graph(
                &changes,
                timestamp,
                Some("reasoner"),
                Some(PLANE_SOURCE),
                companion,
            )?;
        }
        Ok(demoted)
    }
}
