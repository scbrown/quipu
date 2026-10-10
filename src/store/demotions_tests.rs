//! Demotion evidence: atomicity, identity, isolation and provenance.
use super::demotions::{DEMOTED, NS, STATE};
use super::{Datum, Store};
use crate::namespace::RDF_TYPE;
use crate::types::{Op, Value};
use std::collections::BTreeSet;

const TS: &str = "2026-09-26T00:00:00Z";
const SOURCE: &str = "reasoner:lift";

fn fixture() -> (Store, i64, i64, i64, i64, i64) {
    let mut store = Store::open_in_memory().unwrap();
    let g = store.intern("urn:test:premise").unwrap();
    let c = store.ensure_companion_inferred_graph(g, TS).unwrap();
    let s = store.intern("urn:test:subject").unwrap();
    let p = store.intern("urn:test:predicate").unwrap();
    let o = store.intern("urn:test:object").unwrap();
    let d = Datum {
        entity: s,
        attribute: p,
        value: Value::Ref(o),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact_to_graph(&[d], TS, Some("promoter"), Some(SOURCE), g)
        .unwrap();
    (store, g, c, s, p, o)
}

#[test]
fn demotion_evidence_is_atomic_on_second_batch_failure() {
    let (mut store, g, c, s, p, o) = fixture();
    let head = store.transaction_head().unwrap();
    store
        .conn
        .execute_batch(&format!(
            "CREATE TEMP TRIGGER fail_evidence BEFORE INSERT ON facts WHEN NEW.g={c} \
         BEGIN SELECT RAISE(ABORT, 'injected evidence failure'); END;"
        ))
        .unwrap();
    let error = store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap_err();
    assert!(
        error.to_string().contains("injected evidence failure"),
        "{error}"
    );
    assert_eq!(store.transaction_head().unwrap(), head);
    assert!(
        store
            .current_facts_in_graph(g)
            .unwrap()
            .iter()
            .any(|f| f.entity == s && f.attribute == p && f.value == Value::Ref(o))
    );
    assert!(!store.current_facts_in_graph(c).unwrap().iter().any(|f| {
        store
            .lookup(DEMOTED)
            .unwrap()
            .is_some_and(|id| f.value == Value::Ref(id))
    }));
    store
        .conn
        .execute_batch("DROP TRIGGER fail_evidence")
        .unwrap();
    store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap();
    assert!(store.current_facts_in_graph(g).unwrap().is_empty());
}

#[test]
fn demotion_identity_is_stable_and_records_real_transactions() {
    let (mut store, g, c, s, p, o) = fixture();
    let promotion = store.transaction_head().unwrap();
    store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap();
    let before = store.current_facts_in_graph(c).unwrap();
    store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap();
    let after = store.current_facts_in_graph(c).unwrap();
    assert_eq!(before.len(), after.len());
    let class = Value::Ref(store.lookup(DEMOTED).unwrap().unwrap());
    let records: Vec<_> = after
        .iter()
        .filter(|f| f.attribute == store.lookup(RDF_TYPE).unwrap().unwrap() && f.value == class)
        .collect();
    assert_eq!(records.len(), 1);
    let record = records[0].entity;
    let field = |name: &str| {
        after
            .iter()
            .find(|f| {
                f.entity == record
                    && f.attribute == store.lookup(&format!("{NS}{name}")).unwrap().unwrap()
            })
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(field("promotionTx"), Value::Int(promotion));
    let actual: i64 = store
        .conn
        .query_row(
            "SELECT retracted_tx FROM facts WHERE e=?1 AND a=?2 AND v=?3 AND g=?4 AND op=1",
            rusqlite::params![s, p, Value::Ref(o).to_bytes(), g],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(field("invalidationTx"), Value::Int(actual));
    assert_eq!(store.without_plane_metadata(after).unwrap().len(), 0);
}

#[test]
fn demotion_reification_is_not_a_datalog_premise() {
    let (mut store, g, c, _, p, _) = fixture();
    store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap();
    let rules = crate::reasoner::parse_rules(
        &format!(
            r#"
        @prefix rule: <{}> .
        <urn:test:copy> a rule:Rule ; rule:id "EVIDENCE" ;
        rule:head "<urn:test:leaked>(?s, ?o)" ;
        rule:body "<http://www.w3.org/1999/02/22-rdf-syntax-ns#subject>(?s, ?o)" .
    "#,
            crate::reasoner::RULE_NS
        ),
        None,
    )
    .unwrap();
    let result = crate::reasoner::evaluate_in_graph(&mut store, &rules, TS, g).unwrap();
    assert_eq!(result.asserted, 0);
    // Same predicate from an ordinary premise must still match.
    let d = Datum {
        entity: store.intern("urn:test:ordinary").unwrap(),
        attribute: store
            .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#subject")
            .unwrap(),
        value: Value::Ref(store.intern("urn:test:value").unwrap()),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact_to_graph(&[d], TS, None, Some("base"), g)
        .unwrap();
    assert_eq!(
        crate::reasoner::evaluate_in_graph(&mut store, &rules, TS, g)
            .unwrap()
            .asserted,
        1
    );
    let state = store.lookup(STATE).unwrap().unwrap();
    assert_eq!(
        store
            .current_facts_in_graph(c)
            .unwrap()
            .iter()
            .filter(|f| f.attribute == state)
            .count(),
        1
    );
}

#[test]
fn demotion_reification_is_not_an_rdfs_premise() {
    let (mut store, g, c, _, p, _) = fixture();
    store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap();
    let record = store
        .current_facts_in_graph(c)
        .unwrap()
        .into_iter()
        .find(|f| f.attribute == store.lookup(STATE).unwrap().unwrap())
        .unwrap()
        .entity;
    let schema = Datum {
        entity: store
            .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#subject")
            .unwrap(),
        attribute: store.intern(crate::namespace::RDFS_DOMAIN).unwrap(),
        value: Value::Ref(
            store
                .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#Statement")
                .unwrap(),
        ),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact_to_graph(&[schema], TS, None, Some("schema"), g)
        .unwrap();
    crate::sparql::rdfs_closure::materialise(&mut store, g, TS).unwrap();
    let after = store.current_facts_in_graph(c).unwrap();
    assert_eq!(
        after.iter().filter(|f| f.entity == record).count(),
        10,
        "RDFS must not add even rdf:Statement typing to the evidence record"
    );
    let d = Datum {
        entity: store.intern("urn:test:ordinary").unwrap(),
        attribute: store
            .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#subject")
            .unwrap(),
        value: Value::Ref(store.intern("urn:test:value").unwrap()),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    let ordinary = d.entity;
    store
        .transact_to_graph(&[d], TS, None, Some("base"), g)
        .unwrap();
    crate::sparql::rdfs_closure::materialise(&mut store, g, TS).unwrap();
    let statement = Value::Ref(
        store
            .lookup("http://www.w3.org/1999/02/22-rdf-syntax-ns#Statement")
            .unwrap()
            .unwrap(),
    );
    assert!(
        store
            .current_facts_in_graph(c)
            .unwrap()
            .iter()
            .any(|f| f.entity == ordinary && f.value == statement)
    );
}

#[cfg(feature = "owl")]
#[test]
fn demotion_reification_is_not_an_owl_premise_full_or_delta() {
    let mut store = Store::open_in_memory().unwrap();
    let c = store.ensure_companion_inferred_graph(0, TS).unwrap();
    let s = store.intern("urn:test:s").unwrap();
    let p = store.intern("urn:test:p").unwrap();
    let o = store.intern("urn:test:o").unwrap();
    let d = Datum {
        entity: s,
        attribute: p,
        value: Value::Ref(o),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[d], TS, None, Some(SOURCE)).unwrap();
    store
        .reconcile_promoted_derivations(p, SOURCE, 0, c, &BTreeSet::new(), TS)
        .unwrap();
    let ontology = crate::owl::Ontology::from_turtle(
        r#"
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        <http://www.w3.org/1999/02/22-rdf-syntax-ns#subject> rdfs:domain <urn:test:Evidence> .
    "#,
    )
    .unwrap();
    let seed = store.current_facts_in_graph(c).unwrap();
    assert_eq!(
        ontology
            .materialize_delta(&mut store, TS, &seed)
            .unwrap()
            .total,
        0
    );
    assert_eq!(ontology.materialize(&mut store, TS).unwrap().total, 0);
    let d = Datum {
        entity: store.intern("urn:test:ordinary").unwrap(),
        attribute: store
            .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#subject")
            .unwrap(),
        value: Value::Ref(o),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[d], TS, None, Some("base")).unwrap();
    assert!(ontology.materialize(&mut store, TS).unwrap().total > 0);
}

#[test]
fn demotion_does_not_change_the_application_vocabulary_gate() {
    let (mut store, g, c, _, p, _) = fixture();
    let ordinary = "<urn:test:new> a <urn:test:ApplicationType> .";
    crate::vocabulary::enforce_turtle(&store, ordinary).unwrap();
    store
        .reconcile_promoted_derivations(p, SOURCE, g, c, &BTreeSet::new(), TS)
        .unwrap();
    assert!(store.list_shapes().unwrap().is_empty());
    crate::vocabulary::enforce_turtle(&store, ordinary).unwrap();
}
