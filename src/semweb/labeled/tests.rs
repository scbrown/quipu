use super::*;
use crate::store::Datum;
use crate::types::Op;
use proptest::prelude::*;

fn reference(store: &Store, deadline: Option<crate::time::Deadline>) -> Result<Vec<LabeledEntity>> {
    let started = crate::time::Stopwatch::start();
    // Keep these as two indexed single-pattern queries and join in Rust. The
    // equivalent OPTIONAL query makes the generic evaluator materialize and
    // merge the whole label × type binding set; on the production-sized graph
    // that exceeded 30 seconds while each indexed arm completes in <250ms.
    let labels = crate::sparql::query_temporal(
        store,
        "SELECT ?s ?label WHERE { \
         ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label }",
        &crate::sparql::TemporalContext {
            deadline,
            ..Default::default()
        },
    )?;
    let types = crate::sparql::query_temporal(
        store,
        "SELECT ?s ?type WHERE { \
         ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type }",
        &crate::sparql::TemporalContext {
            deadline,
            ..Default::default()
        },
    )?;

    let mut types_by_entity: HashMap<i64, Vec<String>> = HashMap::new();
    for row in types.rows() {
        if deadline.is_some_and(|d| d.passed()) {
            return Err(crate::Error::QueryTimeout {
                elapsed_ms: started.elapsed_ms(),
                limit_ms: deadline
                    .map(|d| d.millis_from(&started))
                    .unwrap_or_default(),
            });
        }
        let (Some(Value::Ref(entity)), Some(Value::Ref(entity_type))) =
            (row.get("s"), row.get("type"))
        else {
            continue;
        };
        types_by_entity
            .entry(*entity)
            .or_default()
            .push(store.resolve(*entity_type).unwrap_or_default());
    }

    let mut entities = Vec::new();
    for row in labels.rows() {
        if deadline.is_some_and(|d| d.passed()) {
            return Err(crate::Error::QueryTimeout {
                elapsed_ms: started.elapsed_ms(),
                limit_ms: deadline
                    .map(|d| d.millis_from(&started))
                    .unwrap_or_default(),
            });
        }
        let entity_id = match row.get("s") {
            Some(Value::Ref(id)) => *id,
            _ => continue,
        };
        let iri = store.resolve(entity_id).unwrap_or_default();
        let label = match row.get("label") {
            Some(Value::Str(s)) => s.clone(),
            _ => continue,
        };
        match types_by_entity.get(&entity_id) {
            Some(types) if !types.is_empty() => {
                entities.extend(types.iter().cloned().map(|entity_type| LabeledEntity {
                    iri: iri.clone().into(),
                    label: label.clone().into(),
                    entity_type: entity_type.into(),
                }));
            }
            _ => entities.push(LabeledEntity {
                iri: iri.into(),
                label: label.into(),
                entity_type: "".into(),
            }),
        }
    }
    Ok(entities)
}

fn sorted(entities: Vec<LabeledEntity>) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = entities
        .into_iter()
        .map(|e| {
            (
                e.iri.to_string(),
                e.label.to_string(),
                e.entity_type.to_string(),
            )
        })
        .collect();
    rows.sort();
    rows
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn streamed_scan_matches_sparql_after_writes(
        changes in prop::collection::vec((0usize..8, 0u8..6, 0u8..4, any::<bool>(), any::<bool>()), 0..45)
    ) {
        let mut store = Store::open_in_memory().unwrap();
        let label = store.intern("http://www.w3.org/2000/01/rdf-schema#label").unwrap();
        let ty = store.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type").unwrap();
        let named = store.intern("urn:test:named").unwrap();
        for (i, (entity, kind, variant, in_named, retract)) in changes.into_iter().enumerate() {
            let entity = store.intern(&format!("urn:test:entity:{entity}")).unwrap();
            let (attribute, value) = match kind {
                0 | 1 => (label, Value::Str(format!("label {variant}"))),
                2 => (label, Value::Lang { lexical: format!("label {variant}"), lang: "en".into() }),
                3 | 4 => (ty, Value::Ref(store.intern(&format!("urn:test:type:{variant}")).unwrap())),
                _ => (ty, Value::Str("not a type reference".into())),
            };
            let timestamp = format!("2026-01-01T00:00:{i:02}Z");
            store.transact_to_graph(&[Datum {
                entity, attribute, value,
                valid_from: timestamp.clone(), valid_to: None,
                op: if retract { Op::Retract } else { Op::Assert },
            }], &timestamp, None, None, if in_named { named } else { 0 }).unwrap();
            let actual = fetch_labeled_entities(&store).unwrap();
            let expected = reference(&store, None).unwrap();
            prop_assert_eq!(super::super::spotlight_over(&actual, "label 0 label 1 label 2 label 3", 0.5), super::super::spotlight_over(&expected, "label 0 label 1 label 2 label 3", 0.5));
            prop_assert_eq!(sorted(actual), sorted(expected));
        }
    }
}

