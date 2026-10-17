//! Private-store controls for the historical evaluation seam. Catalogue and
//! CLI acceptance remain separate until the proposed schema is reviewed.

use quipu::reasoner::historical::{HistoricalError, evaluate_temporal_view};
use quipu::reasoner::{evaluate, parse_rules};
use quipu::sparql::{GraphScope, TemporalContext};
use quipu::store::{Datum, Store};
use quipu::types::{Op, Value};

const START: &str = "2000-01-01T00:00:00Z";
const CORRECTION: &str = "2020-01-01T00:00:00Z";
const SUBJECT: &str = "https://example.org/claim";
const PREDICATE: &str = "https://example.org/deadline";

fn rules() -> quipu::reasoner::RuleSet {
    parse_rules(
        r#"@prefix rule: <http://quipu.local/rule#> .
        <https://example.org/r> a rule:Rule ; rule:id "expiry" ;
          rule:prefix "https://example.org/" ;
          rule:head "expiry(?claim, ?date)" ;
          rule:body "deadline(?claim, ?date)" ."#,
        None,
    )
    .unwrap()
}

fn datum(store: &Store, object: &str, op: Op) -> Datum {
    Datum {
        entity: store.intern(SUBJECT).unwrap(),
        attribute: store.intern(PREDICATE).unwrap(),
        value: Value::Ref(store.intern(object).unwrap()),
        valid_from: START.into(),
        valid_to: None,
        op,
    }
}

fn context(tx: i64) -> TemporalContext {
    TemporalContext {
        valid_at: Some("2021-01-01T00:00:00Z".into()),
        as_of_tx: Some(tx),
        ..TemporalContext::default()
    }
}

#[test]
fn temporal_rule_editions_refuses_smaller_configured_output_ceiling() {
    let mut store = Store::open_in_memory().unwrap();
    let first = datum(&store, "https://example.org/deadline-old", Op::Assert);
    let tx = store.transact(&[first], START, None, None).unwrap();
    assert_eq!(
        evaluate_temporal_view(&store, &rules(), &context(tx))
            .unwrap()
            .len(),
        1
    );
    store.search_config_mut().max_sparql_rows = 1;
    assert!(
        matches!(
            evaluate_temporal_view(&store, &rules(), &context(tx)),
            Err(HistoricalError::Capacity)
        ),
        "a smaller configured output ceiling must refuse at its sentinel, never be bypassed"
    );
}

#[test]
fn temporal_rule_editions_isolated_premise_snapshots() {
    let mut store = Store::open_in_memory().unwrap();
    let old = "https://example.org/deadline-old";
    let new = "https://example.org/deadline-corrected";
    let first = datum(&store, old, Op::Assert);
    store.transact(&[first], START, None, None).unwrap();
    let before = store.latest_tx_id().unwrap();
    let mut revised = datum(&store, new, Op::Assert);
    revised.valid_from = CORRECTION.into();
    let correction = [datum(&store, old, Op::Retract), revised];
    store.transact(&correction, CORRECTION, None, None).unwrap();
    let after = store.latest_tx_id().unwrap();
    // A populated CURRENT companion is the contamination control.
    evaluate(&mut store, &rules(), CORRECTION).unwrap();
    let companion = store
        .lookup(&store.companion_inferred_iri(0).unwrap())
        .unwrap()
        .unwrap();
    let head = store.latest_tx_id().unwrap();
    let current = store.current_facts_in_graph(companion).unwrap();
    assert!(!current.is_empty());
    let a = evaluate_temporal_view(&store, &rules(), &context(before)).unwrap();
    let b = evaluate_temporal_view(&store, &rules(), &context(after)).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(a[0].subject, SUBJECT);
    assert_eq!(a[0].predicate, "https://example.org/expiry");
    assert_eq!(a[0].object, old);
    assert_eq!(b[0].object, new);
    // Retraction at 2020 preserves the old fact's 2010 validity; this is
    // not a retrospective replacement of that earlier interval.
    let mut earlier = context(after);
    earlier.valid_at = Some("2010-01-01T00:00:00Z".into());
    assert_eq!(
        evaluate_temporal_view(&store, &rules(), &earlier).unwrap()[0].object,
        old
    );
    assert_eq!(
        evaluate_temporal_view(&store, &rules(), &context(before)).unwrap(),
        a
    );
    assert_eq!(store.latest_tx_id().unwrap(), head);
    assert_eq!(store.current_facts_in_graph(companion).unwrap(), current);
    let mut inferred_scope = context(after);
    inferred_scope.graph = GraphScope::Named(vec![companion]);
    assert!(matches!(
        evaluate_temporal_view(&store, &rules(), &inferred_scope),
        Err(HistoricalError::UnsupportedScope)
    ));
}

