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
                value: Value::Str("192.0.2.1".into()),
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
                value: Value::Str("192.0.2.2".into()),
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
    assert_eq!(contradictions[0].0.value, Value::Str("192.0.2.1".into()));
    assert_eq!(contradictions[0].1.value, Value::Str("192.0.2.2".into()));
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

// -- Ghost nodes: identity survival across episode retraction (aegis-arup) --

use crate::store::ops::OrphanPolicy;

/// maldoon's measured specimen, reduced: a node whose identity (`rdfs:label` +
/// `rdf:type`) was declared in episode A, with an inbound edge from episode B.
/// Returns `(store, node, label_pred, type_pred, comment_pred, edge_pred, other)`.
#[allow(clippy::type_complexity)]
fn ghost_fixture() -> (Store, i64, i64, i64, i64, i64, i64) {
    let mut store = test_store();
    let node = store.intern("http://example.org/ty4h").unwrap();
    let other = store.intern("http://example.org/lnmc").unwrap();
    let bead = store.intern("http://example.org/Bead").unwrap();
    let label = store.intern(crate::namespace::RDFS_LABEL).unwrap();
    let rdf_type = store.intern(crate::namespace::RDF_TYPE).unwrap();
    let comment = store
        .intern("http://www.w3.org/2000/01/rdf-schema#comment")
        .unwrap();
    let applies_to = store.intern("http://example.org/applies_to").unwrap();

    // Episode A declares the node's identity plus an ordinary fact.
    assert_episode_fact(&mut store, "ep-a", node, label, Value::Str("ty4h".into()));
    assert_episode_fact(&mut store, "ep-a", node, rdf_type, Value::Ref(bead));
    assert_episode_fact(
        &mut store,
        "ep-a",
        node,
        comment,
        Value::Str("notes".into()),
    );
    // Episode B adds an inbound edge — this is what keeps the node in the graph.
    assert_episode_fact(&mut store, "ep-b", other, applies_to, Value::Ref(node));

    (store, node, label, rdf_type, comment, applies_to, other)
}

fn has_active(store: &Store, e: i64, a: i64) -> bool {
    store
        .entity_facts(e)
        .unwrap()
        .iter()
        .any(|f| f.attribute == a)
}

#[test]
fn retract_episode_preserves_identity_of_still_referenced_nodes() {
    let (mut store, node, label, rdf_type, comment, applies_to, other) = ghost_fixture();

    let outcome = store
        .retract_episode_with_policy("ep-a", "2026-02-01T00:00:00Z", None, OrphanPolicy::Preserve)
        .unwrap();

    // The node keeps its NAME and TYPE: it is still findable by a label scan and
    // by `?s a ex:Bead`, which is the whole point (aegis-arup).
    assert!(has_active(&store, node, label), "rdfs:label must survive");
    assert!(has_active(&store, node, rdf_type), "rdf:type must survive");
    // Non-identity facts from the retracted episode are still gone...
    assert!(!has_active(&store, node, comment));
    // ...and the other episode's edge is untouched.
    assert!(has_active(&store, other, applies_to));

    assert_eq!(outcome.retracted.len(), 1, "only the comment was retracted");
    assert_eq!(outcome.preserved_identity.len(), 2);
    assert_eq!(outcome.orphans.len(), 1);
    assert_eq!(outcome.orphans[0].entity, node);
    assert!(outcome.orphans[0].lost_label && outcome.orphans[0].lost_type);
}

#[test]
fn retract_episode_allow_still_creates_the_ghost_positive_control() {
    // DISCRIMINATION (ellie's z0xi standard): the assertions above must FAIL
    // against the old behaviour, or they prove nothing. `OrphanPolicy::Allow` IS
    // the old behaviour — strict episode scope, not attribute-aware — so this
    // test pins the bug that `preserve` fixes. If a future change made identity
    // survive unconditionally, this test breaks and the guard above becomes
    // vacuous without anyone noticing.
    let (mut store, node, label, rdf_type, _comment, applies_to, other) = ghost_fixture();

    let outcome = store
        .retract_episode_with_policy("ep-a", "2026-02-01T00:00:00Z", None, OrphanPolicy::Allow)
        .unwrap();

    assert!(!has_active(&store, node, label), "the ghost has no name");
    assert!(!has_active(&store, node, rdf_type), "and no type");
    assert!(
        has_active(&store, other, applies_to),
        "yet it is still reachable by edge — present AND unfindable"
    );
    assert_eq!(outcome.retracted.len(), 3);
    assert!(outcome.preserved_identity.is_empty());
    // Even when it ghosts, the API no longer stays silent about it.
    assert_eq!(outcome.orphans.len(), 1);
    assert_eq!(outcome.orphans[0].entity, node);
}

#[test]
fn retract_episode_refuse_rejects_the_whole_retraction() {
    let (mut store, node, label, rdf_type, comment, _applies_to, _other) = ghost_fixture();
    let before = store.current_facts().unwrap().len();

    let err = store
        .retract_episode_with_policy("ep-a", "2026-02-01T00:00:00Z", None, OrphanPolicy::Refuse)
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("ty4h"), "names the node at risk: {msg}");

    // Nothing was written: refuse means refuse, not partially applied.
    assert_eq!(store.current_facts().unwrap().len(), before);
    assert!(has_active(&store, node, label));
    assert!(has_active(&store, node, rdf_type));
    assert!(has_active(&store, node, comment));
}

#[test]
fn retract_episode_removes_unreferenced_nodes_whole_identity_included() {
    // A node NOBODY else references is not a ghost risk — it leaves the graph
    // entirely. Preserving its label would leave a stub, which is its own kind
    // of debris.
    let mut store = test_store();
    let node = store.intern("http://example.org/solo").unwrap();
    let cls = store.intern("http://example.org/Thing").unwrap();
    let label = store.intern(crate::namespace::RDFS_LABEL).unwrap();
    let rdf_type = store.intern(crate::namespace::RDF_TYPE).unwrap();
    assert_episode_fact(&mut store, "ep-a", node, label, Value::Str("solo".into()));
    assert_episode_fact(&mut store, "ep-a", node, rdf_type, Value::Ref(cls));

    let outcome = store
        .retract_episode_with_policy("ep-a", "2026-02-01T00:00:00Z", None, OrphanPolicy::Preserve)
        .unwrap();

    assert_eq!(outcome.retracted.len(), 2);
    assert!(outcome.preserved_identity.is_empty());
    assert!(outcome.orphans.is_empty());
    assert!(store.entity_facts(node).unwrap().is_empty());
}

