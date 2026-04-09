//! Integration tests for the reasoner evaluator.
//!
//! These exercise the full round-trip: load rules, seed the EAVT store
//! with base facts, call [`evaluate`], and assert that the derived facts
//! show up in `current_facts` with the expected `reasoner:<rule-id>`
//! provenance tag.

use super::parse::parse_rules;
use super::{RULE_NS, evaluate};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const PFX: &str = "http://ex/";
const TS: &str = "2026-04-07T00:00:00Z";

/// Intern a predicate and assert a `(subject, predicate, object)` triple
/// into the store.
fn assert_triple(store: &mut Store, subject: &str, predicate: &str, object: &str) -> i64 {
    let s = store.intern(subject).expect("intern subject");
    let p = store.intern(predicate).expect("intern predicate");
    let o = store.intern(object).expect("intern object");
    let datum = Datum {
        entity: s,
        attribute: p,
        value: Value::Ref(o),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(&[datum], TS, Some("test"), Some("base"))
        .expect("transact base fact");
    o
}

/// Count facts whose attribute matches `predicate` and whose transaction
/// source matches `source`.
fn count_derived(store: &Store, predicate: &str, source: &str) -> usize {
    let attr = store
        .lookup(predicate)
        .expect("lookup")
        .expect("predicate should exist after derivation");
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

#[test]
fn empty_ruleset_is_a_noop() {
    let mut store = Store::open_in_memory().unwrap();
    let rs = super::parse::RuleSet::empty(PFX);
    let report = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(report.asserted, 0);
    assert_eq!(report.retracted, 0);
    assert_eq!(report.strata_run, 0);
}

#[test]
fn single_atom_projection_derives_facts() {
    // Rule: `h(?x, ?y) :- p(?x, ?y)` — simple projection.
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
    assert_triple(&mut store, "ex:a", &format!("{PFX}p"), "ex:b");
    assert_triple(&mut store, "ex:c", &format!("{PFX}p"), "ex:d");

    let report = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(report.asserted, 2);
    assert_eq!(report.retracted, 0);
    assert_eq!(count_derived(&store, &format!("{PFX}h"), "reasoner:R1"), 2);
}

#[test]
fn two_atom_join_derives_cross_product_via_shared_var() {
    // `affects(?pkg, ?svc) :- installedIn(?pkg, ?c), runsService(?c, ?svc)`
    // With one package in a container running two services, we expect two
    // affects tuples.
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
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    assert_triple(
        &mut store,
        "ex:nginx",
        &format!("{PFX}installedIn"),
        "ex:ctA",
    );
    assert_triple(
        &mut store,
        "ex:ctA",
        &format!("{PFX}runsService"),
        "ex:proxy",
    );
    assert_triple(&mut store, "ex:ctA", &format!("{PFX}runsService"), "ex:api");

    let report = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(report.asserted, 2);
    assert_eq!(
        count_derived(&store, &format!("{PFX}affects"), "reasoner:R1"),
        2
    );
}

#[test]
fn recursive_rule_computes_transitive_closure() {
    // `dependsOn(?a, ?c) :- dependsOn(?a, ?b), dependsOn(?b, ?c)`
    // Seeded with a→b and b→c, the closure also gives us a→c.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    assert_triple(&mut store, "ex:a", &format!("{PFX}dependsOn"), "ex:b");
    assert_triple(&mut store, "ex:b", &format!("{PFX}dependsOn"), "ex:c");

    let report = evaluate(&mut store, &rs, TS).unwrap();
    // Only the (a,c) closure tuple is new — (a,b) and (b,c) are base facts.
    assert_eq!(report.asserted, 1);

    // Re-running should be a no-op: the derivation is already persisted.
    let second = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(second.asserted, 0);
    assert_eq!(second.retracted, 0);
}

#[test]
fn retracted_base_fact_triggers_retraction_of_derived_fact() {
    // Derive `h(?x, ?y) :- p(?x, ?y)`, then retract the single base fact
    // and re-run. The derived fact must be retracted by the second call.
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
    let a = store.intern("ex:a").unwrap();
    let b = store.intern("ex:b").unwrap();
    let p = store.intern(&format!("{PFX}p")).unwrap();
    store
        .transact(
            &[Datum {
                entity: a,
                attribute: p,
                value: Value::Ref(b),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    let first = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(first.asserted, 1);

    // Retract the base fact. Use a different timestamp so valid-time ranges
    // don't collapse.
    let ts2 = "2026-04-07T01:00:00Z";
    store
        .transact(
            &[Datum {
                entity: a,
                attribute: p,
                value: Value::Ref(b),
                valid_from: ts2.to_string(),
                valid_to: None,
                op: Op::Retract,
            }],
            ts2,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    let second = evaluate(&mut store, &rs, ts2).unwrap();
    assert_eq!(second.asserted, 0);
    assert_eq!(second.retracted, 1);
    assert_eq!(count_derived(&store, &format!("{PFX}h"), "reasoner:R1"), 0);
}
