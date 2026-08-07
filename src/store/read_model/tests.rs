//! Tests for the in-memory read model (quipu-d6x).
//!
//! The load-bearing ones are the DIFFERENTIAL tests: after every write shape,
//! a model kept up to date by `apply` must hold exactly what a model rebuilt
//! from SQL holds. Anything less and the two drift, and a drifting index is
//! worse than no index — it answers confidently with facts the store retracted.

use super::*;
use crate::store::Datum;
use crate::types::{Op, Value};

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn datum(store: &Store, s: &str, p: &str, o: &str, op: Op) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: Value::Ref(store.intern(o).unwrap()),
        valid_from: "2026-01-01T00:00:00Z".to_string(),
        valid_to: None,
        op,
    }
}

/// Write `datums` through the store, then assert that a model rebuilt from SQL
/// and a model updated incrementally agree exactly.
fn assert_incremental_matches_rebuild(store: &mut Store, model: &mut ReadModel, datums: &[Datum]) {
    store
        .transact(datums, "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();
    model.apply_all(datums);
    let rebuilt = ReadModel::build(store, crate::schema::ROOT_GRAPH).unwrap();
    assert_eq!(
        model.triples_sorted(),
        rebuilt.triples_sorted(),
        "incremental model diverged from a rebuild"
    );
    assert_eq!(model.len(), rebuilt.len(), "triple counts diverged");
}

#[test]
fn builds_from_an_empty_store() {
    let s = store();
    let model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    assert!(model.is_empty());
    assert_eq!(model.graph(), crate::schema::ROOT_GRAPH);
}

#[test]
fn build_indexes_every_permutation() {
    let mut s = store();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let (e, a, v) = (d.entity, d.attribute, d.value.clone());
    s.transact(&[d], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();

    let model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    assert_eq!(model.by_subject(e), [(a, v.clone())]);
    assert_eq!(model.by_predicate(a), [(e, v.clone())]);
    assert_eq!(model.by_predicate_object(a, &v), [e]);
    assert!(model.contains(e, a, &v));
}

#[test]
fn a_missing_key_answers_empty_rather_than_panicking() {
    let s = store();
    let model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    assert!(model.by_subject(999).is_empty());
    assert!(model.by_predicate(999).is_empty());
    assert!(model.by_predicate_object(999, &Value::Int(1)).is_empty());
    assert!(!model.contains(1, 2, &Value::Int(3)));
}

#[test]
fn incremental_assert_matches_a_rebuild() {
    let mut s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    assert_incremental_matches_rebuild(&mut s, &mut model, &[d]);
    assert_eq!(model.len(), 1);
}

/// Retraction is the half an append-only index gets wrong.
#[test]
fn incremental_retract_matches_a_rebuild() {
    let mut s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();

    let assertion = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    assert_incremental_matches_rebuild(&mut s, &mut model, std::slice::from_ref(&assertion));
    assert_eq!(model.len(), 1);

    let retraction = Datum {
        op: Op::Retract,
        ..assertion
    };
    assert_incremental_matches_rebuild(&mut s, &mut model, &[retraction]);
    assert!(
        model.is_empty(),
        "a retracted triple must leave the indexes"
    );
}

#[test]
fn a_retracted_triple_leaves_all_three_indexes() {
    let s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let (e, a, v) = (d.entity, d.attribute, d.value.clone());
    model.apply(&d);
    model.apply(&Datum {
        op: Op::Retract,
        ..d
    });

    assert!(model.by_subject(e).is_empty(), "spo still holds it");
    assert!(model.by_predicate(a).is_empty(), "pso still holds it");
    assert!(
        model.by_predicate_object(a, &v).is_empty(),
        "pos still holds it"
    );
    assert!(!model.contains(e, a, &v));
}

/// Asserting the same triple twice must not double-count — SQL's `SELECT
/// DISTINCT` never produced duplicate rows, so neither may the index.
#[test]
fn a_repeated_assert_is_idempotent() {
    let s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    model.apply(&d);
    model.apply(&d);
    assert_eq!(model.len(), 1);
    assert_eq!(model.by_subject(d.entity).len(), 1);
    assert_eq!(model.by_predicate(d.attribute).len(), 1);
    assert_eq!(model.by_predicate_object(d.attribute, &d.value).len(), 1);
}

/// Retracting something absent is a no-op, not an underflow. `triples` is a
/// `usize`, so a stray decrement would panic in debug and wrap in release.
#[test]
fn retracting_an_absent_triple_is_a_no_op() {
    let s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Retract,
    );
    model.apply(&d);
    assert!(model.is_empty());
}

/// A tombstone hides a triple from the composed view, so it must remove from
/// the index exactly as a retraction does.
#[test]
fn a_tombstone_removes_like_a_retraction() {
    let s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    model.apply(&d);
    model.apply(&Datum {
        op: Op::Tombstone,
        ..d
    });
    assert!(model.is_empty());
}

/// A datum that arrives already closed is not current, and `build` would not
/// have loaded it (`valid_to IS NULL`). Inserting it would make the incremental
/// path disagree with the build.
#[test]
fn an_already_closed_assert_is_not_indexed() {
    let s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let d = Datum {
        valid_to: Some("2026-06-01T00:00:00Z".to_string()),
        ..datum(
            &s,
            "http://example.org/a",
            "http://example.org/p",
            "http://example.org/b",
            Op::Assert,
        )
    };
    model.apply(&d);
    assert!(model.is_empty());
}

/// The differential property over a realistic mixed sequence, checked after
/// every step rather than only at the end — a model that diverges and then
/// converges is still a model that answered wrongly in between.
#[test]
fn a_mixed_write_sequence_stays_equal_to_a_rebuild() {
    let mut s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();

    let a_p_b = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let a_p_c = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/c",
        Op::Assert,
    );
    let d_q_b = datum(
        &s,
        "http://example.org/d",
        "http://example.org/q",
        "http://example.org/b",
        Op::Assert,
    );

    // Two triples sharing a subject and predicate.
    assert_incremental_matches_rebuild(&mut s, &mut model, &[a_p_b.clone(), a_p_c.clone()]);
    // A third on a different subject and predicate.
    assert_incremental_matches_rebuild(&mut s, &mut model, std::slice::from_ref(&d_q_b));
    // Retract one of the shared-bucket pair — the bucket must survive with the
    // other entry still in it.
    assert_incremental_matches_rebuild(
        &mut s,
        &mut model,
        &[Datum {
            op: Op::Retract,
            ..a_p_b
        }],
    );
    assert!(
        model.contains(a_p_c.entity, a_p_c.attribute, &a_p_c.value),
        "retracting a sibling must not take the whole bucket"
    );
    // Retract the rest.
    assert_incremental_matches_rebuild(
        &mut s,
        &mut model,
        &[
            Datum {
                op: Op::Retract,
                ..a_p_c
            },
            Datum {
                op: Op::Retract,
                ..d_q_b
            },
        ],
    );
    assert!(model.is_empty());
}