#[test]
fn retract_episode_does_not_preserve_identity_another_episode_also_asserts() {
    // If episode B independently gives the node a label, ours is not its only
    // name — retracting ours orphans nothing, so it goes.
    let mut store = test_store();
    let node = store.intern("http://example.org/dual").unwrap();
    let label = store.intern(crate::namespace::RDFS_LABEL).unwrap();
    assert_episode_fact(&mut store, "ep-a", node, label, Value::Str("from-a".into()));
    assert_episode_fact(&mut store, "ep-b", node, label, Value::Str("from-b".into()));

    let outcome = store
        .retract_episode_with_policy("ep-a", "2026-02-01T00:00:00Z", None, OrphanPolicy::Preserve)
        .unwrap();

    assert!(outcome.orphans.is_empty());
    assert!(outcome.preserved_identity.is_empty());
    assert_eq!(outcome.retracted.len(), 1);
    let labels: Vec<_> = store
        .entity_facts(node)
        .unwrap()
        .into_iter()
        .filter(|f| f.attribute == label)
        .collect();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].value, Value::Str("from-b".into()));
}

#[test]
fn retract_episode_preserving_is_still_idempotent() {
    let (mut store, node, label, ..) = ghost_fixture();

    store
        .retract_episode("ep-a", "2026-02-01T00:00:00Z", None)
        .unwrap();
    // The preserved identity facts are still tagged to ep-a, so a second
    // retraction re-selects them — and preserves them again rather than
    // finishing the job the first call declined to do.
    let (tx_id, retracted) = store
        .retract_episode("ep-a", "2026-03-01T00:00:00Z", None)
        .unwrap();
    assert_eq!(tx_id, crate::episode::NOOP_TX);
    assert!(retracted.is_empty());
    assert!(has_active(&store, node, label));
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
        .query_row(
            "SELECT COUNT(*), MIN(g) FROM facts WHERE e=10 AND a=20",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        cnt, 1,
        "existing fact must survive the migration (no data loss)"
    );
    assert_eq!(
        g, 0,
        "existing facts must default to ROOT (g=0), un-mutated"
    );

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
    assert!(
        has_idx,
        "idx_geav must be created when open() migrates a pre-quad store"
    );
    let (cnt, g): (i64, i64) = store
        .conn
        .query_row(
            "SELECT COUNT(*), MIN(g) FROM facts WHERE e=10 AND a=20",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        cnt, 1,
        "existing fact must survive the migration (no data loss)"
    );
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
        .overlay_write(
            ov,
            Op::Assert,
            e,
            add,
            Value::Str("A".into()),
            "2026-01-02T00:00:00Z",
        )
        .unwrap();
    store
        .overlay_write(
            ov,
            Op::Tombstone,
            e,
            hide,
            Value::Str("H".into()),
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

    // Composed view = { (e,keep,K) fell through, (e,add,A) overlay assert }. hide is gone.
    let view = store.compose_view(ov).unwrap();
    let attrs: std::collections::BTreeSet<i64> = view.iter().map(|f| f.attribute).collect();
    assert!(
        attrs.contains(&keep),
        "root triple the overlay ignores falls through"
    );
    assert!(attrs.contains(&add), "overlay assertion is present");
    assert!(
        !attrs.contains(&hide),
        "root triple the overlay tombstones is hidden"
    );
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
    assert_eq!(
        root_hide, 1,
        "tombstone must NOT touch the root fact (base un-mutated)"
    );

    // Tombstone is idempotent (second one is a no-op, no error).
    store
        .overlay_write(
            ov,
            Op::Tombstone,
            e,
            hide,
            Value::Str("H".into()),
            "2026-01-03T00:00:00Z",
        )
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
    assert!(
        store
            .overlay_write(0, Op::Assert, e, add, Value::Str("x".into()), "t")
            .is_err()
    );
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
        entity: e,
        attribute: a,
        value: Value::Str(v.to_string()),
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op,
    };

    // 1. Assert (e,a,"up") into ROOT.
    store
        .transact(&[d("up", Op::Assert)], "2026-01-01T00:00:00Z", None, None)
        .unwrap();
    // 2. Assert the SAME (e,a,"up") into an overlay — must NOT be skipped as a dup.
    store
        .transact_to_graph(
            &[d("up", Op::Assert)],
            "2026-01-02T00:00:00Z",
            None,
            None,
            overlay,
        )
        .unwrap();

    let root_up: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL",
            [e, a],
            |r| r.get(0),
        )
        .unwrap();
    let ov_up: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=?3 AND op=1 AND valid_to IS NULL",
            [e, a, overlay],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(root_up, 1, "ROOT keeps its fact");
    assert_eq!(
        ov_up, 1,
        "overlay gets its OWN fact — not skipped as a dup of ROOT"
    );

    // 3. Retract in the OVERLAY. Must close only the overlay row; ROOT untouched.
    store
        .transact_to_graph(
            &[d("up", Op::Retract)],
            "2026-01-03T00:00:00Z",
            None,
            None,
            overlay,
        )
        .unwrap();
    let root_after: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL",
            [e, a],
            |r| r.get(0),
        )
        .unwrap();
    let ov_after: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=?3 AND op=1 AND valid_to IS NULL",
            [e, a, overlay],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        root_after, 1,
        "overlay retract must NOT close the ROOT fact (base un-mutated)"
    );
    assert_eq!(
        ov_after, 0,
        "overlay retract closes only its own graph's assertion"
    );
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
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("K".into()),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    // Force a DUPLICATE current row in ROOT (as ingest/history produce in the
    // live db), bypassing transact's idempotency.
    let vb = Value::Str("K".into()).to_bytes();
    store
        .conn
        .execute(
            "INSERT INTO transactions (timestamp) VALUES ('2026-01-02T00:00:00Z')",
            [],
        )
        .unwrap();
    let tx2 = store.conn.last_insert_rowid();
    store.conn.execute(
        "INSERT INTO facts (e,a,v,g,tx,valid_from,valid_to,op) VALUES (?1,?2,?3,0,?4,'2026-01-02T00:00:00Z',NULL,1)",
        rusqlite::params![e, a, vb, tx2]).unwrap();
    let dup_count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e=?1 AND a=?2 AND g=0 AND op=1 AND valid_to IS NULL",
            [e, a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        dup_count, 2,
        "precondition: two current rows for the same triple"
    );

    let ov = store.overlay_create("http://ex/g/t", 0).unwrap();
    let view = store.compose_view(ov).unwrap();
    let matches = view
        .iter()
        .filter(|f| f.entity == e && f.attribute == a)
        .count();
    assert_eq!(
        matches, 1,
        "compose must dedupe the re-asserted base fact to one triple"
    );
}

