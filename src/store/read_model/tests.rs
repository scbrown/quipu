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

/// A write MAINTAINS the resident model rather than dropping it (quipu-m9h),
/// and the maintained model equals a rebuild. Dropping was the first cut; the
/// rebuild it forced was the whole reason the fast path could not be default.
#[test]
fn a_write_maintains_the_resident_model() {
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
        s.read_model_is_resident(),
        "the write dropped the model instead of maintaining it"
    );
    assert_eq!(s.read_model().unwrap().len(), 1, "the write is not visible");

    let rebuilt = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    assert_eq!(
        s.read_model().unwrap().triples_sorted(),
        rebuilt.triples_sorted(),
        "maintained model diverged from a rebuild"
    );
}

/// A write to a DIFFERENT graph leaves a ROOT model alone —
/// `current_facts_in_graph(0)` never sees those rows, so there is nothing to
/// invalidate.
#[test]
fn a_write_to_another_graph_does_not_disturb_the_root_model() {
    let mut s = store();
    let _ = s.read_model().unwrap().len();
    let before = s.read_model().unwrap().len();

    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let g = s
        .overlay_create("http://example.org/g", crate::schema::ROOT_GRAPH)
        .unwrap();
    s.overlay_write(
        g,
        Op::Assert,
        d.entity,
        d.attribute,
        d.value.clone(),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    assert!(s.read_model_is_resident(), "an unrelated graph dropped it");
    assert_eq!(s.read_model().unwrap().len(), before);
}

/// The model must not be consulted while a write holds an open savepoint: the
/// policy guard queries the pending post-state, which the model has not seen.
#[test]
fn the_guard_refuses_while_a_write_is_in_progress() {
    let s = store();
    s.set_read_model_enabled(true);
    assert!(read_model_applicable(&s, &TemporalContext::default()));
    s.set_write_in_progress(true);
    assert!(
        !read_model_applicable(&s, &TemporalContext::default()),
        "the model was consulted mid-write"
    );
    s.set_write_in_progress(false);
}

/// On by default — see `Store::set_read_model_enabled` for the measurements.
#[test]
fn the_fast_path_is_on_by_default() {
    let s = store();
    assert!(s.read_model_enabled());
    assert!(read_model_applicable(&s, &TemporalContext::default()));
}

#[test]
fn disabling_makes_the_guard_refuse() {
    let s = store();
    s.set_read_model_enabled(false);
    assert!(!read_model_applicable(&s, &TemporalContext::default()));
}

/// A store larger than the budget keeps the SQL path rather than building a
/// model it cannot afford — checked with a COUNT, so an oversized store never
/// pays a build to discover it is oversized.
#[test]
fn the_guard_refuses_a_store_over_the_size_budget() {
    let mut s = store();
    s.set_read_model_max_triples(0);
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    s.transact(&[d], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();
    assert!(!read_model_applicable(&s, &TemporalContext::default()));

    s.set_read_model_max_triples(crate::store::read_model::DEFAULT_READ_MODEL_MAX_TRIPLES);
    assert!(read_model_applicable(&s, &TemporalContext::default()));
}

/// Disabling drops whatever is resident, so a later re-enable cannot serve
/// from a model built under different conditions.
#[test]
fn disabling_drops_the_resident_model() {
    let s = store();
    let _ = s.read_model().unwrap().len();
    assert!(s.read_model_is_resident());
    s.set_read_model_enabled(false);
    assert!(!s.read_model_is_resident());
}

/// Time travel in either axis must reach SQL: the model was built from
/// `valid_to IS NULL` and holds no history, so it would answer the present and
/// call it the past.
#[test]
fn the_guard_refuses_time_travel() {
    let s = store();
    // Enable it, or these would pass because the flag is off rather than
    // because the dimension is refused.
    s.set_read_model_enabled(true);
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

/// Multi-graph and variable scopes read fact sets one model cannot hold —
/// still refused. Single-graph scopes (ROOT or named, quipu-nip) are
/// admitted: a model answers exactly one graph's own facts, which is what a
/// single-graph scope reads. Overlays need no separate case — an overlay is
/// a named graph, and the model holds its RAW rows, the same rows the SQL
/// path's `g IN (…)` filter reads (composition is a different operation).
#[test]
fn the_guard_refuses_unions_and_graph_variables_but_admits_single_graphs() {
    let s = store();
    // Enable it, or these would pass because the flag is off rather than
    // because the dimension is refused.
    s.set_read_model_enabled(true);
    for scope in [
        GraphScope::Default(vec![]),
        GraphScope::Default(vec![0, 7]),
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
            "a non-single scope was admitted: {:?}",
            ctx.graph
        );
    }
    for scope in [GraphScope::Default(vec![7]), GraphScope::Named(vec![7])] {
        let ctx = TemporalContext {
            graph: scope,
            ..Default::default()
        };
        assert!(
            read_model_applicable(&s, &ctx),
            "a single-graph scope was refused (quipu-nip): {:?}",
            ctx.graph
        );
    }
}

#[test]
fn the_guard_refuses_a_from_named_restriction() {
    let s = store();
    // Enable it, or these would pass because the flag is off rather than
    // because the dimension is refused.
    s.set_read_model_enabled(true);
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
    // Enable it, or these would pass because the flag is off rather than
    // because the dimension is refused.
    s.set_read_model_enabled(true);
    assert!(!s.has_attachments());
    assert!(read_model_applicable(&s, &TemporalContext::default()));
}

/// `?s ?p <o>` is the shape SQL serves from `idx_vaet`. Without the osp index
/// the model would have to scan for it, making that pattern a regression rather
/// than a speedup.
#[test]
fn the_object_index_answers_and_de_indexes() {
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
    assert_eq!(model.by_object(&v), [(e, a)]);

    model.apply(&Datum {
        op: Op::Retract,
        ..d
    });
    assert!(model.by_object(&v).is_empty(), "osp still holds it");
}

/// The unbound fallback must enumerate exactly the triples the other indexes
/// agree on.
#[test]
fn iter_triples_matches_the_other_indexes() {
    let mut s = store();
    let mut model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let ds = vec![
        datum(
            &s,
            "http://example.org/a",
            "http://example.org/p",
            "http://example.org/b",
            Op::Assert,
        ),
        datum(
            &s,
            "http://example.org/a",
            "http://example.org/q",
            "http://example.org/c",
            Op::Assert,
        ),
    ];
    assert_incremental_matches_rebuild(&mut s, &mut model, &ds);

    let mut seen: Vec<(i64, i64, Vec<u8>)> = model
        .iter_triples()
        .map(|(e, a, v)| (e, a, v.to_bytes()))
        .collect();
    seen.sort_unstable();
    assert_eq!(seen, model.triples_sorted());
    assert_eq!(seen.len(), 2);
}

/// The acceptance bar for the fast path: the same query, both ways, identical
/// answers. Covers every pattern shape the model indexes differently, because
/// the shape is what selects the index and therefore what could diverge.
#[test]
fn the_model_path_answers_identically_to_sql() {
    let mut s = store();
    let ds = vec![
        datum(
            &s,
            "http://example.org/a",
            "http://example.org/p",
            "http://example.org/b",
            Op::Assert,
        ),
        datum(
            &s,
            "http://example.org/a",
            "http://example.org/q",
            "http://example.org/c",
            Op::Assert,
        ),
        datum(
            &s,
            "http://example.org/d",
            "http://example.org/p",
            "http://example.org/b",
            Op::Assert,
        ),
    ];
    s.transact(&ds, "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();

    // Every query here has TWO patterns, because a single-pattern BGP is routed
    // to SQL in both modes now (the model is only worth building for a join) —
    // so a one-pattern query would compare SQL against SQL and prove nothing.
    // Each case varies the FIRST pattern's shape, which is what selects the
    // index and therefore what could diverge.
    let queries = [
        // fully unbound
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o . ?s <http://example.org/p> ?z }",
        // subject bound
        "SELECT ?p ?o WHERE { <http://example.org/a> ?p ?o . ?o ?p2 ?o2 }",
        // predicate bound
        "SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o . ?s ?p2 ?o2 }",
        // predicate + object bound
        "SELECT ?s WHERE { ?s <http://example.org/p> <http://example.org/b> . ?s ?p2 ?o2 }",
        // object only bound — the shape that needs the osp index
        "SELECT ?s ?p WHERE { ?s ?p <http://example.org/b> . ?s ?p2 ?o2 }",
        // subject + object bound
        "SELECT ?p WHERE { <http://example.org/a> ?p <http://example.org/b> . ?p ?p2 ?o2 }",
        // all three bound
        "SELECT ?z WHERE { <http://example.org/a> <http://example.org/p> <http://example.org/b> . ?z ?p2 ?o2 }",
        // a join on a shared variable
        "SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o . ?s <http://example.org/q> ?c }",
        // a join with NO shared variable — a cartesian product, which the
        // nested loop also produced
        "SELECT ?s ?d WHERE { ?s <http://example.org/q> ?o . ?d <http://example.org/p> ?e }",
        // three patterns, so the join folds more than once
        "SELECT ?s WHERE { ?s <http://example.org/p> ?o . ?s <http://example.org/q> ?c . ?o ?p3 ?o3 }",
    ];

    for q in queries {
        s.set_read_model_enabled(false);
        let via_sql = crate::sparql::query(&s, q).unwrap();
        s.set_read_model_enabled(true);
        let via_model = crate::sparql::query(&s, q).unwrap();

        // Bindings are HashMaps, so Debug key order is per-map and meaningless.
        // Canonicalise each row to sorted (var, value) pairs before comparing,
        // or the test fails on formatting rather than on results.
        let canonical = |r: &crate::sparql::QueryResult| -> Vec<Vec<(String, String)>> {
            let mut rows: Vec<Vec<(String, String)>> = r
                .rows()
                .iter()
                .map(|b| {
                    let mut pairs: Vec<(String, String)> = b
                        .iter()
                        .map(|(k, v)| (k.clone(), format!("{v:?}")))
                        .collect();
                    pairs.sort();
                    pairs
                })
                .collect();
            rows.sort();
            rows
        };
        let sql_rows = canonical(&via_sql);
        let model_rows = canonical(&via_model);
        assert_eq!(
            sql_rows, model_rows,
            "read model disagreed with SQL for: {q}"
        );
        assert_eq!(
            via_sql.variables(),
            via_model.variables(),
            "variable list differed for: {q}"
        );
    }
    s.set_read_model_enabled(false);
}

/// quipu-nip: the guard admits any SINGLE graph scope, and the query path
/// builds that graph its own model — ROOT is not involved.
#[test]
fn a_named_graph_scope_builds_its_own_model() {
    let mut s = store();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let g = s
        .overlay_create("http://example.org/derived", crate::schema::ROOT_GRAPH)
        .unwrap();
    s.overlay_write(
        g,
        Op::Assert,
        d.entity,
        d.attribute,
        d.value.clone(),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    let ctx = TemporalContext {
        graph: crate::sparql::GraphScope::Named(vec![g]),
        ..Default::default()
    };
    assert!(
        read_model_applicable(&s, &ctx),
        "a single named-graph scope must be admitted"
    );
    let len = s.read_model_for(g).unwrap().len();
    assert_eq!(len, 1, "the graph's own fact is in its model");
    assert!(s.read_model_is_resident_for(g));
    assert!(
        !s.read_model_is_resident_for(crate::schema::ROOT_GRAPH),
        "consulting a named graph must not build ROOT's model"
    );
}

/// quipu-nip acceptance: a store with a ROOT past the budget and a small
/// derived graph holds ONLY the derived graph resident — ROOT queries keep
/// the SQL path, derived-graph queries get the fast path.
#[test]
fn only_the_affordable_graph_goes_resident() {
    let mut s = store();
    for i in 0..3 {
        let d = datum(
            &s,
            &format!("http://example.org/e{i}"),
            "http://example.org/p",
            "http://example.org/o",
            Op::Assert,
        );
        s.transact(&[d], "2026-01-01T00:00:00Z", Some("test"), None)
            .unwrap();
    }
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let g = s
        .overlay_create("http://example.org/derived", crate::schema::ROOT_GRAPH)
        .unwrap();
    s.overlay_write(
        g,
        Op::Assert,
        d.entity,
        d.attribute,
        d.value.clone(),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    s.set_read_model_max_triples(1);

    assert!(
        !read_model_applicable(&s, &TemporalContext::default()),
        "ROOT (3 triples) is past the 1-triple budget"
    );
    let ctx = TemporalContext {
        graph: crate::sparql::GraphScope::Named(vec![g]),
        ..Default::default()
    };
    assert!(
        read_model_applicable(&s, &ctx),
        "the 1-triple derived graph fits the budget"
    );
    let _ = s.read_model_for(g).unwrap().len();
    assert!(s.read_model_is_resident_for(g));
    assert!(
        !s.read_model_is_resident_for(crate::schema::ROOT_GRAPH),
        "only the derived graph is resident"
    );
}

/// quipu-nip: the budget bounds the COMBINED resident size — a second model
/// is affordable only if it fits alongside what is already held.
#[test]
fn the_budget_bounds_the_combined_resident_size() {
    let mut s = store();
    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let g1 = s
        .overlay_create("http://example.org/g1", crate::schema::ROOT_GRAPH)
        .unwrap();
    s.overlay_write(
        g1,
        Op::Assert,
        d.entity,
        d.attribute,
        d.value.clone(),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    let g2 = s
        .overlay_create("http://example.org/g2", crate::schema::ROOT_GRAPH)
        .unwrap();
    for i in 0..2 {
        let d2 = datum(
            &s,
            &format!("http://example.org/x{i}"),
            "http://example.org/p",
            "http://example.org/o",
            Op::Assert,
        );
        s.overlay_write(
            g2,
            Op::Assert,
            d2.entity,
            d2.attribute,
            d2.value.clone(),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
    }

    s.set_read_model_max_triples(2);
    let _ = s.read_model_for(g1).unwrap().len(); // 1 triple resident
    assert!(
        !s.read_model_affordable(g2),
        "g2's 2 triples do not fit beside g1's resident 1 under a budget of 2"
    );
    assert!(
        s.read_model_affordable(g1),
        "already-resident stays affordable"
    );
}

/// quipu-nip: a write to one graph maintains that graph's model and leaves
/// every other graph's model untouched.
#[test]
fn per_graph_models_are_maintained_independently() {
    let mut s = store();
    let d_root = datum(
        &s,
        "http://example.org/r",
        "http://example.org/p",
        "http://example.org/o",
        Op::Assert,
    );
    s.transact(&[d_root], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();

    let d = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/b",
        Op::Assert,
    );
    let g = s
        .overlay_create("http://example.org/derived", crate::schema::ROOT_GRAPH)
        .unwrap();
    s.overlay_write(
        g,
        Op::Assert,
        d.entity,
        d.attribute,
        d.value.clone(),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    let root_len = s.read_model().unwrap().len();
    let g_len = s.read_model_for(g).unwrap().len();
    assert_eq!((root_len, g_len), (1, 1));

    // A ROOT write: ROOT's model follows it, g's is untouched.
    let d2 = datum(
        &s,
        "http://example.org/r2",
        "http://example.org/p",
        "http://example.org/o",
        Op::Assert,
    );
    s.transact(&[d2], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();
    assert_eq!(s.read_model().unwrap().len(), 2, "ROOT model maintained");
    assert!(s.read_model_is_resident_for(g), "g's model survived");
    assert_eq!(s.read_model_for(g).unwrap().len(), 1);
}

/// aegis-98gai. A SECOND `Store` handle on the same database — which is exactly
/// what the REST server's read pool is — must not answer from a model built
/// before a write it never saw.
///
/// This is the shape the server actually runs: `server.rs` opens
/// `read_pool_size` independent `Store::open_read_only` connections, and every
/// `ro_handler!` tool takes one of them. A write goes through the writer's
/// handle, so `maintain_read_model` runs on the WRITER's model and nothing ever
/// touches the pooled ones — `invalidate_read_model` has a single call site and
/// it is `set_read_model_enabled(false)`.
///
/// Because only multi-pattern BGPs consult a model, the live symptom was
/// specific and silent: a freshly written node answered `<iri> a ?t` and
/// `<iri> rdfs:label ?l` with one row each and their CONJUNCTION with zero.
/// Measured on the deployed 0.3.24: 4 of 264 `FailureMode` nodes join-invisible,
/// all written in the preceding ~40 minutes; 0 of 264 after the process cycled.
///
/// Both directions: the reader must see the new fact, AND it must still be
/// holding a model (a fix that simply disabled the model would pass a
/// one-directional version of this and give up the join speedup silently).
#[test]
fn a_second_handle_does_not_serve_a_stale_model_after_another_handles_write() {
    let dir = std::env::temp_dir().join(format!(
        "quipu-98gai-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    let db = path.to_str().unwrap();

    let mut writer = Store::open(db).unwrap();
    let first = datum(
        &writer,
        "http://example.org/before",
        "http://example.org/p",
        "http://example.org/o",
        Op::Assert,
    );
    writer
        .transact(&[first], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();

    // A separate handle, as the pool holds. Build its model by reading.
    let reader = Store::open_read_only(db).unwrap();
    assert_eq!(
        reader.read_model().unwrap().len(),
        1,
        "the reader should see the write that preceded it"
    );
    assert!(
        reader.read_model_is_resident(),
        "the reader must have a resident model, or this test proves nothing"
    );

    // A write the reader's handle knows nothing about.
    let second = datum(
        &writer,
        "http://example.org/after",
        "http://example.org/p",
        "http://example.org/o",
        Op::Assert,
    );
    writer
        .transact(&[second], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();

    assert_eq!(
        reader.read_model().unwrap().len(),
        2,
        "the reader served a model built before a write it never saw — this is \
         the join-invisible-node defect (aegis-98gai)"
    );
    assert!(
        reader.read_model_is_resident(),
        "freshness must be kept by REBUILDING, not by abandoning the model"
    );

    drop(reader);
    drop(writer);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The counterpart: a write THIS handle performed must not force a rebuild, or
/// the stamp has undone quipu-m9h's incremental maintenance.
#[test]
fn a_writers_own_write_still_maintains_rather_than_rebuilds_its_model() {
    let mut s = store();
    let d1 = datum(
        &s,
        "http://example.org/a",
        "http://example.org/p",
        "http://example.org/o",
        Op::Assert,
    );
    s.transact(&[d1], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();
    assert_eq!(s.read_model().unwrap().len(), 1);

    let d2 = datum(
        &s,
        "http://example.org/b",
        "http://example.org/p",
        "http://example.org/o",
        Op::Assert,
    );
    s.transact(&[d2], "2026-01-01T00:00:00Z", Some("test"), None)
        .unwrap();

    let latest = s.latest_tx_id().unwrap();
    assert_eq!(
        s.read_model().unwrap().built_at_tx(),
        latest,
        "the writer's own maintained model must be stamped current, or every \
         read after a write pays a full rebuild"
    );
    assert_eq!(s.read_model().unwrap().len(), 2);
}

/// aegis-98gai cost check, not a correctness test: how long does a rebuild take
/// at production scale? Run explicitly:
/// `cargo test --lib read_model_rebuild_cost -- --ignored --nocapture`
///
/// The freshness check makes a pooled reader rebuild after a write it did not
/// perform. That is correct; whether it is AFFORDABLE on a store already
/// measured at 1.187 request-seconds per wall-second (aegis-x5nr6) is a
/// separate question, and shipping a correctness fix onto a saturated,
/// auto-deploying service without the number is how a fix becomes an incident.
#[test]
#[ignore]
fn read_model_rebuild_cost_at_production_scale() {
    // The live store reported 336,323 facts / 20,255 entities on 2026-08-24.
    const ENTITIES: usize = 20_000;
    const PER_ENTITY: usize = 17; // ~340k facts
    let mut s = store();
    let p: Vec<i64> = (0..PER_ENTITY)
        .map(|i| s.intern(&format!("http://example.org/p{i}")).unwrap())
        .collect();
    let o = s.intern("http://example.org/o").unwrap();

    let mut datums = Vec::with_capacity(ENTITIES * PER_ENTITY);
    for e in 0..ENTITIES {
        let ent = s.intern(&format!("http://example.org/e{e}")).unwrap();
        for a in &p {
            datums.push(Datum {
                entity: ent,
                attribute: *a,
                value: Value::Ref(o),
                valid_from: "2026-01-01T00:00:00Z".to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
    }
    let total = datums.len();
    for chunk in datums.chunks(20_000) {
        s.transact(chunk, "2026-01-01T00:00:00Z", Some("bench"), None)
            .unwrap();
    }

    let t0 = std::time::Instant::now();
    let model = ReadModel::build(&s, crate::schema::ROOT_GRAPH).unwrap();
    let build = t0.elapsed();

    let t1 = std::time::Instant::now();
    let latest = s.latest_tx_id().unwrap();
    let stamp = t1.elapsed();

    // The number that decides whether the fix is deployable: what a POOLED
    // READER pays to catch up after one ordinary write it did not perform.
    let one_more = vec![Datum {
        entity: s.intern("http://example.org/late").unwrap(),
        attribute: p[0],
        value: Value::Ref(o),
        valid_from: "2026-01-02T00:00:00Z".to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    s.transact(&one_more, "2026-01-02T00:00:00Z", Some("bench"), None)
        .unwrap();

    let t2 = std::time::Instant::now();
    let delta = s
        .facts_changed_since_in_graph(crate::schema::ROOT_GRAPH, latest)
        .unwrap();
    let catch_up = t2.elapsed();

    println!(
        "REBUILD COST: {} triples in model (wrote {total}), build={:?}, \
         latest_tx_id()={:?} (tx {latest})",
        model.len(),
        build,
        stamp
    );
    println!(
        "CATCH-UP COST after ONE write: {} change(s) read in {:?} \
         (vs {:?} for a full rebuild)",
        delta.len(),
        catch_up,
        build
    );
}

/// The load-bearing one for aegis-98gai's catch-up path: a model brought
/// forward by `facts_changed_since_in_graph` must hold EXACTLY what a rebuild
/// holds, across the write shapes that are not simple appends.
///
/// A retraction does not delete its row — it stamps `valid_to`/`retracted_tx` —
/// so the closing event sits on a row whose own `tx` may predate the model.
/// A delta that filtered on `tx` alone would apply the asserts and silently
/// keep every retracted triple, which is the WORSE failure: the model would
/// answer confidently with facts the store has withdrawn.
///
/// Retract-then-reassert is included deliberately: it is the case that fails if
/// the delta groups asserts and retractions instead of ordering both by the
/// transaction that caused them.
#[test]
fn a_caught_up_model_equals_a_rebuild_across_retractions_and_reasserts() {
    let dir = std::env::temp_dir().join(format!(
        "quipu-98gai-delta-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    let db = path.to_str().unwrap();
    let mut writer = Store::open(db).unwrap();

    let d = |s: &Store, e: &str, o: &str, op: Op| datum(s, e, "http://example.org/p", o, op);

    // Seed, and let a SEPARATE handle build a model over it.
    writer
        .transact(
            &[
                d(
                    &writer,
                    "http://example.org/a",
                    "http://example.org/1",
                    Op::Assert,
                ),
                d(
                    &writer,
                    "http://example.org/b",
                    "http://example.org/1",
                    Op::Assert,
                ),
                d(
                    &writer,
                    "http://example.org/c",
                    "http://example.org/1",
                    Op::Assert,
                ),
            ],
            "2026-01-01T00:00:00Z",
            Some("test"),
            None,
        )
        .unwrap();
    let reader = Store::open_read_only(db).unwrap();
    assert_eq!(reader.read_model().unwrap().len(), 3);

    // Now every awkward shape, through the writer, unseen by the reader.
    // 1. a plain append
    writer
        .transact(
            &[d(
                &writer,
                "http://example.org/d",
                "http://example.org/1",
                Op::Assert,
            )],
            "2026-01-01T00:00:01Z",
            Some("test"),
            None,
        )
        .unwrap();
    // 2. retract a triple the reader's model was BUILT with (its row is older
    //    than the model; only `retracted_tx` marks it)
    writer
        .transact(
            &[d(
                &writer,
                "http://example.org/a",
                "http://example.org/1",
                Op::Retract,
            )],
            "2026-01-01T00:00:02Z",
            Some("test"),
            None,
        )
        .unwrap();
    // 3. retract, then RE-ASSERT the same triple: order decides the answer
    writer
        .transact(
            &[d(
                &writer,
                "http://example.org/b",
                "http://example.org/1",
                Op::Retract,
            )],
            "2026-01-01T00:00:03Z",
            Some("test"),
            None,
        )
        .unwrap();
    writer
        .transact(
            &[d(
                &writer,
                "http://example.org/b",
                "http://example.org/1",
                Op::Assert,
            )],
            "2026-01-01T00:00:04Z",
            Some("test"),
            None,
        )
        .unwrap();
    // 4. assert then retract within the reader's stale window: must end ABSENT
    writer
        .transact(
            &[d(
                &writer,
                "http://example.org/e",
                "http://example.org/1",
                Op::Assert,
            )],
            "2026-01-01T00:00:05Z",
            Some("test"),
            None,
        )
        .unwrap();
    writer
        .transact(
            &[d(
                &writer,
                "http://example.org/e",
                "http://example.org/1",
                Op::Retract,
            )],
            "2026-01-01T00:00:06Z",
            Some("test"),
            None,
        )
        .unwrap();

    let caught_up = reader.read_model().unwrap().triples_sorted();
    let rebuilt = ReadModel::build(&reader, crate::schema::ROOT_GRAPH)
        .unwrap()
        .triples_sorted();
    assert_eq!(
        caught_up, rebuilt,
        "the caught-up model diverged from a rebuild — a drifting index answers \
         confidently with facts the store retracted"
    );
    // And it is the RIGHT content, not merely two equal wrong answers: c and d
    // remain, b was retracted and re-asserted, a and e are gone.
    assert_eq!(caught_up.len(), 3, "expected b, c, d");

    drop(reader);
    drop(writer);
    let _ = std::fs::remove_dir_all(&dir);
}
