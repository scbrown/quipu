//! Tests for the fact log store.

use super::*;
use crate::types::{Op, Value};

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

#[test]
fn intern_and_resolve() {
    let store = test_store();
    let id = store.intern("http://example.org/Person").unwrap();
    assert!(id > 0);
    let iri = store.resolve(id).unwrap();
    assert_eq!(iri, "http://example.org/Person");

    let id2 = store.intern("http://example.org/Person").unwrap();
    assert_eq!(id, id2);
}

#[test]
fn lookup_missing() {
    let store = test_store();
    assert_eq!(store.lookup("http://nope").unwrap(), None);
}

#[test]
fn round_trip_write_read() {
    let mut store = test_store();

    let e = store.intern("http://example.org/alice").unwrap();
    let a_name = store.intern("http://example.org/name").unwrap();
    let a_age = store.intern("http://example.org/age").unwrap();

    let tx = store
        .transact(
            &[
                Datum {
                    entity: e,
                    attribute: a_name,
                    value: Value::Str("Alice".into()),
                    valid_from: "2026-01-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: e,
                    attribute: a_age,
                    value: Value::Int(30),
                    valid_from: "2026-01-01".into(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            "2026-04-04T00:00:00Z",
            Some("test"),
            Some("unit-test"),
        )
        .unwrap();

    assert!(tx > 0);

    let facts = store.current_facts().unwrap();
    assert_eq!(facts.len(), 2);
    let values: Vec<&Value> = facts.iter().map(|f| &f.value).collect();
    assert!(values.contains(&&Value::Str("Alice".into())));
    assert!(values.contains(&&Value::Int(30)));

    let efacts = store.entity_facts(e).unwrap();
    assert_eq!(efacts.len(), 2);

    let txn = store.get_transaction(tx).unwrap().unwrap();
    assert_eq!(txn.actor.as_deref(), Some("test"));
    assert_eq!(txn.source.as_deref(), Some("unit-test"));
}

#[test]
fn temporal_query() {
    let mut store = test_store();

    let e = store.intern("http://example.org/server").unwrap();
    let a = store.intern("http://example.org/status").unwrap();

    let tx1 = store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("active".into()),
                valid_from: "2026-01-01".into(),
                valid_to: Some("2026-03-01".into()),
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let _tx2 = store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("decommissioned".into()),
                valid_from: "2026-03-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-03-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let facts = store
        .facts_as_of(&AsOf {
            tx: Some(tx1),
            valid_at: None,
        })
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].value, Value::Str("active".into()));

    let facts = store
        .facts_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-02-01".into()),
        })
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].value, Value::Str("active".into()));

    let facts = store
        .facts_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-04-01".into()),
        })
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].value, Value::Str("decommissioned".into()));

    let current = store.current_facts().unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].value, Value::Str("decommissioned".into()));
}

#[test]
fn contradiction_detection() {
    let mut store = test_store();

    let e = store.intern("http://example.org/node").unwrap();
    let a = store.intern("http://example.org/ip").unwrap();

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("10.0.0.1".into()),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("10.0.0.2".into()),
                valid_from: "2026-02-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-02-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let contradictions = store.detect_contradictions(e, a).unwrap();
    assert_eq!(contradictions.len(), 1);
    assert_eq!(contradictions[0].0.value, Value::Str("10.0.0.1".into()));
    assert_eq!(contradictions[0].1.value, Value::Str("10.0.0.2".into()));
}

#[test]
fn attribute_history_tracks_all_ops() {
    let mut store = test_store();

    let e = store.intern("http://example.org/svc").unwrap();
    let a = store.intern("http://example.org/port").unwrap();

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Int(8080),
                valid_from: "2026-01-01".into(),
                valid_to: Some("2026-02-01".into()),
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Int(9090),
                valid_from: "2026-02-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-02-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let history = store.attribute_history(e, a).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].value, Value::Int(8080));
    assert_eq!(history[1].value, Value::Int(9090));
}

#[test]
fn value_round_trip() {
    let cases = vec![
        Value::Ref(42),
        Value::Str("hello world".into()),
        Value::Int(-999),
        Value::Float(3.25),
        Value::Bool(true),
        Value::Bool(false),
        Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]),
    ];
    for v in cases {
        let bytes = v.to_bytes();
        let decoded = Value::from_bytes(&bytes).unwrap();
        assert_eq!(v, decoded, "round-trip failed for {v:?}");
    }
}

#[test]
fn speculate_rolls_back() {
    let mut store = test_store();
    let e = store.intern("http://ex/a").unwrap();
    let a = store.intern("http://ex/p").unwrap();
    let o = store.intern("http://ex/b").unwrap();

    // Assert an edge a→b.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Ref(o),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    assert_eq!(store.current_facts().unwrap().len(), 1);

    // Speculatively retract the edge.
    let retract = Datum {
        entity: e,
        attribute: a,
        value: Value::Ref(o),
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Retract,
    };

    let inside_count = store
        .speculate(&[retract], "2026-02-01T00:00:00Z", |s| {
            // Inside: the edge should be retracted.
            let facts = s.current_facts()?;
            Ok(facts.len())
        })
        .unwrap();

    assert_eq!(inside_count, 0, "edge should be gone inside speculate");

    // After: the edge is back.
    assert_eq!(
        store.current_facts().unwrap().len(),
        1,
        "edge should be restored after speculate"
    );
}