/// Retraction must survive an entity whose logical triple is backed by MANY
/// fact rows, and must close it exactly once (aegis-a0ne).
///
/// The store is append-only, so a re-assert from another transaction adds a row
/// rather than replacing one. Mapping those rows 1:1 onto retraction datums put
/// several identical `(e, a, v)` into one tx and blew the `(e, a, v, tx)`
/// PRIMARY KEY, so retraction failed with a UNIQUE constraint error on exactly
/// the oldest, most re-asserted entities — the ones most worth retracting. Live
/// `luvu` carried 29 rows of one `rdf:type` value.
///
/// The duplicates are inserted DIRECTLY here on purpose: today's `transact`
/// skips an assert whose (e, a, v) is already active, so the modern write path
/// can no longer produce this state and a test built on it would pass against
/// the bug. That is precisely why the original hypothesis looked disproven when
/// checked with a freshly-ingested node.
#[test]
fn retract_survives_duplicate_backing_rows() {
    let mut store = test_store();

    let e = store.intern("http://example.org/luvu").unwrap();
    let a = store.intern("http://example.org/type").unwrap();
    let v = Value::Str("BareMetalHost".into());

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

    // Simulate the pre-dedup ingest path: the same live triple, more rows.
    for _ in 0..3 {
        store
            .conn
            .execute(
                "INSERT INTO transactions (timestamp, actor, source) VALUES ('2026-01-02', NULL, 'legacy')",
                [],
            )
            .unwrap();
        let tx = store.conn.last_insert_rowid();
        store
            .conn
            .execute(
                "INSERT INTO facts (e, a, v, g, tx, valid_from, valid_to, op) \
                 VALUES (?1, ?2, ?3, 0, ?4, '2026-01-02', NULL, 1)",
                rusqlite::params![e, a, v.to_bytes(), tx],
            )
            .unwrap();
    }
    assert_eq!(
        store.entity_facts(e).unwrap().len(),
        4,
        "precondition: one logical triple backed by four rows"
    );

    // Previously: Err(UNIQUE constraint failed: facts.e, facts.a, facts.v, facts.tx)
    let (_tx, count) = store
        .retract_triples(
            e,
            Some(a),
            Some(&v),
            "2026-01-03T00:00:00Z",
            Some("ian"),
            false,
        )
        .expect("retraction must not fail on duplicate backing rows");

    assert_eq!(
        count, 1,
        "one logical triple retracted once, not once per row"
    );
    assert!(
        store.entity_facts(e).unwrap().is_empty(),
        "every backing row must be closed, or the triple survives its own retraction"
    );
}

/// Retracting ONE of an entity's types must leave the others alive (aegis-a0ne).
///
/// This is the discriminating case, and its absence is why the bug shipped: on a
/// node with a SINGLE rdf:type, "removed the value you asked for" and "removed
/// every row for that predicate" are THE SAME OBSERVABLE. Both controls in the
/// original report had one type, both returned green, and the green was
/// published. A single-type test is not coverage — it is a test that cannot
/// fail. The failure mode it misses is silent: the node keeps its label,
/// comments and edges and loses its type, so it stays visible to label scans
/// and vanishes from every `?s a ?type` query. A half-ghost.
#[test]
fn retract_one_type_leaves_the_others() {
    let mut store = test_store();

    let e = store.intern("http://example.org/goldblum-repo").unwrap();
    let a = store.intern("http://example.org/type").unwrap();
    let keep = Value::Str("GitRepo".into());
    let drop = Value::Str("GitRepository".into());

    for v in [&keep, &drop] {
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
    }
    assert_eq!(
        store.entity_facts(e).unwrap().len(),
        2,
        "precondition: two types"
    );

    let (_tx, count) = store
        .retract_triples(
            e,
            Some(a),
            Some(&drop),
            "2026-01-02T00:00:00Z",
            Some("ian"),
            false,
        )
        .expect("targeted retraction must succeed");

    assert_eq!(
        count, 1,
        "exactly ONE triple retracted, not every row for the predicate"
    );

    let left: Vec<Value> = store
        .entity_facts(e)
        .unwrap()
        .into_iter()
        .map(|f| f.value)
        .collect();
    assert_eq!(
        left,
        vec![keep],
        "the untargeted type MUST survive — losing it ghosts the node: label and edges \
         remain while every type query goes blind"
    );
}

