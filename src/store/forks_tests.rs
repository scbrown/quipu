//! Tests for persistent named forks (quipu-gp5) — one per acceptance
//! criterion, plus the refusal paths that keep fork ergonomics from becoming a
//! gate bypass.

use super::*;
use crate::sparql::{GraphScope, TemporalContext, query_temporal};

const TS: &str = "2026-08-22T00:00:00Z";

fn datum(e: i64, a: i64, v: Value, op: Op) -> Datum {
    Datum {
        entity: e,
        attribute: a,
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op,
    }
}

/// A store with two ROOT transactions: tx A asserts alice's name and a knows
/// edge; tx B retracts the name and asserts a replacement. Returns
/// `(store, tx_a, (alice, name, knows, bob))`.
fn two_epoch_store() -> (Store, i64, (i64, i64, i64, i64)) {
    let mut store = Store::open_in_memory().unwrap();
    let alice = store.intern("urn:t:alice").unwrap();
    let bob = store.intern("urn:t:bob").unwrap();
    let name = store.intern("urn:t:name").unwrap();
    let knows = store.intern("urn:t:knows").unwrap();
    let tx_a = store
        .transact(
            &[
                datum(alice, name, Value::Str("v1".into()), Op::Assert),
                datum(alice, knows, Value::Ref(bob), Op::Assert),
            ],
            TS,
            None,
            Some("test"),
        )
        .unwrap();
    store
        .transact(
            &[
                datum(alice, name, Value::Str("v1".into()), Op::Retract),
                datum(alice, name, Value::Str("v2".into()), Op::Assert),
            ],
            TS,
            None,
            Some("test"),
        )
        .unwrap();
    (store, tx_a, (alice, name, knows, bob))
}

fn triple_set(facts: &[crate::types::Fact]) -> std::collections::BTreeSet<(i64, i64, Vec<u8>)> {
    facts
        .iter()
        .map(|f| (f.entity, f.attribute, f.value.to_bytes()))
        .collect()
}

// ---------------------------------------------------------------------------
// Create + read: querying the fork equals querying ROOT as of the fork tx
// ---------------------------------------------------------------------------

#[test]
fn fork_snapshot_equals_root_as_of_tx() {
    let (mut store, tx_a, (alice, name, _, _)) = two_epoch_store();
    let fork = store.fork_create("epoch-a", tx_a, TS, None).unwrap();
    assert_eq!(fork.status, "open");
    assert_eq!(fork.fork_tx, tx_a);

    // The SAME SPARQL query, once scoped to the fork's graph and once as an
    // as-of-tx read on ROOT, must agree — that is the whole point of the
    // materialization predicate matching the #83 as-of predicate.
    let sparql = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    let via_fork = query_temporal(
        &store,
        sparql,
        &TemporalContext {
            graph: GraphScope::Default(vec![fork.g]),
            ..Default::default()
        },
    )
    .unwrap();
    let via_as_of = query_temporal(
        &store,
        sparql,
        &TemporalContext {
            as_of_tx: Some(tx_a),
            ..Default::default()
        },
    )
    .unwrap();
    let rows = |r: &crate::sparql::QueryResult| match r {
        crate::sparql::QueryResult::Select { rows, .. } => {
            // Via a BTreeMap: a HashMap's Debug order is not deterministic.
            let mut out: Vec<String> = rows
                .iter()
                .map(|row| {
                    let sorted: std::collections::BTreeMap<_, _> = row.iter().collect();
                    format!("{sorted:?}")
                })
                .collect();
            out.sort();
            out
        }
        other => panic!("expected SELECT, got {other:?}"),
    };
    assert_eq!(rows(&via_fork), rows(&via_as_of));

    // And the fork sees epoch A, not the present: name is "v1", not "v2".
    let fork_facts = store.current_facts_in_graph(fork.g).unwrap();
    let names: Vec<_> = fork_facts
        .iter()
        .filter(|f| f.entity == alice && f.attribute == name)
        .collect();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].value, Value::Str("v1".into()));
}