#[test]
fn speculate_hypothetical_visible_inside() {
    let mut store = test_store();
    let e = store.intern("http://ex/x").unwrap();
    let a = store.intern("http://ex/label").unwrap();

    // Start empty. Speculatively assert a fact.
    let assert_datum = Datum {
        entity: e,
        attribute: a,
        value: Value::Str("speculative".into()),
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Assert,
    };

    let inside_count = store
        .speculate(&[assert_datum], "2026-01-01T00:00:00Z", |s| {
            Ok(s.current_facts()?.len())
        })
        .unwrap();

    assert_eq!(inside_count, 1, "speculative fact should be visible inside");
    assert_eq!(
        store.current_facts().unwrap().len(),
        0,
        "speculative fact must not persist"
    );
}

#[test]
fn speculate_error_still_rolls_back() {
    let mut store = test_store();
    let e = store.intern("http://ex/y").unwrap();
    let a = store.intern("http://ex/tag").unwrap();

    let datum = Datum {
        entity: e,
        attribute: a,
        value: Value::Str("temp".into()),
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Assert,
    };

    let result: crate::error::Result<()> =
        store.speculate(&[datum], "2026-01-01T00:00:00Z", |_s| {
            Err(crate::error::Error::InvalidValue(
                "intentional test error".into(),
            ))
        });

    assert!(result.is_err());
    assert_eq!(
        store.current_facts().unwrap().len(),
        0,
        "speculative state must be rolled back even on error"
    );
}

#[test]
fn retract_hides_from_current() {
    let mut store = test_store();

    let e = store.intern("http://example.org/thing").unwrap();
    let a = store.intern("http://example.org/label").unwrap();

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("old-label".into()),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    assert_eq!(store.current_facts().unwrap().len(), 1);

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("old-label".into()),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Retract,
            }],
            "2026-02-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let current = store.current_facts().unwrap();
    assert_eq!(
        current.len(),
        0,
        "retracted fact should not appear in current state"
    );

    let history = store.attribute_history(e, a).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].op, Op::Assert);
    assert_eq!(history[0].valid_to, Some("2026-02-01T00:00:00Z".into()));
    assert_eq!(history[1].op, Op::Retract);

    let before_retract = store
        .facts_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-01-15".into()),
        })
        .unwrap();
    assert_eq!(before_retract.len(), 1);
    assert_eq!(before_retract[0].value, Value::Str("old-label".into()));

    let after_retract = store
        .facts_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-03-01".into()),
        })
        .unwrap();
    assert_eq!(after_retract.len(), 0);
}

#[test]
fn duplicate_assert_across_transactions_is_idempotent() {
    let mut store = test_store();

    let e = store.intern("http://example.org/ct-244").unwrap();
    let a = store
        .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
        .unwrap();
    let v = Value::Str("LXCContainer".into());

    // First assertion — should write.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            Some("ep1"),
            Some("episode:scan-1"),
        )
        .unwrap();

    assert_eq!(store.current_facts().unwrap().len(), 1);

    // Second assertion of same (e, a, v) in a different transaction — should be skipped.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-02-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-02-01T00:00:00Z",
            Some("ep2"),
            Some("episode:scan-2"),
        )
        .unwrap();

    let facts = store.current_facts().unwrap();
    assert_eq!(
        facts.len(),
        1,
        "duplicate assertion should not create a second row"
    );

    // The original fact should remain unchanged.
    assert_eq!(facts[0].valid_from, "2026-01-01");
}

#[test]
fn retract_then_reassert_creates_new_fact() {
    let mut store = test_store();

    let e = store.intern("http://example.org/svc").unwrap();
    let a = store.intern("http://example.org/status").unwrap();
    let v = Value::Str("active".into());

    // Assert.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    assert_eq!(store.current_facts().unwrap().len(), 1);

    // Retract.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Retract,
            }],
            "2026-02-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    assert_eq!(store.current_facts().unwrap().len(), 0);

    // Re-assert same (e, a, v) — should succeed since the old one was retracted.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-03-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-03-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let facts = store.current_facts().unwrap();
    assert_eq!(
        facts.len(),
        1,
        "re-assertion after retract should create a new fact"
    );
    assert_eq!(facts[0].valid_from, "2026-03-01");
}

#[test]
fn within_transaction_dedup() {
    let mut store = test_store();

    let e = store.intern("http://example.org/node").unwrap();
    let a = store.intern("http://example.org/label").unwrap();
    let v = Value::Str("test".into());

    // Two identical datums in the same transaction — second should hit PK conflict.
    // The first is written; the second is a duplicate within the same tx so the PK
    // constraint handles it. We just need to make sure it doesn't panic.
    let result = store.transact(
        &[
            Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: e,
                attribute: a,
                value: v.clone(),
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            },
        ],
        "2026-01-01T00:00:00Z",
        None,
        None,
    );

    // The first datum writes fine. The second is skipped by the exists check
    // (the first one is now visible within the savepoint).
    assert!(result.is_ok());
    assert_eq!(store.current_facts().unwrap().len(), 1);
}