/// The frozen-source-of-truth bug: a bare-string object for an
/// IRI-valued predicate retracts NOTHING and used to report success, so no
/// agent could ever be re-parented and nobody was told. All four arms, because
/// a guard that only ever refuses is as useless as one that never does:
///   1. bare `Str` for a `Ref`-only predicate -> LOUD error (the footgun),
///   2. correctly shaped `Ref` -> retracts the one edge (the fix works),
///   3. assert the replacement -> re-parenting actually completes end to end,
///   4. a correctly shaped but genuinely-absent `Ref` -> idempotent `(0, 0)`,
///      NOT an error (the guard must not break legitimate re-retraction).
#[test]
fn retract_str_for_an_iri_edge_is_loud_not_silent() {
    let mut store = test_store();
    let agent = store.intern("http://example.org/kprobe-a").unwrap();
    let reports_to = store.intern("http://example.org/reports_to").unwrap();
    let boss = store.intern("http://example.org/kprobe-b").unwrap();
    let boss2 = store.intern("http://example.org/kprobe-c").unwrap();

    let edge = |entity, attribute, value| Datum {
        entity,
        attribute,
        value,
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(
            &[edge(agent, reports_to, Value::Ref(boss))],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    // ARM 1: the footgun — a bare string that json_to_value would build as
    // Value::Str. It can never equal the stored Value::Ref, so it matches
    // nothing; the API must now SAY so instead of returning retracted:0.
    let bare = Value::Str("http://example.org/kprobe-b".into());
    let err = store
        .retract_triples(
            agent,
            Some(reports_to),
            Some(&bare),
            "2026-01-02",
            None,
            false,
        )
        .expect_err("a bare string for an IRI edge must be refused, not silently no-op");
    let msg = err.to_string();
    assert!(
        msg.contains("string literal"),
        "error must name the shape mismatch: {msg}"
    );
    assert!(
        msg.contains("iri"),
        "error must teach the {{\"iri\": ...}} shape: {msg}"
    );
    // ...and it wrote nothing.
    assert_eq!(
        store.entity_facts(agent).unwrap().len(),
        1,
        "the edge must survive a refused retract"
    );

    // ARM 2: the correctly shaped object retracts exactly the one edge.
    let (_tx, count) = store
        .retract_triples(
            agent,
            Some(reports_to),
            Some(&Value::Ref(boss)),
            "2026-01-03",
            None,
            false,
        )
        .expect("a Ref-shaped object must retract the edge");
    assert_eq!(count, 1, "exactly the one reports_to edge");
    assert!(
        store.entity_facts(agent).unwrap().is_empty(),
        "the old supervisor edge is gone"
    );

    // ARM 3: re-parenting COMPLETES — assert the new supervisor.
    store
        .transact(
            &[edge(agent, reports_to, Value::Ref(boss2))],
            "2026-01-03T00:01:00Z",
            None,
            None,
        )
        .unwrap();
    let now: Vec<Value> = store
        .entity_facts(agent)
        .unwrap()
        .into_iter()
        .map(|f| f.value)
        .collect();
    assert_eq!(
        now,
        vec![Value::Ref(boss2)],
        "the agent now reports to the new supervisor, and only that"
    );

    // ARM 4: a correctly shaped but absent object is an idempotent no-op, NOT an
    // error. The guard fires only on the unambiguous Str-for-Ref mistake.
    let (_tx, count) = store
        .retract_triples(
            agent,
            Some(reports_to),
            Some(&Value::Ref(boss)),
            "2026-01-04",
            None,
            false,
        )
        .expect("re-retracting an absent, correctly-shaped edge must stay idempotent");
    assert_eq!(
        count, 0,
        "nothing to retract -> a quiet no-op, not a refusal"
    );
}

/// The residual `{"retracted":0}` ambiguity: a bare IRI-shaped string on a
/// predicate that has NO current fact to compare against still used to no-op
/// silently — the case that nearly got a freshly-deployed fix reported as broken
/// (an operator retracting `rdf:type` with a bare string). The two outcomes MUST
/// diverge, and this test would FAIL against pre-fix behaviour, where BOTH arms
/// returned `Ok((_, 0))` (per the standing negative-test rule).
#[test]
fn retract_bare_iri_string_errors_even_with_no_matching_fact() {
    let mut store = test_store();
    let node = store.intern("http://example.org/some-node").unwrap();
    let rdf_type = store.intern(crate::namespace::RDF_TYPE).unwrap();

    // The entity has NO rdf:type fact at all (already retracted, or never set).
    assert!(
        store.entity_facts(node).unwrap().is_empty(),
        "precondition: nothing to compare the retract value against"
    );

    // ARM A: a bare string that PARSES as an IRI -> ERROR, not a silent 0. There
    // is no stored Ref to infer from, so the string's own `scheme://` is the
    // signal it is a mis-shaped edge retract.
    let bare = Value::Str("http://example.org/Person".into());
    let err = store
        .retract_triples(node, Some(rdf_type), Some(&bare), "2026-01-02", None, false)
        .expect_err("a bare IRI-shaped string must be refused even with no matching fact");
    assert!(
        err.to_string().contains("iri"),
        "error must teach the {{\"iri\": ...}} form: {err}"
    );

    // ARM B: THE DIVERGENCE. A correctly shaped `{iri}` object for a triple that
    // genuinely does not exist stays an idempotent `retracted: 0` — a fix that
    // turned this into an error too would just move the ambiguity.
    let iri = Value::Ref(store.intern("http://example.org/Person").unwrap());
    let (_tx, count) = store
        .retract_triples(node, Some(rdf_type), Some(&iri), "2026-01-03", None, false)
        .expect("a correctly shaped, genuinely-absent object must stay a quiet no-op");
    assert_eq!(count, 0, "absent + correctly shaped -> 0, NOT an error");

    // ARM C: a plain string literal (no scheme) on an entity with no such fact is
    // a real idempotent no-op, not a mistake — the guard must not cry wolf.
    let plain = Value::Str("just a label".into());
    let (_tx, count) = store
        .retract_triples(
            node,
            Some(rdf_type),
            Some(&plain),
            "2026-01-04",
            None,
            false,
        )
        .expect("a plain literal with no matching fact is a legitimate no-op");
    assert_eq!(count, 0, "no scheme -> treated as a literal -> quiet no-op");
}

/// Retracting an entity's LAST rdf:type while it keeps other facts is refused
/// unless explicitly overridden (aegis-a0ne).
///
/// Asserts all four arms, because a guard that only ever refuses is as useless
/// as one that never does: refuse the half-ghost, allow the override, allow a
/// retraction that leaves another type behind, and allow retracting an entity
/// whole (nothing survives, so nothing is orphaned).
#[test]
fn retract_refuses_to_orphan_the_last_type() {
    // The REAL rdf:type IRI — the guard keys on it, so a placeholder predicate
    // would make this test pass without ever exercising the guard.
    let ty = crate::namespace::RDF_TYPE;
    let lbl = "http://example.org/label";

    // ARM 1: last type + surviving facts -> REFUSED, and nothing is written.
    let mut store = test_store();
    let e = store.intern("http://example.org/goldblum-repo").unwrap();
    let a = store.intern(ty).unwrap();
    let l = store.intern(lbl).unwrap();
    let t = Value::Str("GitRepository".into());
    let mk = |entity, attribute, value: Value| Datum {
        entity,
        attribute,
        value,
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(
            &[
                mk(e, a, t.clone()),
                mk(e, l, Value::Str("goldblum-repo".into())),
            ],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    assert!(
        store
            .retract_triples(e, Some(a), Some(&t), "2026-01-02", None, false)
            .is_err(),
        "stripping the last type off a surviving node must be refused"
    );
    assert_eq!(
        store.entity_facts(e).unwrap().len(),
        2,
        "a refused retraction must write NOTHING"
    );

    // ARM 2: same call, explicit override -> allowed.
    assert!(
        store
            .retract_triples(e, Some(a), Some(&t), "2026-01-02", None, true)
            .is_ok(),
        "allow_orphan must let a caller do it deliberately"
    );

    // ARM 3: another type survives -> allowed without override.
    let mut s2 = test_store();
    let e2 = s2.intern("http://example.org/two-types").unwrap();
    let a2 = s2.intern(ty).unwrap();
    let keep = Value::Str("GitRepo".into());
    let drop = Value::Str("GitRepository".into());
    s2.transact(
        &[mk(e2, a2, keep.clone()), mk(e2, a2, drop.clone())],
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    assert!(
        s2.retract_triples(e2, Some(a2), Some(&drop), "2026-01-02", None, false)
            .is_ok(),
        "dropping one of two types leaves the node typed — must be allowed"
    );

    // ARM 4: whole-entity retraction -> nothing survives, so no ghost, allowed.
    let mut s3 = test_store();
    let e3 = s3.intern("http://example.org/whole").unwrap();
    let a3 = s3.intern(ty).unwrap();
    s3.transact(
        &[mk(e3, a3, Value::Str("Thing".into()))],
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    assert!(
        s3.retract_triples(e3, None, None, "2026-01-02", None, false)
            .is_ok(),
        "retracting an entity whole removes identity AND references — no ghost"
    );
}

// ---------------------------------------------------------------------------
// Committed reads are ROOT-scoped (quipu #56)
//
// Every one of these fails without the `g` predicate on the shared read path.
// They were invisible before because every other fixture is ROOT-only, so the
// cross-graph reads were correct by accident.
// ---------------------------------------------------------------------------

/// ROOT holds one fact; a tenant overlay asserts another on the same entity.
/// Returns `(store, entity, root_attr, overlay_attr)`.
fn store_with_tenant_overlay() -> (Store, i64, i64, i64) {
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://ex/svc").unwrap();
    let root_attr = store.intern("http://ex/root-attr").unwrap();
    let ov_attr = store.intern("http://ex/overlay-attr").unwrap();

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: root_attr,
                value: Value::Str("ROOT".into()),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let ov = store.overlay_create("http://ex/graph/tenant-1", 0).unwrap();
    store
        .overlay_write(
            ov,
            Op::Assert,
            e,
            ov_attr,
            Value::Str("TENANT".into()),
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

    (store, e, root_attr, ov_attr)
}

#[test]
fn current_facts_is_root_scoped() {
    // The headline of #56: the reasoner, PageRank, export, OWL parse and
    // reconcile all read through this. Un-scoped it returned 2.
    let (store, _e, root_attr, _ov) = store_with_tenant_overlay();
    let facts = store.current_facts().unwrap();
    assert_eq!(
        facts.len(),
        1,
        "current_facts must not see the tenant overlay: {facts:?}"
    );
    assert_eq!(facts[0].attribute, root_attr);
}

#[test]
fn entity_facts_is_root_scoped() {
    let (store, e, root_attr, _ov) = store_with_tenant_overlay();
    let facts = store.entity_facts(e).unwrap();
    assert_eq!(facts.len(), 1, "entity_facts must not see the overlay");
    assert_eq!(facts[0].attribute, root_attr);
}

#[test]
fn retraction_in_root_does_not_touch_an_overlay() {
    // #36's stated invariant, which did not hold: retract_triples selected via
    // the cross-graph entity_facts and then committed the datums to ROOT, so
    // retracting the entity whole produced retractions for the OVERLAY's fact
    // and wrote them into ROOT.
    let (mut store, e, _root_attr, ov_attr) = store_with_tenant_overlay();

    let (_tx, count) = store
        .retract_triples(e, None, None, "2026-01-03T00:00:00Z", None, true)
        .unwrap();
    assert_eq!(count, 1, "only ROOT's single fact is retractable from ROOT");

    // The overlay's own fact is untouched in its own graph.
    let ov_g = store.lookup("http://ex/graph/tenant-1").unwrap().unwrap();
    let ov_facts = store.current_facts_in_graph(ov_g).unwrap();
    assert!(
        ov_facts.iter().any(|f| f.attribute == ov_attr),
        "the overlay's fact must survive a ROOT retraction: {ov_facts:?}"
    );
}

#[test]
fn half_ghost_guard_does_not_count_overlay_facts_as_survivors() {
    // The last-rdf:type guard refuses to strip a type while OTHER facts on the
    // entity survive. Un-scoped, a tenant overlay's fact counted as a survivor,
    // so an overlay could block a legitimate ROOT retraction.
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://ex/svc").unwrap();
    let type_id = store.intern(crate::namespace::RDF_TYPE).unwrap();
    let ov_attr = store.intern("http://ex/overlay-attr").unwrap();

    // ROOT holds ONLY the rdf:type, so retracting it leaves no ghost.
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: type_id,
                value: Value::Str("http://ex/Service".into()),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let ov = store.overlay_create("http://ex/graph/tenant-1", 0).unwrap();
    store
        .overlay_write(
            ov,
            Op::Assert,
            e,
            ov_attr,
            Value::Str("TENANT".into()),
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

    let (_tx, count) = store
        .retract_triples(e, Some(type_id), None, "2026-01-03T00:00:00Z", None, false)
        .expect("no ROOT fact survives, so this is not a half-ghost");
    assert_eq!(count, 1);
}

#[test]
fn entity_history_is_root_scoped() {
    let (store, e, _root_attr, _ov) = store_with_tenant_overlay();
    let hist = store.entity_history(e).unwrap();
    assert_eq!(
        hist.len(),
        1,
        "history is per-graph; the overlay write is not ROOT history: {hist:?}"
    );
}

#[test]
fn facts_as_of_is_root_scoped() {
    let (store, _e, _root_attr, _ov) = store_with_tenant_overlay();
    let facts = store
        .facts_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-06-01T00:00:00Z".into()),
        })
        .unwrap();
    assert_eq!(
        facts.len(),
        1,
        "time travel scopes within a graph (named-graphs.md §1): {facts:?}"
    );
}

#[test]
fn contradiction_detection_is_root_scoped() {
    // An overlay asserting a different value for the same (e, a) is an
    // override, not a contradiction in its committed parent.
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://ex/svc").unwrap();
    let a = store.intern("http://ex/status").unwrap();
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("up".into()),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    let ov = store.overlay_create("http://ex/graph/tenant-1", 0).unwrap();
    store
        .overlay_write(
            ov,
            Op::Assert,
            e,
            a,
            Value::Str("down".into()),
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

    let pairs = store.detect_contradictions(e, a).unwrap();
    assert!(
        pairs.is_empty(),
        "an overlay override is not a ROOT contradiction: {pairs:?}"
    );
}

#[test]
fn compose_view_still_sees_the_overlay() {
    // The regression guard for the fix: overlays.rs carries its own
    // graph-aware SQL, so ROOT-scoping the shared read path must not blind it.
    let (store, _e, root_attr, ov_attr) = store_with_tenant_overlay();
    let ov_g = store.lookup("http://ex/graph/tenant-1").unwrap().unwrap();
    let view = store.compose_view(ov_g).unwrap();
    let attrs: std::collections::BTreeSet<i64> = view.iter().map(|f| f.attribute).collect();
    assert!(attrs.contains(&ov_attr), "overlay assert present in view");
    assert!(
        attrs.contains(&root_attr),
        "root fact falls through to view"
    );
}

// ---------------------------------------------------------------------------
// quipu #71 — bitemporal shape + ontology registry
// ---------------------------------------------------------------------------

#[test]
fn loading_a_second_version_closes_the_first_rather_than_overwriting() {
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();
    store
        .load_shapes("s", "# v2", "2026-08-02T00:00:00Z")
        .unwrap();

    // Current reads see only v2 — unchanged behaviour.
    let now = store.list_shapes().unwrap();
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].1, "# v2");

    // But v1 is still there, closed.
    let total: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM shapes WHERE name = 's'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(total, 2, "the prior version is CLOSED, not discarded");
}

#[test]
fn validate_as_of_v1s_window_uses_v1_semantics() {
    // #71 acceptance 1 — the headline.
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();
    store
        .load_shapes("s", "# v2", "2026-08-02T00:00:00Z")
        .unwrap();

    let at_v1 = store
        .get_combined_shapes_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-08-01T12:00:00Z".into()),
        })
        .unwrap()
        .unwrap();
    assert_eq!(at_v1, "# v1", "as-of v1's window must use v1");

    let at_v2 = store
        .get_combined_shapes_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-08-02T12:00:00Z".into()),
        })
        .unwrap()
        .unwrap();
    assert_eq!(at_v2, "# v2");

    // And the default (no as_of) is still current.
    assert_eq!(store.get_combined_shapes().unwrap().unwrap(), "# v2");
}

#[test]
fn a_window_before_any_version_has_no_shapes() {
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();
    assert!(
        store
            .get_combined_shapes_as_of(&AsOf {
                tx: None,
                valid_at: Some("2026-07-01T00:00:00Z".into()),
            })
            .unwrap()
            .is_none(),
        "before it was loaded, it did not govern anything"
    );
}

#[test]
fn a_shape_load_appears_in_events_with_a_tx() {
    // #71 acceptance 2. Before this the audit spine had NO record the rules moved.
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();

    let (etype, tx): (String, i64) = store
        .conn
        .query_row(
            "SELECT type, tx_id FROM events WHERE subject = 's' ORDER BY \"offset\" DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(etype, "shapes.loaded");
    assert!(tx >= 0, "carries the tx watermark");
}

#[test]
fn remove_closes_the_row_and_history_remains_queryable() {
    // #71 acceptance 4.
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();
    assert!(store.remove_shapes("s").unwrap());

    assert!(store.list_shapes().unwrap().is_empty(), "gone from current");
    let rows: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM shapes WHERE name = 's'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1, "the row is CLOSED, never deleted");

    // And it is still readable at a time when it was in force.
    let then = store
        .get_combined_shapes_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-08-01T12:00:00Z".into()),
        })
        .unwrap();
    assert_eq!(then.as_deref(), Some("# v1"), "history stays queryable");
}

