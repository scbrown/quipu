//! Indexed compatibility lookup without rewriting historical literal blobs.
use super::Store;
use crate::{Result, Value, namespace};
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

    /// Include attached graphs for query lookup, local facts only for writes.
    pub(crate) fn literal_aliases(&self, value: &Value, composed: bool) -> Result<Vec<Vec<u8>>> {
        let mut aliases = value.physical_aliases();
        let key = value.term_key();
        let nan = Value::Typed {
            lexical: "NaN".into(),
            datatype: namespace::XSD_DOUBLE.into(),
        };
        if key == nan.term_key() {
            let facts = if composed {
                self.facts_source()
            } else {
                "facts"
            };
            // The range is a BLOB range over the existing value-leading index.
            // Do not reconstruct NaN payloads or reinterpret their stored bits.
            let mut stmt = self.prepare(&format!(
                "SELECT DISTINCT v FROM {facts} WHERE v >= ?1 AND v < ?2"
            ))?;
            let mut rows = stmt.query(params![vec![3_u8], vec![4_u8]])?;
            while let Some(row) = rows.next()? {
                let bytes: Vec<u8> = row.get(0)?;
                if Value::from_bytes(&bytes)?.term_key() == key {
                    aliases.push(bytes);
                }
            }
        }
        aliases.sort_unstable();
        aliases.dedup();
        Ok(aliases)
    }
}

impl Store {
    pub(crate) fn has_literal(&self, e: i64, a: i64, value: &Value, graph: i64) -> Result<bool> {
        let mut stmt = self.prepare("SELECT 1 FROM facts WHERE e=?1 AND a=?2 AND v=?3 AND g=?4 AND op=1 AND valid_to IS NULL LIMIT 1")?;
        for bytes in self.literal_aliases(value, false)? {
            if stmt.exists(params![e, a, bytes, graph])? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn source_has_literal(&self, datum: &super::Datum, graph: i64, source: &str) -> Result<bool> {
        let mut stmt = self.prepare("SELECT 1 FROM facts f JOIN transactions t ON t.id=f.tx WHERE f.e=?1 AND f.a=?2 AND f.v=?3 AND f.g=?4 AND f.op=1 AND f.valid_to IS NULL AND t.source=?5 LIMIT 1")?;
        for bytes in self.literal_aliases(&datum.value, false)? {
            if stmt.exists(params![datum.entity, datum.attribute, bytes, graph, source])? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Write raw audit rows, returning only changes to logical RDF presence.
    /// A source restriction narrows closures; it never changes lookup identity.
    pub(super) fn stage_literal_facts(
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
            let present = self.has_literal(datum.entity, datum.attribute, &datum.value, graph)?;
            let new_claim = if present && datum.op == Op::Assert {
                match source.as_deref() {
                    Some(source) => !self.source_has_literal(datum, graph, source)?,
                    None => false,
                }
            } else {
                false
            };
            if matches!(datum.op, Op::Assert | Op::Retract) {
                before
                    .entry((datum.entity, datum.attribute, datum.value.term_key()))
                    .or_insert_with(|| (datum.clone(), present));
            }
            if datum.op == Op::Retract {
                for bytes in self.literal_aliases(&datum.value, false)? {
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
            let present = self.has_literal(datum.entity, datum.attribute, &datum.value, graph)?;
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

#[cfg(test)]
mod tests;
