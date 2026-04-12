//! Integration tests for the reactive reasoner (Phase 3).
//!
//! These exercise the TransactObserver path: register a ReactiveReasoner
//! on a store, transact base facts, and verify that derived facts appear
//! automatically without a separate `evaluate()` call.

use std::sync::Arc;

use super::RULE_NS;
use super::parse::parse_rules;
use super::reactive::ReactiveReasoner;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const PFX: &str = "http://ex/";
const TS: &str = "2026-04-07T00:00:00Z";

/// Count facts whose attribute matches `predicate` and whose transaction
/// source matches `source`.
fn count_derived(store: &Store, predicate: &str, source: &str) -> usize {
    let attr = match store.lookup(predicate).unwrap() {
        Some(id) => id,
        None => return 0,
    };
    let mut stmt = store
        .conn
        .prepare(
            "SELECT COUNT(*) FROM facts f \
             JOIN transactions t ON f.tx = t.id \
             WHERE f.a = ?1 AND t.source = ?2 \
               AND f.op = 1 AND f.valid_to IS NULL",
        )
        .unwrap();
    let count: i64 = stmt
        .query_row(rusqlite::params![attr, source], |row| row.get(0))
        .unwrap();
    usize::try_from(count).expect("non-negative count")
}

/// Count all current facts for a given predicate (any source).
fn count_facts_for_pred(store: &Store, predicate: &str) -> usize {
    let attr = match store.lookup(predicate).unwrap() {
        Some(id) => id,
        None => return 0,
    };
    let mut stmt = store
        .conn
        .prepare(
            "SELECT COUNT(*) FROM facts \
             WHERE a = ?1 AND op = 1 AND valid_to IS NULL",
        )
        .unwrap();
    let count: i64 = stmt
        .query_row(rusqlite::params![attr], |row| row.get(0))
        .unwrap();
    usize::try_from(count).expect("non-negative count")
}