#[test]
fn removing_something_absent_is_false_and_emits_nothing() {
    let store = Store::open_in_memory().unwrap();
    assert!(!store.remove_shapes("nope").unwrap());
    let events: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 0, "a no-op must not pollute the audit spine");
}

#[test]
fn a_reload_at_the_same_instant_replaces_that_instants_version() {
    // No meaningful ordering within one timestamp, and the (name, valid_from)
    // primary key would otherwise collide.
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# a", "2026-08-01T00:00:00Z")
        .unwrap();
    store
        .load_shapes("s", "# b", "2026-08-01T00:00:00Z")
        .unwrap();
    let rows: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM shapes WHERE name = 's'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(store.list_shapes().unwrap()[0].1, "# b");
}

#[test]
fn ontologies_get_the_same_discipline() {
    let store = Store::open_in_memory().unwrap();
    store
        .load_ontology("o", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();
    store
        .load_ontology("o", "# v2", "2026-08-02T00:00:00Z")
        .unwrap();
    assert_eq!(
        store.list_ontologies().unwrap().len(),
        1,
        "current is one row"
    );
    let at_v1 = store
        .list_ontologies_as_of(&AsOf {
            tx: None,
            valid_at: Some("2026-08-01T12:00:00Z".into()),
        })
        .unwrap();
    assert_eq!(at_v1[0].1, "# v1");
    assert!(store.remove_ontology("o").unwrap());
    assert!(store.list_ontologies().unwrap().is_empty());
}

#[test]
fn a_pre_migration_registry_migrates_with_open_rows_and_unchanged_reads() {
    // #71 acceptance 3, driven from the ACTUAL old schema rather than from the
    // migrated one — the only way to know the migration works is to run it on a
    // table shaped the way the old code left it.
    let store = Store::open_in_memory().unwrap();
    store
        .conn
        .execute_batch(
            "DROP TABLE shapes;
             CREATE TABLE shapes (
                 name      TEXT PRIMARY KEY,
                 turtle    TEXT NOT NULL,
                 loaded_at TEXT NOT NULL
             );
             INSERT INTO shapes (name, turtle, loaded_at)
                 VALUES ('legacy', '# old', '2026-01-01T00:00:00Z');",
        )
        .unwrap();

    Store::migrate_bitemporal_registries(&store.conn).unwrap();

    // valid_from backfilled from loaded_at, valid_to open.
    let (vf, vt): (String, Option<String>) = store
        .conn
        .query_row(
            "SELECT valid_from, valid_to FROM shapes WHERE name = 'legacy'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(vf, "2026-01-01T00:00:00Z", "valid_from = loaded_at");
    assert_eq!(vt, None, "open-ended");

    // Default reads unchanged.
    let listed = store.list_shapes().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, "legacy");
    assert_eq!(listed[0].1, "# old");
    assert_eq!(store.get_combined_shapes().unwrap().unwrap(), "# old");

    // And the migration is idempotent.
    Store::migrate_bitemporal_registries(&store.conn).unwrap();
    assert_eq!(store.list_shapes().unwrap().len(), 1);
}

#[test]
fn the_migration_preserves_every_legacy_row() {
    // A rebuild that silently dropped rows would be the worst outcome here, and
    // a single-row fixture would not catch it.
    let store = Store::open_in_memory().unwrap();
    store
        .conn
        .execute_batch(
            "DROP TABLE ontologies;
             CREATE TABLE ontologies (
                 name      TEXT PRIMARY KEY,
                 turtle    TEXT NOT NULL,
                 loaded_at TEXT NOT NULL
             );
             INSERT INTO ontologies (name, turtle, loaded_at) VALUES
                 ('a', '# a', '2026-01-01T00:00:00Z'),
                 ('b', '# b', '2026-01-02T00:00:00Z'),
                 ('c', '# c', '2026-01-03T00:00:00Z');",
        )
        .unwrap();

    Store::migrate_bitemporal_registries(&store.conn).unwrap();
    let names: Vec<String> = store
        .list_ontologies()
        .unwrap()
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    assert_eq!(names, vec!["a", "b", "c"], "all three survive the rebuild");
}

#[test]
fn validate_falls_back_to_stored_shapes_and_honours_as_of() {
    // quipu #71: the `/validate` surface. An explicit `shapes` still wins (the
    // existing contract); absent one, the STORED shapes are used, optionally
    // as they stood in a prior window.
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("s", "# v1", "2026-08-01T00:00:00Z")
        .unwrap();
    store
        .load_shapes("s", "# v2", "2026-08-02T00:00:00Z")
        .unwrap();

    // Explicit shapes -> None, i.e. "caller supplied them, don't touch it".
    let explicit = crate::resolve_validation_shapes(
        &store,
        &serde_json::json!({"shapes": "# mine", "data": ""}),
    )
    .unwrap();
    assert!(
        explicit.is_none(),
        "an explicit `shapes` must not be overridden"
    );

    // No shapes, no window -> current.
    let now = crate::resolve_validation_shapes(&store, &serde_json::json!({"data": ""}))
        .unwrap()
        .unwrap();
    assert_eq!(now, "# v2", "defaults to now");

    // No shapes, a prior window -> that window's version.
    let then = crate::resolve_validation_shapes(
        &store,
        &serde_json::json!({"data": "", "valid_at": "2026-08-01T12:00:00Z"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(then, "# v1", "as-of v1's window uses v1 semantics");
}

// ---------------------------------------------------------------------------
// quipu #83 — as_of_tx can see facts retracted since
// ---------------------------------------------------------------------------

/// Assert `(e,a,v)`, returning the tx that did it.
fn assert_one(store: &mut Store, e: i64, a: i64, v: &str, ts: &str) -> i64 {
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str(v.into()),
                valid_from: ts.into(),
                valid_to: None,
                op: Op::Assert,
            }],
            ts,
            None,
            None,
        )
        .unwrap()
}

#[test]
fn as_of_tx_sees_a_fact_that_was_live_then_and_is_retracted_now() {
    // THE defect. `as_of_tx = N` means "what did the store know at N?" — a fact
    // asserted before N and retracted after it MUST be visible at N.
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();
    let tx1 = assert_one(&mut store, e, a, "v1", "2026-01-01T00:00:00Z");

    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("v1".into()),
                valid_from: "2026-06-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Retract,
            }],
            "2026-06-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    // Now: gone.
    let now = crate::sparql::query(&store, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(now.rows().len(), 0, "retracted, so absent now");

    // As of tx1: present.
    let ctx = crate::sparql::TemporalContext {
        as_of_tx: Some(tx1),
        ..Default::default()
    };
    let then = crate::sparql::query_temporal(&store, "SELECT ?o WHERE { ?s ?p ?o }", &ctx).unwrap();
    assert_eq!(
        then.rows().len(),
        1,
        "as of tx1 the fact was live — this is what quipu #83 fixed"
    );
}

