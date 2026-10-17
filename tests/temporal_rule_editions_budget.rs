//! Historical derived-output regression from the independent source review.
use quipu::{
    reasoner::{
        historical::{HistoricalError, evaluate_temporal_view},
        parse_rules,
    },
    sparql::TemporalContext,
    store::{Datum, Store},
    types::{Op, Value},
};
#[test]
fn same_reviewer_derived_amplification_probe() {
    let mut store = Store::open_in_memory().unwrap();
    let pred = store.intern("https://example.org/edge").unwrap();
    let hub = store.intern("https://example.org/hub").unwrap();
    let mut rows = Vec::new();
    for n in 0..8 {
        let left = store
            .intern(&format!("https://example.org/left{n}"))
            .unwrap();
        let right = store
            .intern(&format!("https://example.org/right{n}"))
            .unwrap();
        rows.push(Datum {
            entity: left,
            attribute: pred,
            value: Value::Ref(hub),
            valid_from: "2000-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
        rows.push(Datum {
            entity: hub,
            attribute: pred,
            value: Value::Ref(right),
            valid_from: "2000-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    let tx = store
        .transact(&rows, "2000-01-01T00:00:00Z", None, None)
        .unwrap();
    let rules=parse_rules(r#"@prefix rule: <http://quipu.local/rule#> . <https://example.org/r> a rule:Rule ; rule:id "twohop" ; rule:prefix "https://example.org/" ; rule:head "reach(?x, ?z)" ; rule:body "edge(?x, ?y), edge(?y, ?z)" ."#,None).unwrap();
    let ctx = TemporalContext {
        valid_at: Some("2021-01-01T00:00:00Z".into()),
        as_of_tx: Some(tx),
        ..TemporalContext::default()
    };
    store.search_config_mut().max_sparql_rows = 100;
    let control = evaluate_temporal_view(&store, &rules, &ctx).unwrap();
    assert_eq!(control.len(), 64);
    println!("POSITIVE:16premises->64derived at cap100");
    store.search_config_mut().max_sparql_rows = 16;
    assert!(matches!(
        evaluate_temporal_view(&store, &rules, &ctx),
        Err(HistoricalError::Capacity)
    ));
    println!("NEGATIVE:premise sentinel16 refuses");
    store.search_config_mut().max_sparql_rows = 17;
    match evaluate_temporal_view(&store, &rules, &ctx) {
        Ok(facts) => {
            println!(
                "FALSIFIER:cap17 returns{}derived, expected Capacity",
                facts.len()
            );
            assert!(facts.len() < 17, "derived output exceeded caller ceiling");
        }
        Err(HistoricalError::Capacity) => println!("PASS:derived amplification refused"),
        Err(e) => panic!("unexpected refusal:{e}"),
    }
}

fn private_store(edges: &[(&str, &str, &str)]) -> (Store, TemporalContext) {
    let mut store = Store::open_in_memory().unwrap();
    let rows: Vec<_> = edges
        .iter()
        .map(|(s, p, o)| Datum {
            entity: store.intern(&format!("https://example.org/{s}")).unwrap(),
            attribute: store.intern(&format!("https://example.org/{p}")).unwrap(),
            value: Value::Ref(store.intern(&format!("https://example.org/{o}")).unwrap()),
            valid_from: "2000-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    let tx = store
        .transact(&rows, "2000-01-01T00:00:00Z", None, None)
        .unwrap();
    (
        store,
        TemporalContext {
            valid_at: Some("2021-01-01T00:00:00Z".into()),
            as_of_tx: Some(tx),
            ..TemporalContext::default()
        },
    )
}

fn parse(body: &str) -> quipu::reasoner::RuleSet {
    parse_rules(
        &format!("@prefix rule: <http://quipu.local/rule#> . {body}"),
        None,
    )
    .unwrap()
}

#[test]
fn bounded_closure_preserves_recursion_negation_constants_and_source() {
    let (mut source, ctx) = private_store(&[
        ("a", "edge", "b"),
        ("b", "edge", "c"),
        ("c", "edge", "d"),
        ("a", "blocked", "d"),
        ("z", "reach", "z"),
    ]);
    let rules = parse(
        r#"
      <https://example.org/base> a rule:Rule ; rule:id "base" ; rule:prefix "https://example.org/" ; rule:head "reach(?x, ?y)" ; rule:body "edge(?x, ?y)" .
      <https://example.org/trans> a rule:Rule ; rule:id "trans" ; rule:prefix "https://example.org/" ; rule:head "reach(?x, ?z)" ; rule:body "reach(?x, ?y), edge(?y, ?z)" .
      <https://example.org/allowed> a rule:Rule ; rule:id "allowed" ; rule:prefix "https://example.org/" ; rule:head "allowed(?x, ?z)" ; rule:body "reach(?x, ?z), not blocked(?x, ?z)" .
      <https://example.org/self> a rule:Rule ; rule:id "self" ; rule:prefix "https://example.org/" ; rule:head "self(?x, <https://example.org/z>)" ; rule:body "reach(?x, ?x)" .
    "#,
    );
    let head = source.latest_tx_id().unwrap();
    let actual = evaluate_temporal_view(&source, &rules, &ctx).unwrap();
    assert_eq!(source.latest_tx_id().unwrap(), head);
    assert_eq!(
        evaluate_temporal_view(&source, &rules, &ctx).unwrap(),
        actual
    );
    assert_eq!(
        actual
            .iter()
            .filter(|f| f.predicate.ends_with("/reach"))
            .count(),
        6
    );
    assert_eq!(
        actual
            .iter()
            .filter(|f| f.predicate.ends_with("/allowed"))
            .count(),
        6
    );
    assert!(actual.iter().any(|f| f.predicate.ends_with("/self")
        && f.subject.ends_with("/z")
        && f.object.ends_with("/z")));
    // Compare the complete head projection with the established legacy engine.
    quipu::reasoner::evaluate(&mut source, &rules, "2021-01-01T00:00:00Z").unwrap();
    let companion = source
        .lookup(&source.companion_inferred_iri(0).unwrap())
        .unwrap()
        .unwrap();
    let legacy: std::collections::BTreeSet<_> = source
        .current_facts_in_graph(companion)
        .unwrap()
        .into_iter()
        .filter(|f| {
            rules
                .rules
                .iter()
                .any(|r| r.head.predicate == source.resolve(f.attribute).unwrap())
        })
        .map(|f| {
            let Value::Ref(object) = f.value else {
                panic!("IRI fixture")
            };
            quipu::reasoner::historical::HistoricalFact {
                subject: source.resolve(f.entity).unwrap(),
                predicate: source.resolve(f.attribute).unwrap(),
                object: source.resolve(object).unwrap(),
            }
        })
        .collect();
    assert_eq!(
        actual
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        legacy
    );
}

#[test]
fn bounded_work_refuses_duplicate_amplification_and_expired_deadline() {
    let (mut source, mut ctx) = private_store(&[
        ("a", "edge", "hub"),
        ("b", "edge", "hub"),
        ("hub", "edge", "c"),
        ("hub", "edge", "d"),
    ]);
    let rules = parse(
        r#"<https://example.org/r> a rule:Rule ; rule:id "r" ; rule:prefix "https://example.org/" ; rule:head "same(<https://example.org/a>, <https://example.org/c>)" ; rule:body "edge(?a, ?b), edge(?c, ?d), edge(?e, ?f), edge(?g, ?h)" ."#,
    );
    let head = source.latest_tx_id().unwrap();
    assert_eq!(
        evaluate_temporal_view(&source, &rules, &ctx).unwrap().len(),
        1
    );
    source.search_config_mut().max_join_rows = 100;
    assert!(matches!(
        evaluate_temporal_view(&source, &rules, &ctx),
        Err(HistoricalError::Capacity)
    ));
    source.search_config_mut().max_join_rows = 0;
    ctx.row_cap = Some(100);
    assert!(matches!(
        evaluate_temporal_view(&source, &rules, &ctx),
        Err(HistoricalError::Capacity)
    ));
    ctx.row_cap = None;
    ctx.deadline = Some(quipu::time::Deadline::after_millis(0));
    assert!(evaluate_temporal_view(&source, &rules, &ctx).is_err());
    assert_eq!(source.latest_tx_id().unwrap(), head);
}

#[test]
fn output_ceiling_applies_to_union_across_rules_and_exact_sentinel() {
    let (mut source, ctx) = private_store(&[
        ("a", "edge", "b"),
        ("b", "edge", "c"),
        ("c", "edge", "d"),
        ("d", "edge", "e"),
    ]);
    let rules = parse(
        r#"
      <https://example.org/r1> a rule:Rule ; rule:id "r1" ; rule:prefix "https://example.org/" ; rule:head "first(?x, ?y)" ; rule:body "edge(?x, ?y)" .
      <https://example.org/r2> a rule:Rule ; rule:id "r2" ; rule:prefix "https://example.org/" ; rule:head "second(?x, ?y)" ; rule:body "edge(?x, ?y)" .
    "#,
    );
    source.search_config_mut().max_sparql_rows = 9;
    assert_eq!(
        evaluate_temporal_view(&source, &rules, &ctx).unwrap().len(),
        8
    );
    source.search_config_mut().max_sparql_rows = 8;
    assert!(matches!(
        evaluate_temporal_view(&source, &rules, &ctx),
        Err(HistoricalError::Capacity)
    ));
    source.search_config_mut().max_sparql_rows = 7;
    assert!(matches!(
        evaluate_temporal_view(&source, &rules, &ctx),
        Err(HistoricalError::Capacity)
    ));
}
