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
        let mut before = std::collections::BTreeMap::new();
        let mut insert = self.prepare(
            "INSERT INTO facts(e,a,v,g,tx,valid_from,valid_to,op) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(e,a,v,tx) DO UPDATE SET valid_from=excluded.valid_from,valid_to=excluded.valid_to,op=excluded.op,retracted_tx=NULL",
        )?;
        let mut close = self.prepare("UPDATE facts SET valid_to=?1,retracted_tx=?6 WHERE e=?2 AND a=?3 AND v=?4 AND g=?5 AND op=1 AND valid_to IS NULL AND (?7 IS NULL OR tx IN (SELECT id FROM transactions WHERE source=?7))")?;
        for datum in datums {
            let present = self.has_literal(datum.entity, datum.attribute, &datum.value, graph)?;
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
            } else if !present {
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
mod tests {
    use super::*;
    use crate::{Datum, Op};
    const TIME: &str = "2026-01-01T00:00:00Z";
    #[cfg(feature = "reactive-reasoner")]
    #[derive(Default)]
    struct Capture(std::sync::Mutex<Vec<(usize, usize)>>);
    #[cfg(feature = "reactive-reasoner")]
    impl super::super::TransactObserver for Capture {
        fn after_commit(&self, _store: &mut Store, delta: &super::super::Delta) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .push((delta.asserts.len(), delta.retracts.len()));
            Ok(())
        }
    }

    fn legacy(store: &Store, e: i64, a: i64, value: &Value, source: &str) {
        store
            .conn
            .execute(
                "INSERT INTO transactions(timestamp,source) VALUES(?1,?2)",
                params![TIME, source],
            )
            .unwrap();
        let tx = store.conn.last_insert_rowid();
        store
            .conn
            .execute(
                "INSERT INTO facts(e,a,v,g,tx,valid_from,op) VALUES(?1,?2,?3,0,?4,?5,1)",
                params![e, a, value.to_bytes(), tx, TIME],
            )
            .unwrap();
    }

    #[test]
    fn source_cleanup_preserves_other_alias_and_catchup_presence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let mut store = Store::open(path.to_str().unwrap()).unwrap();
        let e = store.intern("http://example.org/s").unwrap();
        let a = store.intern("http://example.org/p").unwrap();
        let old = Value::Float(1.0);
        let typed = Value::Typed {
            lexical: "1".into(),
            datatype: namespace::XSD_DOUBLE.into(),
        };
        let sibling = Value::Typed {
            lexical: "1.0".into(),
            datatype: namespace::XSD_DOUBLE.into(),
        };
        legacy(&store, e, a, &old, "episode:A");
        legacy(&store, e, a, &typed, "episode:B");
        legacy(&store, e, a, &sibling, "episode:B");
        let old_bytes = old.to_bytes();
        let event_offset = store.latest_event_offset().unwrap();
        #[cfg(feature = "reactive-reasoner")]
        let observer = std::sync::Arc::new(Capture::default());
        #[cfg(feature = "reactive-reasoner")]
        store.add_observer(observer.clone());
        let since = store.latest_tx_id().unwrap();
        assert_eq!(store.read_model().unwrap().len(), 2);
        let reader = Store::open_read_only(path.to_str().unwrap()).unwrap();
        assert_eq!(reader.read_model().unwrap().len(), 2);
        let d = Datum {
            entity: e,
            attribute: a,
            value: typed.clone(),
            valid_from: TIME.into(),
            valid_to: None,
            op: Op::Retract,
        };
        store
            .transact_to_graph_scoped(
                std::slice::from_ref(&d),
                "2026-01-02",
                None,
                Some("cleanup"),
                0,
                Some("episode:A"),
            )
            .unwrap();
        assert_eq!(store.latest_event_offset().unwrap(), event_offset);
        #[cfg(feature = "reactive-reasoner")]
        assert_eq!(*observer.0.lock().unwrap(), vec![(0, 0)]);
        let raw = store.current_facts().unwrap();
        assert_eq!(raw.len(), 2);
        assert!(raw.iter().any(|f| f.value.to_bytes() == typed.to_bytes()));
        assert!(raw.iter().any(|f| f.value.to_bytes() == sibling.to_bytes()));
        assert_eq!(store.read_model().unwrap().len(), 2);
        assert_eq!(reader.read_model().unwrap().len(), 2);
        let changes = reader.facts_changed_since_in_graph(0, since).unwrap();
        assert!(changes.iter().all(|d| d.op == Op::Assert));
        let stored: Vec<u8> = store
            .conn
            .query_row(
                "SELECT v FROM facts WHERE op=1 ORDER BY tx LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, old_bytes);
        // Logical retraction closes the remaining equivalent encoding only.
        store
            .transact(&[d], "2026-01-03", None, Some("logical-retract"))
            .unwrap();
        assert!(store.latest_event_offset().unwrap() > event_offset);
        #[cfg(feature = "reactive-reasoner")]
        assert_eq!(*observer.0.lock().unwrap(), vec![(0, 0), (0, 1)]);
        assert_eq!(store.current_facts().unwrap().len(), 1);
        assert_eq!(reader.read_model().unwrap().len(), 1);
        assert_eq!(store.read_model().unwrap().len(), 1);
        assert_eq!(store.current_facts().unwrap()[0].value, sibling);
    }

    #[test]
    fn logical_retraction_closes_every_nan_payload_without_touching_other_graph() {
        let mut store = Store::open_in_memory().unwrap();
        let e = store.intern("http://example.org/s").unwrap();
        let a = store.intern("http://example.org/p").unwrap();
        for bits in [
            0x7ff8_0000_0000_0000,
            0x7ff8_0000_0000_0123,
            0xfff8_0000_0000_0001,
            0x7ff0_0000_0000_0001,
            0xfff0_0000_0000_0001,
        ] {
            legacy(&store, e, a, &Value::Float(f64::from_bits(bits)), "legacy");
        }
        let value = Value::Typed {
            lexical: "NaN".into(),
            datatype: namespace::XSD_DOUBLE.into(),
        };
        let mut d = Datum {
            entity: e,
            attribute: a,
            value,
            valid_from: TIME.into(),
            valid_to: None,
            op: Op::Assert,
        };
        let graph = store.graph_create("http://example.org/other").unwrap();
        store
            .transact_to_graph(std::slice::from_ref(&d), TIME, None, None, graph)
            .unwrap();
        assert_eq!(store.read_model().unwrap().len(), 1);
        d.op = Op::Retract;
        store.transact(&[d], "2026-01-02", None, None).unwrap();
        assert!(store.current_facts().unwrap().is_empty());
        assert_eq!(store.current_facts_in_graph(graph).unwrap().len(), 1);
        assert_eq!(store.read_model().unwrap().len(), 0);
        let audit: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM facts WHERE op=0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audit, 5);
    }

    #[test]
    fn attached_nan_lookup_keeps_payload_and_excludes_finite_controls() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let layer = dir.path().join("layer.db");
        let main = dir.path().join("main.db");
        let mut store = Store::open(source.to_str().unwrap()).unwrap();
        let e = store.intern("http://example.org/s").unwrap();
        let a = store.intern("http://example.org/p").unwrap();
        let g = store.graph_create("http://example.org/layer").unwrap();
        let nan = Value::Float(f64::from_bits(0xfff0_0000_0000_0123));
        for value in [nan.clone(), Value::Float(1.0)] {
            store
                .transact_to_graph(
                    &[Datum {
                        entity: e,
                        attribute: a,
                        value,
                        op: Op::Assert,
                        valid_from: TIME.into(),
                        valid_to: None,
                    }],
                    TIME,
                    None,
                    None,
                    g,
                )
                .unwrap();
        }
        drop(store);
        crate::store::respace::respace_file(&source, &layer, 3).unwrap();
        let store = Store::open_with_attachments(
            main.to_str().unwrap(),
            &[crate::store::attach::Attachment::read_only(
                "layer",
                layer.to_str().unwrap(),
            )],
        )
        .unwrap();
        let typed = Value::Typed {
            lexical: "NaN".into(),
            datatype: namespace::XSD_DOUBLE.into(),
        };
        let aliases = store.literal_aliases(&typed, true).unwrap();
        assert!(aliases.contains(&nan.to_bytes()));
        assert!(!aliases.contains(&Value::Float(1.0).to_bytes()));
        let rows = crate::sparql::query(&store, "SELECT ?s FROM <http://example.org/layer> WHERE { ?s <http://example.org/p> \"NaN\"^^<http://www.w3.org/2001/XMLSchema#double> }").unwrap();
        assert_eq!(rows.rows().len(), 1);
    }

    #[test]
    fn mixed_alias_batch_reports_only_committed_presence_changes() {
        for initially_present in [false, true] {
            for assert_last in [false, true] {
                let mut store = Store::open_in_memory().unwrap();
                let e = store.intern("http://example.org/s").unwrap();
                let a = store.intern("http://example.org/p").unwrap();
                if initially_present {
                    legacy(&store, e, a, &Value::Float(1.0), "legacy");
                }
                assert_eq!(
                    store.read_model().unwrap().len(),
                    usize::from(initially_present)
                );
                let offset = store.latest_event_offset().unwrap();
                #[cfg(feature = "reactive-reasoner")]
                let observer = std::sync::Arc::new(Capture::default());
                #[cfg(feature = "reactive-reasoner")]
                store.add_observer(observer.clone());
                let assert = Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Typed {
                        lexical: "1".into(),
                        datatype: namespace::XSD_DOUBLE.into(),
                    },
                    op: Op::Assert,
                    valid_from: TIME.into(),
                    valid_to: None,
                };
                let mut retract = assert.clone();
                retract.value = Value::Float(1.0);
                retract.op = Op::Retract;
                let batch = if assert_last {
                    [retract, assert]
                } else {
                    [assert, retract]
                };
                store.transact(&batch, "2026-01-02", None, None).unwrap();
                let expected = usize::from(assert_last);
                assert_eq!(store.current_facts().unwrap().len(), expected);
                assert_eq!(store.read_model().unwrap().len(), expected);
                assert_eq!(store.build_read_model(0).unwrap().len(), expected);
                assert_eq!(
                    store.latest_event_offset().unwrap() > offset,
                    initially_present != assert_last
                );
                #[cfg(feature = "reactive-reasoner")]
                assert_eq!(
                    *observer.0.lock().unwrap(),
                    vec![(
                        usize::from(!initially_present && assert_last),
                        usize::from(initially_present && !assert_last)
                    )]
                );
            }
        }
    }
}