fn setup_store_with_observer(ttl: &str) -> Store {
    let rs = parse_rules(ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let observer = Arc::new(ReactiveReasoner::new(rs));
    store.add_observer(observer);
    store
}

#[test]
fn reactive_derives_on_base_fact_insert() {
    // Rule: h(?x, ?y) :- p(?x, ?y)
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let mut store = setup_store_with_observer(&ttl);

    // Transact a base fact — the reactive observer should auto-derive.
    let s = store.intern("ex:a").unwrap();
    let p = store.intern(&format!("{PFX}p")).unwrap();
    let o = store.intern("ex:b").unwrap();
    store
        .transact(
            &[Datum {
                entity: s,
                attribute: p,
                value: Value::Ref(o),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    // The derived fact h(ex:a, ex:b) should exist now.
    assert_eq!(
        count_derived(&store, &format!("{PFX}h"), "reasoner:R1"),
        1,
        "reactive observer should have derived h(ex:a, ex:b)"
    );
}

#[test]
fn reactive_retracts_when_base_fact_removed() {
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let mut store = setup_store_with_observer(&ttl);

    let s = store.intern("ex:a").unwrap();
    let p = store.intern(&format!("{PFX}p")).unwrap();
    let o = store.intern("ex:b").unwrap();

    // Insert base fact → triggers derivation.
    store
        .transact(
            &[Datum {
                entity: s,
                attribute: p,
                value: Value::Ref(o),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();
    assert_eq!(count_derived(&store, &format!("{PFX}h"), "reasoner:R1"), 1);

    // Retract the base fact → derived should disappear.
    let ts2 = "2026-04-07T01:00:00Z";
    store
        .transact(
            &[Datum {
                entity: s,
                attribute: p,
                value: Value::Ref(o),
                valid_from: ts2.to_string(),
                valid_to: None,
                op: Op::Retract,
            }],
            ts2,
            Some("test"),
            Some("base"),
        )
        .unwrap();
    assert_eq!(
        count_derived(&store, &format!("{PFX}h"), "reasoner:R1"),
        0,
        "reactive observer should have retracted h(ex:a, ex:b)"
    );
}

#[test]
fn reactive_skips_own_output() {
    // Verify the observer doesn't re-trigger on its own derived facts.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let observer = Arc::new(ReactiveReasoner::new(rs));
    let obs_ref = Arc::clone(&observer);
    store.add_observer(observer);

    let s = store.intern("ex:a").unwrap();
    let p = store.intern(&format!("{PFX}p")).unwrap();
    let o = store.intern("ex:b").unwrap();
    store
        .transact(
            &[Datum {
                entity: s,
                attribute: p,
                value: Value::Ref(o),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    // Should have triggered exactly once.
    let stats = obs_ref.stats();
    assert_eq!(
        stats.triggers, 1,
        "observer should fire once for the base fact"
    );
    assert_eq!(stats.total_asserted, 1);
}

#[test]
fn reactive_handles_two_atom_join() {
    // affects(?pkg, ?svc) :- installedIn(?pkg, ?c), runsService(?c, ?svc)
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "affects(?pkg, ?svc)" ;
    rule:body "installedIn(?pkg, ?c), runsService(?c, ?svc)" .
"#
    );
    let mut store = setup_store_with_observer(&ttl);

    // Set up the join: nginx is installed in ctA, ctA runs proxy.
    let nginx = store.intern("ex:nginx").unwrap();
    let ct_a = store.intern("ex:ctA").unwrap();
    let proxy = store.intern("ex:proxy").unwrap();
    let installed = store.intern(&format!("{PFX}installedIn")).unwrap();
    let runs = store.intern(&format!("{PFX}runsService")).unwrap();

    // First transact: installedIn(nginx, ctA) — no derivation yet (need both)
    store
        .transact(
            &[Datum {
                entity: nginx,
                attribute: installed,
                value: Value::Ref(ct_a),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();
    assert_eq!(
        count_facts_for_pred(&store, &format!("{PFX}affects")),
        0,
        "no derivation yet — need both sides of join"
    );

    // Second transact: runsService(ctA, proxy) — now the join completes.
    store
        .transact(
            &[Datum {
                entity: ct_a,
                attribute: runs,
                value: Value::Ref(proxy),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();
    assert_eq!(
        count_derived(&store, &format!("{PFX}affects"), "reasoner:R1"),
        1,
        "reactive observer should derive affects(nginx, proxy)"
    );
}

#[test]
fn reactive_transitive_dependency_propagates() {
    // R1: h(?x, ?y) :- p(?x, ?y)
    // R2: g(?x, ?y) :- h(?x, ?y)  (depends on R1's output)
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
ex:r2 a rule:Rule ;
    rule:id "R2" ;
    rule:head "g(?x, ?y)" ;
    rule:body "h(?x, ?y)" .
"#
    );
    let mut store = setup_store_with_observer(&ttl);

    let s = store.intern("ex:a").unwrap();
    let p = store.intern(&format!("{PFX}p")).unwrap();
    let o = store.intern("ex:b").unwrap();
    store
        .transact(
            &[Datum {
                entity: s,
                attribute: p,
                value: Value::Ref(o),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    // Both R1 and R2 should have derived.
    assert_eq!(
        count_derived(&store, &format!("{PFX}h"), "reasoner:R1"),
        1,
        "R1 should derive h(a, b)"
    );
    assert_eq!(
        count_derived(&store, &format!("{PFX}g"), "reasoner:R2"),
        1,
        "R2 should transitively derive g(a, b)"
    );
}

#[test]
fn unrelated_predicate_does_not_trigger() {
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let observer = Arc::new(ReactiveReasoner::new(rs));
    let obs_ref = Arc::clone(&observer);
    store.add_observer(observer);

    // Transact a fact on predicate `q` — unrelated to the rule.
    let s = store.intern("ex:a").unwrap();
    let q = store.intern(&format!("{PFX}q")).unwrap();
    let o = store.intern("ex:b").unwrap();
    store
        .transact(
            &[Datum {
                entity: s,
                attribute: q,
                value: Value::Ref(o),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    // Observer should not have triggered.
    let stats = obs_ref.stats();
    assert_eq!(
        stats.triggers, 0,
        "unrelated predicate should not trigger observer"
    );
}
