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

// -- Episode-scoped logical retraction (aegis-hxb) --

/// Write one (e, a, v) fact under a transaction tagged `source="episode:{name}"`,
/// mirroring how `episode::ingest_episode` stamps its writes.
fn assert_episode_fact(store: &mut Store, name: &str, e: i64, a: i64, v: Value) {
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: v,
                valid_from: "2026-01-01".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            Some(&format!("episode:{name}")),
        )
        .unwrap();
}

#[test]
fn retract_episode_removes_only_that_episodes_facts() {
    let mut store = test_store();
    let alice = store.intern("http://example.org/alice").unwrap();
    let bob = store.intern("http://example.org/bob").unwrap();
    let name = store.intern("http://example.org/name").unwrap();

    assert_episode_fact(&mut store, "ep-a", alice, name, Value::Str("Alice".into()));
    assert_episode_fact(&mut store, "ep-b", bob, name, Value::Str("Bob".into()));
    assert_eq!(store.current_facts().unwrap().len(), 2);

    let (tx_id, retracted) = store
        .retract_episode("ep-a", "2026-02-01T00:00:00Z", Some("tester"))
        .unwrap();
    assert!(tx_id > 0);
    assert_eq!(retracted.len(), 1);
    assert_eq!(retracted[0].entity, alice);

    // ep-a's fact left the current view; ep-b's survived untouched.
    let current = store.current_facts().unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].entity, bob);
}

#[test]
fn retract_episode_preserves_history() {
    let mut store = test_store();
    let alice = store.intern("http://example.org/alice").unwrap();
    let name = store.intern("http://example.org/name").unwrap();
    assert_episode_fact(&mut store, "ep-a", alice, name, Value::Str("Alice".into()));

    store
        .retract_episode("ep-a", "2026-02-01T00:00:00Z", None)
        .unwrap();

    // Logical, not physical: the original assertion row still exists (now closed)
    // alongside the new retract row, so time-travel history is intact.
    let history = store.entity_history(alice).unwrap();
    assert_eq!(history.len(), 2, "assert + retract rows both retained");
    assert!(
        history
            .iter()
            .any(|f| f.op == Op::Assert && f.valid_to.is_some())
    );
    assert!(history.iter().any(|f| f.op == Op::Retract));

    // The fact as it was before the retraction is still visible via time-travel.
    let before = store
        .facts_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-01-15T00:00:00Z".into()),
        })
        .unwrap();
    assert!(before.iter().any(|f| f.entity == alice));
}

#[test]
fn retract_episode_is_idempotent() {
    let mut store = test_store();
    let alice = store.intern("http://example.org/alice").unwrap();
    let name = store.intern("http://example.org/name").unwrap();
    assert_episode_fact(&mut store, "ep-a", alice, name, Value::Str("Alice".into()));

    let (_, first) = store
        .retract_episode("ep-a", "2026-02-01T00:00:00Z", None)
        .unwrap();
    assert_eq!(first.len(), 1);

    // Second retraction finds nothing active: no-op.
    let (tx_id, second) = store
        .retract_episode("ep-a", "2026-03-01T00:00:00Z", None)
        .unwrap();
    assert_eq!(tx_id, crate::episode::NOOP_TX);
    assert!(second.is_empty());
}

#[test]
fn retract_episode_unknown_is_noop() {
    let mut store = test_store();
    let (tx_id, retracted) = store
        .retract_episode("never-ingested", "2026-02-01T00:00:00Z", None)
        .unwrap();
    assert_eq!(tx_id, crate::episode::NOOP_TX);
    assert!(retracted.is_empty());
}