/// Literal values (not just `Ref`s) must index and de-index correctly — the
/// `pos` key is the value's BYTES, and the tagged encoding is what keeps
/// `Int(1)` distinct from `Str("1")`.
#[test]
fn typed_literals_are_distinct_index_keys() {
    let s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let e = s.intern("http://example.org/a").unwrap();
    let a = s.intern("http://example.org/p").unwrap();

    for value in [Value::Int(1), Value::Str("1".to_string())] {
        model.apply(&Datum {
            entity: e,
            attribute: a,
            value,
            valid_from: "2026-01-01T00:00:00Z".to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    assert_eq!(
        model.len(),
        2,
        "Int(1) and Str(\"1\") are different triples"
    );
    assert_eq!(model.by_predicate_object(a, &Value::Int(1)), [e]);
    assert_eq!(model.by_predicate_object(a, &Value::Str("1".into())), [e]);
}

// --- Residency and the scope guard (quipu-syt) -----------------------------

use crate::sparql::{GraphScope, TemporalContext};

/// The resident model is built on first use and reused after that.
#[test]
fn the_resident_model_is_built_lazily() {
    let s = store();
    assert!(
        !s.read_model_is_resident(),
        "nothing built before first use"
    );
    let len = s.read_model().unwrap().len();
    assert!(s.read_model_is_resident(), "first use builds it");
    assert_eq!(s.read_model().unwrap().len(), len, "second use reuses it");
}

/// Every committed write drops it. A stale index that survives a write is the
/// failure mode this whole design has to avoid.
#[test]
fn a_write_invalidates_the_resident_model() {
    let mut s = store();
    let _ = s.read_model().unwrap().len();
    assert!(s.read_model_is_resident());

    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    s.transact(&[d], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();
    assert!(
        !s.read_model_is_resident(),
        "the write left a stale model resident"
    );

    // And the rebuild sees the new fact.
    assert_eq!(s.read_model().unwrap().len(), 1);
}

#[test]
fn the_guard_admits_a_plain_current_root_read() {
    let s = store();
    assert!(read_model_applicable(&s, &TemporalContext::default()));
}

/// Time travel in either axis must reach SQL: the model was built from
/// `valid_to IS NULL` and holds no history, so it would answer the present and
/// call it the past.
#[test]
fn the_guard_refuses_time_travel() {
    let s = store();
    let valid_at = TemporalContext {
        valid_at: Some("2020-01-01T00:00:00Z".to_string()),
        ..Default::default()
    };
    assert!(!read_model_applicable(&s, &valid_at), "valid_at admitted");

    let as_of = TemporalContext {
        as_of_tx: Some(1),
        ..Default::default()
    };
    assert!(!read_model_applicable(&s, &as_of), "as_of_tx admitted");
}

/// Any graph scope but the plain ROOT default reads a different fact set.
/// Overlays need no separate case — an overlay is a named graph.
#[test]
fn the_guard_refuses_every_non_root_graph_scope() {
    let s = store();
    for scope in [
        GraphScope::Default(vec![]),
        GraphScope::Default(vec![7]),
        GraphScope::Default(vec![0, 7]),
        GraphScope::Named(vec![7]),
        GraphScope::AnyNamed {
            var: "g".to_string(),
            restrict: None,
        },
    ] {
        let ctx = TemporalContext {
            graph: scope,
            ..Default::default()
        };
        assert!(
            !read_model_applicable(&s, &ctx),
            "a non-ROOT scope was admitted: {:?}",
            ctx.graph
        );
    }
}

#[test]
fn the_guard_refuses_a_from_named_restriction() {
    let s = store();
    let ctx = TemporalContext {
        named_dataset: Some(vec![7]),
        ..Default::default()
    };
    assert!(!read_model_applicable(&s, &ctx));
}

/// A store with no attachments has an identity `canonical_id` and a plain
/// `facts` source, which is what makes the model equivalent to SQL. The guard
/// keys on that one predicate for both.
#[test]
fn the_guard_keys_on_attachments_for_composition_and_aliasing() {
    let s = store();
    assert!(!s.has_attachments());
    assert!(read_model_applicable(&s, &TemporalContext::default()));
}