#[test]
fn temporal_rule_editions_refuses_unpinned_or_unsupported_views() {
    let mut store = Store::open_in_memory().unwrap();
    let fact = datum(&store, "https://example.org/date", Op::Assert);
    store.transact(&[fact], START, None, None).unwrap();
    let tx = store.latest_tx_id().unwrap();
    assert!(matches!(
        evaluate_temporal_view(&store, &rules(), &TemporalContext::default()),
        Err(HistoricalError::MissingPin)
    ));
    assert!(matches!(
        evaluate_temporal_view(&store, &rules(), &context(tx + 100)),
        Err(HistoricalError::UnknownSnapshot(_))
    ));
    for date in [
        "not-a-date",
        "2010-02-30T00:00:00Z",
        "2010-01-01T00:00:00.5Z",
    ] {
        let mut ctx = context(tx);
        ctx.valid_at = Some(date.into());
        assert!(matches!(
            evaluate_temporal_view(&store, &rules(), &ctx),
            Err(HistoricalError::InvalidInstant(_))
        ));
    }
    let mut ctx = context(tx);
    ctx.graph = GraphScope::Default(vec![]);
    assert!(matches!(
        evaluate_temporal_view(&store, &rules(), &ctx),
        Err(HistoricalError::UnsupportedScope)
    ));
    let mut literal = datum(&store, "https://example.org/unused", Op::Assert);
    literal.value = Value::Str("unsupported literal".into());
    store.transact(&[literal], CORRECTION, None, None).unwrap();
    assert!(matches!(
        evaluate_temporal_view(&store, &rules(), &context(store.latest_tx_id().unwrap())),
        Err(HistoricalError::LiteralPremise)
    ));
}

#[test]
fn temporal_rule_editions_preserves_explicit_finite_validity() {
    let mut store = Store::open_in_memory().unwrap();
    let mut fact = datum(&store, "https://example.org/finite-deadline", Op::Assert);
    fact.valid_to = Some("2015-01-01T00:00:00Z".into());
    store.transact(&[fact], START, None, None).unwrap();
    let tx = store.latest_tx_id().unwrap();
    // An explicitly finite interval was already finite at this transaction;
    // it must not be mistaken for an end introduced by a future retraction.
    assert!(
        evaluate_temporal_view(&store, &rules(), &context(tx))
            .unwrap()
            .is_empty()
    );
    let mut within = context(tx);
    within.valid_at = Some("2010-01-01T00:00:00Z".into());
    let answer = evaluate_temporal_view(&store, &rules(), &within).unwrap();
    assert_eq!(answer.len(), 1);
    assert_eq!(answer[0].object, "https://example.org/finite-deadline");
    within.result_limit = Some(1);
    assert!(matches!(
        evaluate_temporal_view(&store, &rules(), &within),
        Err(HistoricalError::Capacity)
    ));
}

#[test]
fn temporal_rule_editions_concurrent_readers_keep_contexts_separate() {
    use sha2::{Digest, Sha256};

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private.db");
    let before;
    let after;
    {
        let mut writer = Store::open(path.to_str().unwrap()).unwrap();
        let first = datum(&writer, "https://example.org/old", Op::Assert);
        writer.transact(&[first], START, None, None).unwrap();
        before = writer.latest_tx_id().unwrap();
        let mut second = datum(&writer, "https://example.org/new", Op::Assert);
        second.valid_from = CORRECTION.into();
        let change = [
            datum(&writer, "https://example.org/old", Op::Retract),
            second,
        ];
        writer.transact(&change, CORRECTION, None, None).unwrap();
        after = writer.latest_tx_id().unwrap();
    }
    let digest = Sha256::digest(std::fs::read(&path).unwrap());
    let readers: Vec<_> = [before, after, before, after]
        .into_iter()
        .map(|tx| {
            let path = path.clone();
            std::thread::spawn(move || {
                // Owned fixture: no writer policy is replaced with defaults.
                let reader = Store::open_read_only(path.to_str().unwrap()).unwrap();
                let result = evaluate_temporal_view(&reader, &rules(), &context(tx)).unwrap();
                assert_eq!(result.len(), 1);
                (tx, result[0].object.clone())
            })
        })
        .collect();
    for reader in readers {
        let (tx, object) = reader.join().unwrap();
        assert_eq!(
            object,
            if tx == before {
                "https://example.org/old"
            } else {
                "https://example.org/new"
            }
        );
    }
    assert_eq!(Sha256::digest(std::fs::read(&path).unwrap()), digest);
}