#[test]
fn named_graph_migration_is_additive_and_defaults_existing_facts_to_root() {
    // aegis-g1al / #36. A store created before the `g` column must migrate
    // additively: existing facts land in g=0 (ROOT, source of truth), un-mutated,
    // and a fresh store has the column from the start. No table rebuild.
    use rusqlite::Connection;

    // Simulate an OLD store: facts table WITHOUT the g column, with a fact in it.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE terms (id INTEGER PRIMARY KEY, iri TEXT NOT NULL UNIQUE);
         CREATE TABLE transactions (id INTEGER PRIMARY KEY, ts TEXT);
         CREATE TABLE facts (e INTEGER NOT NULL, a INTEGER NOT NULL, v BLOB NOT NULL,
             tx INTEGER NOT NULL, valid_from TEXT NOT NULL, valid_to TEXT,
             op INTEGER NOT NULL DEFAULT 1, PRIMARY KEY (e,a,v,tx));
         INSERT INTO transactions (id, ts) VALUES (1, '2026-01-01T00:00:00Z');
         INSERT INTO facts (e,a,v,tx,valid_from) VALUES (10, 20, X'30', 1, '2026-01-01T00:00:00Z');",
    )
    .unwrap();

    // Pre-migration: no g column.
    let has_g_before: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('facts') WHERE name='g'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(!has_g_before, "precondition: old store has no g column");

    Store::migrate_named_graphs(&conn).unwrap();

    // Post-migration: g exists, the existing fact survived, and it's in ROOT (0).
    let has_g_after: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('facts') WHERE name='g'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(has_g_after, "migration must add the g column");
    let (cnt, g): (i64, i64) = conn
        .query_row("SELECT COUNT(*), MIN(g) FROM facts WHERE e=10 AND a=20", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(cnt, 1, "existing fact must survive the migration (no data loss)");
    assert_eq!(g, 0, "existing facts must default to ROOT (g=0), un-mutated");

    // Idempotent: running it again is a no-op, not an error.
    Store::migrate_named_graphs(&conn).unwrap();
}