#[test]
fn fork_create_refuses_bad_tx_and_bad_and_duplicate_names() {
    let (mut store, tx_a, _) = two_epoch_store();
    let latest = store.latest_tx_id().unwrap();
    assert!(store.fork_create("f", latest + 1, TS, None).is_err());
    assert!(store.fork_create("f", -1, TS, None).is_err());
    assert!(store.fork_create("no spaces", tx_a, TS, None).is_err());
    assert!(store.fork_create("", tx_a, TS, None).is_err());
    store.fork_create("f", tx_a, TS, None).unwrap();
    let err = store.fork_create("f", tx_a, TS, None).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
    // A refused create leaves no registry row behind it.
    assert_eq!(store.fork_list().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Isolation: ROOT moves on, the fork does not
// ---------------------------------------------------------------------------

#[test]
fn root_writes_after_fork_do_not_appear_in_the_fork() {
    let (mut store, tx_a, (alice, _, _, _)) = two_epoch_store();
    let fork = store.fork_create("frozen", tx_a, TS, None).unwrap();
    let before = triple_set(&store.current_facts_in_graph(fork.g).unwrap());

    let age = store.intern("urn:t:age").unwrap();
    store
        .transact(
            &[datum(alice, age, Value::Int(30), Op::Assert)],
            TS,
            None,
            None,
        )
        .unwrap();

    let after = triple_set(&store.current_facts_in_graph(fork.g).unwrap());
    assert_eq!(before, after, "a ROOT write leaked into the fork");
}

// ---------------------------------------------------------------------------
// Diff: structural, present-state, both directions
// ---------------------------------------------------------------------------

#[test]
fn diff_detects_an_added_and_a_removed_triple() {
    let (mut store, _, (alice, name, knows, bob)) = two_epoch_store();
    let latest = store.latest_tx_id().unwrap();
    let fork = store.fork_create("work", latest, TS, None).unwrap();

    // Diverge the fork: add one triple, retract one. Writing straight to the
    // fork's committed graph is the sanctioned v1 write path (it is a
    // registered committed graph, so /knot's graph param reaches it too).
    let color = store.intern("urn:t:color").unwrap();
    store
        .transact_to_graph(
            &[
                datum(alice, color, Value::Str("teal".into()), Op::Assert),
                datum(alice, knows, Value::Ref(bob), Op::Retract),
            ],
            TS,
            None,
            None,
            fork.g,
        )
        .unwrap();

    let diff = store.fork_diff("main", "work").unwrap();
    assert_eq!(diff.added.len(), 1, "one added triple");
    assert_eq!(diff.added[0].attribute, color);
    assert_eq!(diff.removed.len(), 1, "one removed triple");
    assert_eq!(diff.removed[0].attribute, knows);

    // Swapping sides swaps the signs.
    let rev = store.fork_diff("work", "main").unwrap();
    assert_eq!(rev.added.len(), 1);
    assert_eq!(rev.added[0].attribute, knows);
    assert_eq!(rev.removed.len(), 1);

    // An untouched pair diffs empty.
    let same = store.fork_diff("main", "main").unwrap();
    assert!(same.added.is_empty() && same.removed.is_empty());
    let _ = name;
}

// ---------------------------------------------------------------------------
// Drop: terminal, and blocks further fork ops
// ---------------------------------------------------------------------------

#[test]
fn drop_marks_dropped_and_blocks_further_ops() {
    let (mut store, tx_a, _) = two_epoch_store();
    let fork = store.fork_create("doomed", tx_a, TS, None).unwrap();
    store.fork_drop("doomed", TS).unwrap();

    let f = store.fork_lookup("doomed").unwrap().unwrap();
    assert_eq!(f.status, "dropped");
    // The facts are left in place (history), but the name is no longer a read
    // surface, a promote target, a drop target, or reusable.
    assert!(!store.current_facts_in_graph(fork.g).unwrap().is_empty());
    assert!(store.fork_graph_for_read("doomed").is_err());
    assert!(store.fork_promote("doomed", TS, None).is_err());
    assert!(store.fork_drop("doomed", TS).is_err());
    assert!(store.fork_create("doomed", tx_a, TS, None).is_err());
    assert!(store.fork_diff("main", "doomed").is_err());
}

// ---------------------------------------------------------------------------
// Promote: the delta lands on ROOT through the gates
// ---------------------------------------------------------------------------

#[test]
fn promote_applies_the_delta_to_root() {
    let (mut store, _, (alice, _, knows, bob)) = two_epoch_store();
    let latest = store.latest_tx_id().unwrap();
    let fork = store.fork_create("work", latest, TS, None).unwrap();
    let color = store.intern("urn:t:color").unwrap();
    store
        .transact_to_graph(
            &[
                datum(alice, color, Value::Str("teal".into()), Op::Assert),
                datum(alice, knows, Value::Ref(bob), Op::Retract),
            ],
            TS,
            None,
            None,
            fork.g,
        )
        .unwrap();

    match store.fork_promote("work", TS, None).unwrap() {
        ForkPromotion::Promoted {
            tx,
            asserted,
            retracted,
        } => {
            assert!(tx > 0);
            assert_eq!(asserted, 1);
            assert_eq!(retracted, 1);
        }
        #[cfg(feature = "shacl")]
        ForkPromotion::Refused(f) => panic!("unexpected refusal: {f:?}"),
    }

    // ROOT now matches the fork's present state.
    let diff = store.fork_diff("main", "work").unwrap();
    assert!(diff.added.is_empty() && diff.removed.is_empty(), "{diff:?}");
    assert_eq!(
        store.fork_lookup("work").unwrap().unwrap().status,
        "promoted"
    );
    // Promotion is terminal; a promoted fork stays readable but not re-promotable.
    assert!(store.fork_promote("work", TS, None).is_err());
    assert!(store.fork_graph_for_read("work").is_ok());
}

/// The constraint the bead states up front: promotion must re-enter through
/// SHACL, and a refusal must be a REAL refusal — nothing written.
#[cfg(feature = "shacl")]
#[test]
fn promote_is_refused_by_shacl_and_root_is_untouched() {
    let (mut store, _, _) = two_epoch_store();
    store
        .load_shapes(
            "person-name",
            r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:t:> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:minCount 1 ;
    ] .
"#,
            TS,
        )
        .unwrap();

    let latest = store.latest_tx_id().unwrap();
    let fork = store.fork_create("bad", latest, TS, None).unwrap();
    // A Person with no name — violates the stored shape.
    let carol = store.intern("urn:t:carol").unwrap();
    let rdf_type = store.intern(crate::namespace::RDF_TYPE).unwrap();
    let person = store.intern("urn:t:Person").unwrap();
    store
        .transact_to_graph(
            &[datum(carol, rdf_type, Value::Ref(person), Op::Assert)],
            TS,
            None,
            None,
            fork.g,
        )
        .unwrap();

    let root_before = triple_set(&store.current_facts().unwrap());
    match store.fork_promote("bad", TS, None).unwrap() {
        ForkPromotion::Refused(feedback) => {
            assert!(!feedback.conforms);
            assert!(feedback.violations > 0);
        }
        ForkPromotion::Promoted { .. } => panic!("a SHACL-violating delta was promoted"),
    }
    // Nothing written, and the fork stays open for repair.
    assert_eq!(root_before, triple_set(&store.current_facts().unwrap()));
    assert_eq!(store.fork_lookup("bad").unwrap().unwrap().status, "open");
}

// ---------------------------------------------------------------------------
// Registry plumbing: meta-graph mirror + events
// ---------------------------------------------------------------------------

#[test]
fn fork_create_mirrors_the_meta_graph_and_emits_an_event() {
    let (mut store, tx_a, _) = two_epoch_store();
    let fork = store.fork_create("audited", tx_a, TS, None).unwrap();

    let meta_g = store.meta_graph_id().unwrap();
    let meta = store.current_facts_in_graph(meta_g).unwrap();
    let o_fork = store.lookup(crate::namespace::QUIPU_FORK).unwrap().unwrap();
    assert!(
        meta.iter()
            .any(|f| f.entity == fork.g && f.value == Value::Ref(o_fork)),
        "meta-graph missing the quipu:Fork typing"
    );
    assert!(
        meta.iter()
            .any(|f| f.entity == fork.g && f.value == Value::Int(tx_a)),
        "meta-graph missing the quipu:forkTx pin"
    );

    let n: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'fork.created' AND subject = 'audited'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "fork.created event not emitted");
}