#[test]
fn as_of_tx_still_excludes_a_fact_retracted_before_that_tx() {
    // The CONTROL that makes the test above mean something. If `as_of_tx`
    // simply ignored retraction, both tests would pass for the wrong reason.
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();
    assert_one(&mut store, e, a, "v1", "2026-01-01T00:00:00Z");
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("v1".into()),
                valid_from: "2026-02-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Retract,
            }],
            "2026-02-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    // A tx AFTER the retraction.
    let later = assert_one(&mut store, e, a, "v2", "2026-03-01T00:00:00Z");

    let ctx = crate::sparql::TemporalContext {
        as_of_tx: Some(later),
        ..Default::default()
    };
    let rows = crate::sparql::query_temporal(&store, "SELECT ?o WHERE { ?s ?p ?o }", &ctx)
        .unwrap()
        .rows()
        .len();
    assert_eq!(rows, 1, "only v2 — v1 was already retracted by then");
}

#[test]
fn the_retracting_tx_is_recorded_on_the_row() {
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();
    assert_one(&mut store, e, a, "v1", "2026-01-01T00:00:00Z");
    let rtx = store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("v1".into()),
                valid_from: "2026-06-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Retract,
            }],
            "2026-06-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();

    let recorded: Option<i64> = store
        .conn
        .query_row(
            "SELECT retracted_tx FROM facts WHERE e = ?1 AND valid_to IS NOT NULL",
            params![e],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        recorded,
        Some(rtx),
        "the closing tx is recorded, not inferred"
    );
}