#[test]
fn scan_keeps_row_cap_and_clears_expired_budget() {
    let mut store = Store::open_in_memory().unwrap();
    let label = store
        .intern("http://www.w3.org/2000/01/rdf-schema#label")
        .unwrap();
    let entity = store.intern("urn:test:entity").unwrap();
    let datums: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|s| Datum {
            entity,
            attribute: label,
            value: Value::Str(s.into()),
            valid_from: "2026-01-01".into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store.transact(&datums, "2026-01-01", None, None).unwrap();
    store.search_config_mut().max_join_rows = 1;
    assert!(matches!(
        fetch_labeled_entities(&store),
        Err(Error::QueryComplexity { limit: 1 })
    ));
    store.search_config_mut().max_join_rows = 0;
    assert!(matches!(
        fetch_labeled_entities_until(&store, Some(Deadline::after_millis(0))),
        Err(Error::QueryTimeout { .. })
    ));
    assert_eq!(fetch_labeled_entities(&store).unwrap().len(), 2);
}

#[test]
fn scan_collapses_duplicate_current_fact_rows() {
    let mut store = Store::open_in_memory().unwrap();
    let label = store
        .intern("http://www.w3.org/2000/01/rdf-schema#label")
        .unwrap();
    let entity = store.intern("urn:test:duplicate").unwrap();
    store
        .transact(
            &[Datum {
                entity,
                attribute: label,
                value: Value::Str("duplicate".into()),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01",
            None,
            None,
        )
        .unwrap();
    // Legacy imports can leave the same triple current in several transactions.
    store
        .conn
        .execute(
            "INSERT INTO transactions(id, timestamp) VALUES (9000, '2026-01-02')",
            [],
        )
        .unwrap();
    store.conn.execute("INSERT INTO facts(e,a,v,g,tx,valid_from,valid_to,op) SELECT e,a,v,g,9000,valid_from,valid_to,op FROM facts", []).unwrap();
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM facts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let actual = fetch_labeled_entities(&store).unwrap();
    assert_eq!(actual.len(), 1);
    assert_eq!(sorted(actual), sorted(reference(&store, None).unwrap()));
}

#[test]
fn attached_aliases_deduplicate_without_admitting_layer_labels() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.db");
    let layer = dir.path().join("layer.db");
    let packed = dir.path().join("packed.db");
    let seed = |path: &std::path::Path, text: &str| {
        let mut store = Store::open(&path.to_string_lossy()).unwrap();
        let e = store.intern("urn:test:shared").unwrap();
        let label = store
            .intern("http://www.w3.org/2000/01/rdf-schema#label")
            .unwrap();
        let ty = store
            .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
            .unwrap();
        let kind = store.intern("urn:test:kind").unwrap();
        let datums: Vec<_> = [(label, Value::Str(text.into())), (ty, Value::Ref(kind))]
            .into_iter()
            .map(|(attribute, value)| Datum {
                entity: e,
                attribute,
                value,
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            })
            .collect();
        store.transact(&datums, "2026-01-01", None, None).unwrap();
        (e, label, ty, kind)
    };
    let (entity, label, ty, kind) = seed(&main, "visible");
    seed(&layer, "hidden");
    crate::store::respace::respace_file(&layer, &packed, 7).unwrap();
    let mut store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[crate::store::attach::Attachment::read_only(
            "packed",
            &packed.to_string_lossy(),
        )],
    )
    .unwrap();
    let alias = |id| {
        store
            .conn
            .query_row(
                "SELECT alias_id FROM term_alias WHERE canonical_id = ?1",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    let other_entity = alias(entity);
    let other_kind = alias(kind);
    assert_ne!(entity, other_entity);
    let datums: Vec<_> = [
        (label, Value::Str("visible".into())),
        (ty, Value::Ref(other_kind)),
    ]
    .into_iter()
    .map(|(attribute, value)| Datum {
        entity: other_entity,
        attribute,
        value,
        valid_from: "2026-01-02".into(),
        valid_to: None,
        op: Op::Assert,
    })
    .collect();
    store.transact(&datums, "2026-01-02", None, None).unwrap();
    let actual = fetch_labeled_entities(&store).unwrap();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].label.as_ref(), "visible");
    assert_eq!(sorted(actual), sorted(reference(&store, None).unwrap()));
}
