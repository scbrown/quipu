//! Backtest tests — the pre-creation hit list, and the honesty rules around it.
//!
//! History is built through REAL transacts, never by poking rows: the backtest
//! claims to replay what the write gate would have seen, so the fixture must be
//! what the write path actually records.

use super::*;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const TS: &str = "2026-01-01T00:00:00Z";
const DOC_TYPE: &str = "http://ex/Doc";
const NOTE_TYPE: &str = "http://ex/Note";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";

fn datum(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn typed(store: &Store, s: &str, type_iri: &str) -> Datum {
    let v = Value::Ref(store.intern(type_iri).unwrap());
    datum(store, s, crate::namespace::RDF_TYPE, v)
}

fn candidate() -> Candidate {
    Candidate {
        policy_iri: "http://ex/candidate".into(),
        target_type_iri: Some(DOC_TYPE.into()),
        claim: Some(REQUIRE_LABEL.into()),
    }
}

/// tx1: doc1 lands WITHOUT a label (the exemplar-shaped offence).
/// tx2: doc1 gains its label (the fix).
/// tx3: doc2 lands WITH a label in the same write (compliant from birth).
/// tx4: a Note lands label-less (wrong type — out of the candidate's scope).
fn history(store: &mut Store) -> (i64, i64, i64, i64) {
    let d = vec![typed(store, "http://ex/doc1", DOC_TYPE)];
    let tx1 = store.transact(&d, TS, None, None).unwrap();
    let d = vec![datum(
        store,
        "http://ex/doc1",
        RDFS_LABEL,
        Value::Str("doc one".into()),
    )];
    let tx2 = store.transact(&d, TS, None, None).unwrap();
    let d = vec![
        typed(store, "http://ex/doc2", DOC_TYPE),
        datum(
            store,
            "http://ex/doc2",
            RDFS_LABEL,
            Value::Str("doc two".into()),
        ),
    ];
    let tx3 = store.transact(&d, TS, None, None).unwrap();
    let d = vec![typed(store, "http://ex/note1", NOTE_TYPE)];
    let tx4 = store.transact(&d, TS, None, None).unwrap();
    (tx1, tx2, tx3, tx4)
}

#[test]
fn the_hit_list_identifies_what_when_and_on_which_target() {
    let mut store = Store::open_in_memory().unwrap();
    let (tx1, tx2, tx3, tx4) = history(&mut store);
    let window = Window {
        from_tx: tx1,
        to_tx: tx4,
    };
    let report = backtest(&store, &candidate(), &window).unwrap();

    assert!(report.unevaluable.is_none(), "{:?}", report.unevaluable);
    // Exactly one firing: doc1 at its label-less birth. tx2 re-evaluates doc1
    // (touched again) and finds it compliant; doc2 is compliant from birth;
    // the Note is never evaluated at all.
    assert_eq!(
        report.hits,
        vec![Hit {
            tx: tx1,
            timestamp: TS.into(),
            target_iri: "http://ex/doc1".into(),
        }],
        "one hit, naming the write, the time and the target"
    );
    // The evaluated population is the gate's: (tx1,doc1), (tx2,doc1), (tx3,doc2).
    assert_eq!(report.evaluations, 3, "{report:?}");
    assert_eq!(report.transactions, 4);
    let _ = (tx2, tx3);
    assert!(
        report.summary().contains("would have fired 1 time(s)"),
        "{}",
        report.summary()
    );
    assert!(report.hits[0].line().contains("http://ex/doc1"));
}

#[test]
fn evaluation_is_as_of_the_transaction_not_of_the_present() {
    // doc1 is compliant NOW (tx2 fixed it). A backtest of the window that only
    // covers tx1 must still fire — the claim is asked against the graph as it
    // stood, which is the difference between a retrospective evaluation and a
    // present-state check wearing one's name.
    let mut store = Store::open_in_memory().unwrap();
    let (tx1, ..) = history(&mut store);
    let window = Window {
        from_tx: tx1,
        to_tx: tx1,
    };
    let report = backtest(&store, &candidate(), &window).unwrap();
    assert_eq!(report.hits.len(), 1, "{report:?}");
    assert_eq!(report.hits[0].target_iri, "http://ex/doc1");
}

#[test]
fn zero_hits_is_a_measured_answer() {
    // The green side of the honesty rule: a window where every touched target
    // was compliant reports 0 hits WITH the evaluations that back the number,
    // and is not unevaluable.
    let mut store = Store::open_in_memory().unwrap();
    let (_, tx2, tx3, _) = history(&mut store);
    let window = Window {
        from_tx: tx2,
        to_tx: tx3,
    };
    let report = backtest(&store, &candidate(), &window).unwrap();
    assert!(report.unevaluable.is_none());
    assert!(report.hits.is_empty());
    assert!(report.evaluations > 0, "zero hits must rest on evaluations");
    assert!(report.summary().contains("fired 0 time(s)"));
    assert!(!report.summary().contains("CANNOT EVALUATE"));
}

#[test]
fn a_claimless_candidate_is_unevaluable_never_silently_clean() {
    let mut store = Store::open_in_memory().unwrap();
    let (tx1, _, _, tx4) = history(&mut store);
    let window = Window {
        from_tx: tx1,
        to_tx: tx4,
    };
    let report = backtest(
        &store,
        &Candidate {
            claim: None,
            ..candidate()
        },
        &window,
    )
    .unwrap();
    let why = report.unevaluable.as_deref().expect("must be unevaluable");
    assert!(why.contains("no SPARQL aegis:claim"), "{why}");
    assert!(report.hits.is_empty());
    let summary = report.summary();
    assert!(
        summary.contains("CANNOT EVALUATE") && summary.contains("not \"0 hits\""),
        "unevaluable must be loud and distinct from zero: {summary}"
    );
    assert!(!summary.contains("would have fired"));
}

#[test]
fn a_targetless_claim_is_unevaluable() {
    let mut store = Store::open_in_memory().unwrap();
    let (tx1, _, _, tx4) = history(&mut store);
    let report = backtest(
        &store,
        &Candidate {
            claim: Some("ASK { ?s ?p ?o }".into()),
            ..candidate()
        },
        &Window {
            from_tx: tx1,
            to_tx: tx4,
        },
    )
    .unwrap();
    assert!(
        report
            .unevaluable
            .as_deref()
            .is_some_and(|w| w.contains("$target")),
        "{report:?}"
    );
}

#[test]
fn a_malformed_claim_is_unevaluable_with_the_parse_reason() {
    // Failing to evaluate must come back as "cannot measure, here is why" —
    // an Err would read as the BACKTEST being broken, and a silent skip would
    // read as clean.
    let mut store = Store::open_in_memory().unwrap();
    let (tx1, _, _, tx4) = history(&mut store);
    let report = backtest(
        &store,
        &Candidate {
            claim: Some("ASK { $target THIS IS NOT SPARQL".into()),
            ..candidate()
        },
        &Window {
            from_tx: tx1,
            to_tx: tx4,
        },
    )
    .unwrap();
    assert!(
        report
            .unevaluable
            .as_deref()
            .is_some_and(|w| w.contains("failed to evaluate")),
        "{report:?}"
    );
    assert!(
        report.hits.is_empty(),
        "no partial hit list under a failed evaluation"
    );
}

#[test]
fn window_last_clamps_to_recorded_history() {
    let mut store = Store::open_in_memory().unwrap();
    let (_, _, _, tx4) = history(&mut store);
    let w = Window::last(&store, 2).unwrap();
    assert_eq!((w.from_tx, w.to_tx), (tx4 - 1, tx4));
    let all = Window::last(&store, 10_000).unwrap();
    assert_eq!(
        (all.from_tx, all.to_tx),
        (1, tx4),
        "asking for more than exists means all of it, not tx 0"
    );
}

#[test]
fn a_candidate_reads_out_of_draft_turtle_and_backtests_end_to_end() {
    // The whole quipu-side gesture composed: scaffold → parse → backtest,
    // with the store untouched by the candidate throughout (pre-creation is
    // the point).
    let mut store = Store::open_in_memory().unwrap();
    let (tx1, _, _, tx4) = history(&mut store);
    let turtle = super::super::draft::draft_turtle(&super::super::draft::DraftIntent {
        name: "no-unlabeled-docs".into(),
        label: "never land a Doc without a label again".into(),
        exemplar: "http://ex/verdict_1".into(),
        target_type_iri: DOC_TYPE.into(),
        claim: REQUIRE_LABEL.into(),
        class: None,
        point: None,
        layer: None,
        authority: None,
    })
    .unwrap();
    let cand = Candidate::from_turtle(&turtle).unwrap();
    assert_eq!(cand.target_type_iri.as_deref(), Some(DOC_TYPE));
    assert_eq!(cand.claim.as_deref(), Some(REQUIRE_LABEL));

    let before = store.latest_tx_id().unwrap();
    let report = backtest(
        &store,
        &cand,
        &Window {
            from_tx: tx1,
            to_tx: tx4,
        },
    )
    .unwrap();
    assert_eq!(report.hits.len(), 1);
    assert_eq!(
        store.latest_tx_id().unwrap(),
        before,
        "backtesting must write nothing — the candidate does not exist yet"
    );
}

#[test]
fn turtle_with_zero_or_two_policies_is_refused() {
    assert!(Candidate::from_turtle("@prefix ex: <http://ex/> .").is_err());
    let two = "@prefix aegis: <http://aegis.gastown.local/ontology/> .\n\
               <http://ex/p1> a aegis:Policy .\n\
               <http://ex/p2> a aegis:Policy .";
    let err = Candidate::from_turtle(two);
    assert!(
        err.is_err(),
        "two policies in one candidate file must be refused: {err:?}"
    );
}