#[test]
fn a_legacy_row_closed_before_the_migration_stays_invisible() {
    // Deliberate, and the reason there is no backfill: the tx that closed a
    // legacy row was NEVER RECORDED. Leaving `retracted_tx` NULL makes
    // `retracted_tx > N` NULL, so the row stays invisible exactly as it is
    // today — no behaviour change for existing stores. Inventing a plausible
    // value would make it visible at windows it may not have been live in,
    // which is a worse answer than the honest gap.
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();
    let tx1 = assert_one(&mut store, e, a, "v1", "2026-01-01T00:00:00Z");
    store
        .transact(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("v1".into()),
                valid_from: "2026-06-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Retract,
            }],
            "2026-06-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    // Simulate a pre-#83 row: closed, with no retracting tx recorded.
    store
        .conn
        .execute("UPDATE facts SET retracted_tx = NULL", [])
        .unwrap();

    let ctx = crate::sparql::TemporalContext {
        as_of_tx: Some(tx1),
        ..Default::default()
    };
    let rows = crate::sparql::query_temporal(&store, "SELECT ?o WHERE { ?s ?p ?o }", &ctx)
        .unwrap()
        .rows()
        .len();
    assert_eq!(rows, 0, "legacy rows behave exactly as they did before #83");
}

#[test]
fn the_retraction_tx_migration_is_idempotent() {
    let store = Store::open_in_memory().unwrap();
    Store::migrate_retraction_tx(&store.conn).unwrap();
    Store::migrate_retraction_tx(&store.conn).unwrap();
    let cols: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('facts') WHERE name = 'retracted_tx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cols, 1);
}

