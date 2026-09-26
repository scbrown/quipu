//! Per-source audit claims and first/last visible fact transitions.
use super::Store;
use crate::{Result, Value};
use rusqlite::params;

impl Store {
    /// Replace one producer snapshot without widening its retraction scope.
    pub(crate) fn transact_snapshot(
        &mut self,
        datums: &[super::Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: &str,
        graph: i64,
    ) -> Result<i64> {
        self.transact_to_graph_scoped(datums, timestamp, actor, Some(source), graph, Some(source))
    }
}

impl Store {
    pub(crate) fn has_fact_claim(&self, e: i64, a: i64, value: &Value, graph: i64) -> Result<bool> {
        let mut stmt = self.prepare("SELECT 1 FROM facts WHERE e=?1 AND a=?2 AND v=?3 AND g=?4 AND op=1 AND valid_to IS NULL LIMIT 1")?;
        Ok(stmt.exists(params![e, a, value.to_bytes(), graph])?)
    }

    fn source_has_fact_claim(
        &self,
        datum: &super::Datum,
        graph: i64,
        source: Option<&str>,
    ) -> Result<bool> {
        let mut stmt = self.prepare("SELECT 1 FROM facts f JOIN transactions t ON t.id=f.tx WHERE f.e=?1 AND f.a=?2 AND f.v=?3 AND f.g=?4 AND f.op=1 AND f.valid_to IS NULL AND t.source IS ?5 LIMIT 1")?;
        Ok(stmt.exists(params![
            datum.entity,
            datum.attribute,
            datum.value.to_bytes(),
            graph,
            source
        ])?)
    }

    /// Write raw audit rows, returning only changes to logical RDF presence.
    /// A source restriction narrows closures; it never changes lookup identity.
    pub(super) fn stage_source_claims(
        &self,
        datums: &[super::Datum],
        graph: i64,
        tx: i64,
        timestamp: &str,
        retract_source: Option<&str>,
    ) -> Result<(Vec<super::Datum>, Vec<super::Datum>)> {
        use crate::Op;
        let source: Option<String> =
            self.conn
                .query_row("SELECT source FROM transactions WHERE id=?1", [tx], |row| {
                    row.get(0)
                })?;
        let mut before = std::collections::BTreeMap::new();
        let mut insert = self.prepare(
            "INSERT INTO facts(e,a,v,g,tx,valid_from,valid_to,op) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(e,a,v,tx) DO UPDATE SET valid_from=excluded.valid_from,valid_to=excluded.valid_to,op=excluded.op,retracted_tx=NULL",
        )?;
        let mut close = self.prepare("UPDATE facts SET valid_to=?1,retracted_tx=?6 WHERE e=?2 AND a=?3 AND v=?4 AND g=?5 AND op=1 AND valid_to IS NULL AND (?7 IS NULL OR tx IN (SELECT id FROM transactions WHERE source=?7))")?;
        for datum in datums {
            let present =
                self.has_fact_claim(datum.entity, datum.attribute, &datum.value, graph)?;
            let new_claim = if present && datum.op == Op::Assert {
                !self.source_has_fact_claim(datum, graph, source.as_deref())?
            } else {
                false
            };
            if matches!(datum.op, Op::Assert | Op::Retract) {
                before
                    .entry((datum.entity, datum.attribute, datum.value.to_bytes()))
                    .or_insert_with(|| (datum.clone(), present));
            }
            if datum.op == Op::Retract {
                {
                    let bytes = datum.value.to_bytes();
                    let count = close.execute(params![
                        timestamp,
                        datum.entity,
                        datum.attribute,
                        bytes,
                        graph,
                        tx,
                        retract_source
                    ])?;
                    if count > 0 {
                        // Exact original blob in the immutable retraction log.
                        insert.execute(params![
                            datum.entity,
                            datum.attribute,
                            bytes,
                            graph,
                            tx,
                            datum.valid_from,
                            datum.valid_to,
                            Op::Retract as i32
                        ])?;
                    }
                }
            } else if !present || new_claim {
                insert.execute(params![
                    datum.entity,
                    datum.attribute,
                    datum.value.to_bytes(),
                    graph,
                    tx,
                    datum.valid_from,
                    datum.valid_to,
                    datum.op as i32
                ])?;
            }
        }
        // Consumers see the committed before/after transition, not intermediate
        // disappearances inside a retract/assert batch. The schema has one row
        // per blob/transaction, so repeated operations replace only this tx's
        // staged row. Earlier assertions retain their bytes and closure tx.
        let mut asserts = Vec::new();
        let mut retracts = Vec::new();
        for (_, (mut datum, was_present)) in before {
            let present =
                self.has_fact_claim(datum.entity, datum.attribute, &datum.value, graph)?;
            match (was_present, present) {
                (false, true) => {
                    datum.op = Op::Assert;
                    asserts.push(datum);
                }
                (true, false) => {
                    datum.op = Op::Retract;
                    retracts.push(datum);
                }
                _ => {}
            }
        }
        Ok((asserts, retracts))
    }
}

impl Store {
    /// Project physical claims to RDF statements after applying scope/time/source filters.
    /// History and retraction planners deliberately keep using `collect_facts`.
    pub(crate) fn collect_visible_facts(
        stmt: &mut rusqlite::Statement<'_>,
        params: impl rusqlite::Params,
    ) -> Result<Vec<crate::Fact>> {
        let mut facts = Self::collect_facts(stmt, params)?;
        let mut seen = std::collections::HashSet::new();
        facts.retain(|f| seen.insert((f.entity, f.attribute, f.value.to_bytes())));
        Ok(facts)
    }
}