#[test]
fn open_migrates_a_pre_quad_store_through_the_real_init_path() {
    // Regression for aegis-akb8. The test above calls migrate_named_graphs()
    // DIRECTLY, so it never runs schema::INIT_SQL first — and INIT_SQL is where
    // the real bug lived: its `CREATE INDEX ... ON facts(g, ...)` executed
    // against a pre-quad `facts` table (CREATE TABLE IF NOT EXISTS is a no-op)
    // and hard-failed with `no such column: g` BEFORE the migration's ALTER
    // could add the column. Store::open crash-looped on the live db; the direct
    // test stayed green. This exercises the actual open() sequence on disk.
    use rusqlite::Connection;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prequad.db");
    let path_str = path.to_str().unwrap();

    // A store as it existed before #36: facts table WITHOUT g, holding a fact.
    {
        let conn = Connection::open(path_str).unwrap();
        conn.execute_batch(
            "CREATE TABLE terms (id INTEGER PRIMARY KEY, iri TEXT NOT NULL UNIQUE);
             CREATE TABLE transactions (id INTEGER PRIMARY KEY, timestamp TEXT NOT NULL, actor TEXT, source TEXT);
             CREATE TABLE facts (e INTEGER NOT NULL, a INTEGER NOT NULL, v BLOB NOT NULL,
                 tx INTEGER NOT NULL REFERENCES transactions(id), valid_from TEXT NOT NULL,
                 valid_to TEXT, op INTEGER NOT NULL DEFAULT 1, PRIMARY KEY (e,a,v,tx));
             INSERT INTO transactions (id, timestamp) VALUES (1, '2026-01-01T00:00:00Z');
             INSERT INTO facts (e,a,v,tx,valid_from) VALUES (10, 20, X'30', 1, '2026-01-01T00:00:00Z');",
        )
        .unwrap();
    }

    // THE regression: this used to return `no such column: g` and abort open.
    let store =
        Store::open(path_str).expect("open() must migrate a pre-quad store, not crash on idx_geav");

    // g column present, idx_geav present, and the pre-existing fact survived in ROOT.
    let has_idx: bool = store
        .conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_geav'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(has_idx, "idx_geav must be created when open() migrates a pre-quad store");
    let (cnt, g): (i64, i64) = store
        .conn
        .query_row("SELECT COUNT(*), MIN(g) FROM facts WHERE e=10 AND a=20", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(cnt, 1, "existing fact must survive the migration (no data loss)");
    assert_eq!(g, 0, "existing fact must land in ROOT (g=0), un-mutated");

    // Idempotent through the real path: re-opening the migrated store is clean.
    Store::open(path_str).expect("re-open of an already-migrated store must be a no-op");
}

#[test]
fn overlay_create_binds_once_and_rejects_rebind() {
    let store = Store::open_in_memory().unwrap();
    let a = store.intern("http://ex/graph/a").unwrap();
    // create over ROOT (g=0) is fine; idempotent on identical parent.
    let g1 = store.overlay_create("http://ex/graph/a", 0).unwrap();
    let g2 = store.overlay_create("http://ex/graph/a", 0).unwrap();
    assert_eq!(g1, a);
    assert_eq!(g1, g2, "re-create with same parent is idempotent");
    assert_eq!(store.graph_class(g1).unwrap().as_deref(), Some("overlay"));
    assert_eq!(store.overlay_parent(g1).unwrap(), 0);
    // a committed branch to rebind against
    let branch = store.intern("http://ex/graph/branch").unwrap();
    store
        .conn
        .execute(
            "INSERT INTO graphs (g, class, parent_branch, created_at) VALUES (?1,'committed',NULL,'t')",
            [branch],
        )
        .unwrap();
    // rebind to a different parent must error (bind-once).
    assert!(store.overlay_create("http://ex/graph/a", branch).is_err());
    // overlay cannot extend a non-committed (overlay) parent.
    assert!(store.overlay_create("http://ex/graph/b", g1).is_err());
    // unregistered graph is not an overlay.
    assert!(store.overlay_parent(branch).is_err());
}

#[test]
fn compose_view_resolves_assert_tombstone_fallthrough_without_touching_root() {
    // The uniform rule: present iff asserted AND not tombstoned, nearest-overlay-wins.
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://ex/svc").unwrap();
    let keep = store.intern("http://ex/keep").unwrap(); // root attr the overlay leaves alone
    let hide = store.intern("http://ex/hide").unwrap(); // root attr the overlay tombstones
    let add = store.intern("http://ex/add").unwrap(); // attr the overlay asserts fresh
    let d = |a: i64, v: &str, op| Datum {
        entity: e,
        attribute: a,
        value: Value::Str(v.to_string()),
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op,
    };
    // ROOT holds two triples.
    store
        .transact(
            &[d(keep, "K", Op::Assert), d(hide, "H", Op::Assert)],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let ov = store.overlay_create("http://ex/graph/tenant-1", 0).unwrap();
    // overlay: assert a new triple, and TOMBSTONE the root's (e,hide,"H").
    store
        .overlay_write(ov, Op::Assert, e, add, Value::Str("A".into()), "2026-01-02T00:00:00Z")
        .unwrap();
    store
        .overlay_write(ov, Op::Tombstone, e, hide, Value::Str("H".into()), "2026-01-02T00:00:00Z")
        .unwrap();

    // Composed view = { (e,keep,K) fell through, (e,add,A) overlay assert }. hide is gone.
    let view = store.compose_view(ov).unwrap();
    let attrs: std::collections::BTreeSet<i64> = view.iter().map(|f| f.attribute).collect();
    assert!(attrs.contains(&keep), "root triple the overlay ignores falls through");
    assert!(attrs.contains(&add), "overlay assertion is present");
    assert!(!attrs.contains(&hide), "root triple the overlay tombstones is hidden");
    assert_eq!(view.len(), 2, "exactly keep + add, no duplicates");

    // ROOT is un-mutated: a plain read of g=0 still sees both original triples,
    // including the tombstoned one — the tombstone is view-only.
    let root_hide: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL",
            [e, hide],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(root_hide, 1, "tombstone must NOT touch the root fact (base un-mutated)");

    // Tombstone is idempotent (second one is a no-op, no error).
    store
        .overlay_write(ov, Op::Tombstone, e, hide, Value::Str("H".into()), "2026-01-03T00:00:00Z")
        .unwrap();
    let tomb_count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=?3 AND op=2 AND valid_to IS NULL",
            [e, hide, ov],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tomb_count, 1, "tombstone is idempotent");

    // overlay_write rejects a non-overlay target graph.
    assert!(store
        .overlay_write(0, Op::Assert, e, add, Value::Str("x".into()), "t")
        .is_err());
}

#[test]
fn overlay_writes_do_not_touch_root() {
    // aegis-g1al / #36, Stiwi's core requirement: an overlay extends the base
    // WITHOUT mutating it. Writing the same (e,a,v) to ROOT and to an overlay
    // must yield two independent facts; an overlay's retract must NOT close the
    // ROOT fact. Enforced by graph-scoped idempotency + retract in transact.
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/svc").unwrap();
    let a = store.intern("http://example.org/status").unwrap();
    let overlay = store.intern("http://example.org/graph/tenant-1").unwrap();
    let d = |v: &str, op| Datum {
        entity: e, attribute: a, value: Value::Str(v.to_string()),
        valid_from: "2026-01-01T00:00:00Z".into(), valid_to: None, op,
    };

    // 1. Assert (e,a,"up") into ROOT.
    store.transact(&[d("up", Op::Assert)], "2026-01-01T00:00:00Z", None, None).unwrap();
    // 2. Assert the SAME (e,a,"up") into an overlay — must NOT be skipped as a dup.
    store.transact_to_graph(&[d("up", Op::Assert)], "2026-01-02T00:00:00Z", None, None, overlay).unwrap();

    let root_up: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL", [e,a], |r| r.get(0)).unwrap();
    let ov_up: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=?3 AND op=1 AND valid_to IS NULL", [e,a,overlay], |r| r.get(0)).unwrap();
    assert_eq!(root_up, 1, "ROOT keeps its fact");
    assert_eq!(ov_up, 1, "overlay gets its OWN fact — not skipped as a dup of ROOT");

    // 3. Retract in the OVERLAY. Must close only the overlay row; ROOT untouched.
    store.transact_to_graph(&[d("up", Op::Retract)], "2026-01-03T00:00:00Z", None, None, overlay).unwrap();
    let root_after: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL", [e,a], |r| r.get(0)).unwrap();
    let ov_after: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=?3 AND op=1 AND valid_to IS NULL", [e,a,overlay], |r| r.get(0)).unwrap();
    assert_eq!(root_after, 1, "overlay retract must NOT close the ROOT fact (base un-mutated)");
    assert_eq!(ov_after, 0, "overlay retract closes only its own graph's assertion");
}

#[test]
fn compose_view_dedupes_reasserted_root_facts() {
    // Regression (found in the 69co live deploy): a base fact re-asserted across
    // transactions leaves multiple current op=1 rows; compose_view must return
    // the composed triple ONCE, not once per assertion.
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://ex/s").unwrap();
    let a = store.intern("http://ex/p").unwrap();
    store
        .transact(
            &[Datum { entity: e, attribute: a, value: Value::Str("K".into()),
                valid_from: "2026-01-01T00:00:00Z".into(), valid_to: None, op: Op::Assert }],
            "2026-01-01T00:00:00Z", None, None)
        .unwrap();
    // Force a DUPLICATE current row in ROOT (as ingest/history produce in the
    // live db), bypassing transact's idempotency.
    let vb = Value::Str("K".into()).to_bytes();
    store.conn.execute(
        "INSERT INTO transactions (timestamp) VALUES ('2026-01-02T00:00:00Z')", []).unwrap();
    let tx2 = store.conn.last_insert_rowid();
    store.conn.execute(
        "INSERT INTO facts (e,a,v,g,tx,valid_from,valid_to,op) VALUES (?1,?2,?3,0,?4,'2026-01-02T00:00:00Z',NULL,1)",
        rusqlite::params![e, a, vb, tx2]).unwrap();
    let dup_count: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL",
        [e, a], |r| r.get(0)).unwrap();
    assert_eq!(dup_count, 2, "precondition: two current rows for the same triple");

    let ov = store.overlay_create("http://ex/g/t", 0).unwrap();
    let view = store.compose_view(ov).unwrap();
    let matches = view.iter().filter(|f| f.entity == e && f.attribute == a).count();
    assert_eq!(matches, 1, "compose must dedupe the re-asserted base fact to one triple");
}