// --- Term cache (quipu-yzf) ------------------------------------------------

/// The safety argument for [`TermCache`] is that `terms` is append-only while a
/// store is open, so a memoized mapping can never go stale. That is a property
/// of the SQL in this file, not a law — this test is the tripwire. If a write
/// path ever starts mutating `terms` in place, the cache becomes incorrect and
/// must gain invalidation; this failing is the signal to go do that.
#[test]
fn terms_table_is_append_only() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/store/mod.rs"))
        .expect("store source readable");
    for forbidden in ["UPDATE terms", "DELETE FROM terms", "REPLACE INTO terms"] {
        assert!(
            !src.contains(forbidden),
            "`{forbidden}` would make the memoized TermCache stale — see TermCache docs"
        );
    }
}

#[test]
fn resolve_memoizes_and_agrees_with_sql() {
    let store = test_store();
    let id = store.intern("http://example.org/Memo").unwrap();
    assert_eq!(store.term_cache.borrow().len(), 0, "cold before first read");

    let first = store.resolve(id).unwrap();
    assert_eq!(store.term_cache.borrow().len(), 1, "populated by resolve");

    // Second read comes from the cache and must be identical.
    assert_eq!(store.resolve(id).unwrap(), first);

    // And identical to what the uncached SQL would have returned.
    let direct: String = store
        .conn
        .query_row("SELECT iri FROM terms WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(first, direct);
}

#[test]
fn lookup_memoizes_hits_and_stays_consistent() {
    let store = test_store();
    let id = store.intern("http://example.org/Hit").unwrap();
    assert_eq!(store.lookup("http://example.org/Hit").unwrap(), Some(id));
    // Served from cache the second time, same answer.
    assert_eq!(store.lookup("http://example.org/Hit").unwrap(), Some(id));
    assert_eq!(store.resolve(id).unwrap(), "http://example.org/Hit");
}

/// A miss must NOT be cached. Interning after a failed lookup has to become
/// visible immediately — caching the `None` would hide the write from a reader
/// that happened to ask first.
#[test]
fn a_lookup_miss_is_not_cached() {
    let store = test_store();
    assert_eq!(store.lookup("http://example.org/Later").unwrap(), None);
    let id = store.intern("http://example.org/Later").unwrap();
    assert_eq!(
        store.lookup("http://example.org/Later").unwrap(),
        Some(id),
        "the intern must be visible to a reader that already missed"
    );
}

#[test]
fn warm_term_cache_matches_per_term_resolution() {
    let store = test_store();
    let iris = [
        "http://example.org/A",
        "http://example.org/B",
        "http://example.org/C",
    ];
    let ids: Vec<i64> = iris.iter().map(|i| store.intern(i).unwrap()).collect();

    // A fresh store already interns bootstrap terms, so the invariant is
    // "every row in `terms`", not "the three we just added".
    let rows: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM terms", [], |r| r.get(0))
        .unwrap();
    let warmed = store.warm_term_cache().unwrap();
    assert_eq!(warmed as i64, rows, "one cache entry per row in `terms`");

    // Every warmed entry equals what a cold per-term resolve would return.
    for (id, iri) in ids.iter().zip(iris.iter()) {
        assert_eq!(store.resolve(*id).unwrap(), *iri);
        assert_eq!(store.lookup(iri).unwrap(), Some(*id));
    }
}

/// Warming twice is idempotent — it must not double-count or diverge.
#[test]
fn warm_term_cache_is_idempotent() {
    let store = test_store();
    store.intern("http://example.org/Once").unwrap();
    let first = store.warm_term_cache().unwrap();
    let second = store.warm_term_cache().unwrap();
    assert_eq!(first, second);
}

/// The cap must bound growth without ever changing an answer — a miss reads
/// SQL, so a full cache is slower and never wrong.
#[test]
fn a_capped_term_cache_still_resolves_correctly() {
    let store = test_store();
    store.set_term_cache_limit(2);
    let iris = [
        "http://example.org/one",
        "http://example.org/two",
        "http://example.org/three",
        "http://example.org/four",
    ];
    let ids: Vec<i64> = iris.iter().map(|i| store.intern(i).unwrap()).collect();

    for (id, iri) in ids.iter().zip(iris.iter()) {
        assert_eq!(store.resolve(*id).unwrap(), *iri, "resolve past the cap");
        assert_eq!(store.lookup(iri).unwrap(), Some(*id), "lookup past the cap");
    }
    assert!(
        store.term_cache_len() <= 2,
        "cache grew past its cap: {}",
        store.term_cache_len()
    );
}

/// A zero cap disables memoization entirely and drops what is held. Every
/// answer must survive that, since the fallback is the original SQL path.
#[test]
fn a_zero_term_cache_limit_disables_caching() {
    let store = test_store();
    let id = store.intern("http://example.org/x").unwrap();
    assert_eq!(store.resolve(id).unwrap(), "http://example.org/x");
    assert!(store.term_cache_len() > 0);

    store.set_term_cache_limit(0);
    assert_eq!(store.term_cache_len(), 0, "existing entries dropped");
    assert_eq!(
        store.resolve(id).unwrap(),
        "http://example.org/x",
        "resolution still works with caching off"
    );
    assert_eq!(store.term_cache_len(), 0, "and nothing is re-admitted");
}

/// `warm_term_cache` must respect the cap too, or the bulk path would be a
/// hole straight through it.
#[test]
fn warm_term_cache_respects_the_cap() {
    let store = test_store();
    for i in 0..20 {
        store.intern(&format!("http://example.org/{i}")).unwrap();
    }
    store.set_term_cache_limit(5);
    let warmed = store.warm_term_cache().unwrap();
    assert!(
        warmed <= 5,
        "warm admitted {warmed} entries past a cap of 5"
    );
}
