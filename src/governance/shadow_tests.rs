//! Shadow gate tests. Size-exempt (`*tests.rs`).
//!
//! The identity arm (candidate == governing set gives 0 diffs) is a CONTROL
//! only: both sides run the same evaluator, so it holds by construction. The
//! diff machinery is proven by SABOTAGE instead (sattler, aegis-xfuch4.2):
//! a. a stricter candidate yields exactly the expected NEW REFUSAL rows;
//! b. a looser candidate yields exactly the expected WOULD-NOW-ADMIT rows;
//! c. a mutated evaluator surfaces as verdict mismatches, exactly on its rule;
//! d. an edited recorded verdict surfaces as a mismatch.

use super::*;
use crate::governance::guard::sabotage;
use crate::namespace::DEFAULT_BASE_NS;

const TS: &str = "2026-01-01T00:00:00Z";
const DOC: &str = "http://ex/Doc";
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const COLOR: &str = "http://ex/color";
const P_LABEL: &str = "http://ex/P_label";
const P_COLOR: &str = "http://ex/P_color";
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";
const REQUIRE_COLOR: &str = "ASK { $target <http://ex/color> ?c }";
const ALWAYS: &str = "ASK { $target ?p ?o }";

fn signed_store(enforce: bool) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = enforce;
    let dir = tempfile::tempdir().unwrap();
    let identity =
        crate::signing::SigningIdentity::load(&dir.path().join("k.pk8"), "quipu").unwrap();
    store.set_signing_identity(std::sync::Arc::new(identity));
    store
}

fn a(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn define(store: &mut Store, iri: &str, claim: &str) -> i64 {
    let d = vec![
        a(
            store,
            iri,
            RDF_TYPE,
            Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Policy")).unwrap()),
        ),
        a(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}targets"),
            Value::Str(DOC.into()),
        ),
        a(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}claim"),
            Value::Str(claim.into()),
        ),
        a(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}boundary"),
            Value::Str("action".into()),
        ),
        a(
            store,
            iri,
            &format!("{DEFAULT_BASE_NS}effect"),
            Value::Str("deny".into()),
        ),
    ];
    store.transact(&d, TS, Some("admin"), None).unwrap()
}

/// Write one Doc; returns its tx id, or `None` when the gate refused it.
fn doc(store: &mut Store, name: &str, label: bool, color: bool, actor: &str) -> Option<i64> {
    let e = format!("http://ex/{name}");
    let mut d = vec![a(
        store,
        &e,
        RDF_TYPE,
        Value::Ref(store.intern(DOC).unwrap()),
    )];
    if label {
        d.push(a(store, &e, LABEL, Value::Str("l".into())));
    }
    if color {
        d.push(a(store, &e, COLOR, Value::Str("red".into())));
    }
    store.transact(&d, TS, Some(actor), None).ok()
}

fn candidate(policies: &[(&str, &str)]) -> Candidate {
    let mut ttl = String::from("@prefix a: <http://aegis.gastown.local/ontology/> .\n");
    for (iri, claim) in policies {
        ttl.push_str(&format!(
            "<{iri}> a a:Policy ; a:targets \"{DOC}\" ; a:claim \"{claim}\" ; \
             a:boundary \"action\" ; a:effect \"deny\" .\n"
        ));
    }
    Candidate::from_turtle(&ttl).unwrap()
}

fn all(store: &Store, mode: Mode) -> Options {
    Options {
        window: Window {
            from_tx: 1,
            to_tx: store.latest_tx_id().unwrap(),
        },
        max_txs: None,
        mode,
    }
}

/// Enforced history: a label policy, then compliant docs by two writers,
/// half of them also coloured. Returns (store, txs of uncoloured docs).
fn enforced_history() -> (Store, Vec<i64>) {
    let mut store = signed_store(true);
    define(&mut store, P_LABEL, REQUIRE_LABEL);
    let mut uncoloured = Vec::new();
    for i in 0..6 {
        let actor = if i % 2 == 0 { "alice" } else { "bob" };
        let coloured = i % 3 == 0;
        let tx = doc(&mut store, &format!("d{i}"), true, coloured, actor).unwrap();
        if !coloured {
            uncoloured.push(tx);
        }
    }
    (store, uncoloured)
}

#[test]
fn control_identity_candidate_yields_no_diffs_and_joins_every_verdict() {
    let (store, _) = enforced_history();
    let r = run(
        &store,
        &candidate(&[(P_LABEL, REQUIRE_LABEL)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    assert!(r.diffs.is_empty(), "{:?}", r.diffs);
    assert_eq!(r.baseline_refuses_committed, 0);
    // Six docs were gated, each recorded a satisfied verdict in the next tx.
    assert_eq!(r.verdict_join.judged, 6);
    assert_eq!(r.verdict_join.matched, 6, "{:?}", r.verdict_join);
    // The six verdict-recording transactions are bookkeeping, not judged.
    assert_eq!(r.bypass_skipped, 6);
}

#[test]
fn sabotage_a_stricter_candidate_yields_exactly_the_expected_new_refusals() {
    let (store, uncoloured) = enforced_history();
    let r = run(
        &store,
        &candidate(&[(P_COLOR, REQUIRE_COLOR)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    let got: Vec<i64> = r.diffs.iter().map(|d| d.tx).collect();
    assert_eq!(got, uncoloured);
    assert!(
        r.diffs
            .iter()
            .all(|d| d.kind == DiffKind::NewRefusal
                && d.refused_by.iter().all(|(p, _)| p == P_COLOR))
    );
    // Per (rule, writer): 4 uncoloured docs, split across alice and bob.
    let refusals: usize = r
        .candidate
        .iter()
        .filter(|((p, _), _)| p == P_COLOR)
        .map(|(_, s)| s.would_refuse)
        .sum();
    assert_eq!(refusals, 4);
    assert!(
        r.candidate
            .contains_key(&(P_COLOR.to_string(), "alice".to_string()))
    );
    assert!(
        r.candidate
            .contains_key(&(P_COLOR.to_string(), "bob".to_string()))
    );
}

#[test]
fn sabotage_b_looser_candidate_yields_exactly_the_expected_would_now_admit() {
    // Unenforced history: non-compliant docs committed. The label policy as it
    // stood would refuse them; a candidate that loosens it admits them.
    let mut store = signed_store(false);
    define(&mut store, P_LABEL, REQUIRE_LABEL);
    let mut unlabelled = Vec::new();
    for i in 0..5 {
        let labelled = i % 2 == 0;
        let tx = doc(&mut store, &format!("u{i}"), labelled, false, "carol").unwrap();
        if !labelled {
            unlabelled.push(tx);
        }
    }
    let r = run(
        &store,
        &candidate(&[(P_LABEL, ALWAYS)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    let got: Vec<i64> = r.diffs.iter().map(|d| d.tx).collect();
    assert_eq!(got, unlabelled);
    assert!(r.diffs.iter().all(|d| d.kind == DiffKind::WouldNowAdmit));
    assert_eq!(r.baseline_refuses_committed, unlabelled.len());
}

#[test]
fn sabotage_c_a_mutated_evaluator_surfaces_as_verdict_mismatches_on_its_rule() {
    let (store, _) = enforced_history();
    sabotage::invert(Some(P_LABEL));
    let r = run(
        &store,
        &candidate(&[(P_LABEL, REQUIRE_LABEL)]),
        &all(&store, Mode::Add),
    );
    sabotage::invert(None);
    let r = r.unwrap();
    // Every recorded verdict said satisfied; the mutated evaluator says not.
    assert_eq!(r.verdict_join.outcome_mismatch, 6, "{:?}", r.verdict_join);
    assert_eq!(r.verdict_join.matched, 0);
    // And the baseline now "refuses" committed history: the fidelity signal.
    assert_eq!(r.baseline_refuses_committed, 6);
}

#[test]
fn sabotage_d_an_edited_recorded_verdict_surfaces_as_a_mismatch() {
    let (mut store, _) = enforced_history();
    // Rewrite ONE verdict's outcome.
    let outcome = format!("{DEFAULT_BASE_NS}outcome");
    let target = format!("{DEFAULT_BASE_NS}targetRef");
    let q = format!("SELECT ?v WHERE {{ ?v <{target}> \"http://ex/d2\" }}");
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(&store, &q).unwrap()
    else {
        panic!("select")
    };
    let Some(Value::Ref(v)) = rows[0].get("v").cloned() else {
        panic!("verdict")
    };
    let viri = store.resolve(v).unwrap();
    store.governance_config_mut().enforce_on_write = false;
    let mut edit = vec![a(&store, &viri, &outcome, Value::Str("satisfied".into()))];
    edit[0].op = Op::Retract;
    edit.push(a(&store, &viri, &outcome, Value::Str("unsatisfied".into())));
    store.transact(&edit, TS, Some("mallory"), None).unwrap();

    let r = run(
        &store,
        &candidate(&[(P_LABEL, REQUIRE_LABEL)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    assert_eq!(r.verdict_join.outcome_mismatch, 1, "{:?}", r.verdict_join);
    assert_eq!(r.verdict_join.matched, 5);
}

#[test]
fn a_policy_change_is_a_reported_boundary_and_judges_later_txs_only() {
    let mut store = signed_store(false);
    let before = doc(&mut store, "early", false, false, "dave").unwrap();
    let boundary = define(&mut store, P_LABEL, REQUIRE_LABEL);
    let after = doc(&mut store, "late", false, false, "dave").unwrap();
    let r = run(
        &store,
        &candidate(&[(P_COLOR, ALWAYS)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    assert_eq!(r.policy_boundaries, vec![boundary]);
    // Only the post-boundary doc was judged by P_label.
    let label_evals: usize = r
        .baseline
        .iter()
        .filter(|((p, _), _)| p == P_LABEL)
        .map(|(_, s)| s.evaluations)
        .sum();
    assert_eq!(label_evals, 1);
    assert_eq!(r.baseline_refuses_committed, 1);
    let _ = (before, after);
}

#[test]
fn refusals_are_counted_as_not_replayable() {
    let (mut store, _) = enforced_history();
    assert!(doc(&mut store, "bad", false, false, "erin").is_none());
    let r = run(
        &store,
        &candidate(&[(P_LABEL, REQUIRE_LABEL)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    assert_eq!(r.refusals_not_replayable, 1);
}

#[test]
fn replace_mode_drops_the_governing_set() {
    let mut store = signed_store(false);
    define(&mut store, P_LABEL, REQUIRE_LABEL);
    let tx = doc(&mut store, "x", false, true, "fay").unwrap();
    // Replace with a colour rule the doc satisfies: the label refusal vanishes.
    let r = run(
        &store,
        &candidate(&[(P_COLOR, REQUIRE_COLOR)]),
        &all(&store, Mode::Replace),
    )
    .unwrap();
    assert_eq!(r.diffs.len(), 1);
    assert_eq!(r.diffs[0].tx, tx);
    assert_eq!(r.diffs[0].kind, DiffKind::WouldNowAdmit);
}

#[test]
fn max_txs_truncates_and_says_where() {
    let (store, _) = enforced_history();
    let mut opts = all(&store, Mode::Add);
    opts.max_txs = Some(2);
    let r = run(&store, &candidate(&[(P_COLOR, REQUIRE_COLOR)]), &opts).unwrap();
    assert_eq!(r.judged, 2);
    assert!(r.truncated_at.is_some());
}

#[test]
fn a_candidate_with_no_enforceable_policy_is_refused() {
    let err = Candidate::from_turtle("<http://ex/x> <http://ex/p> \"o\" .").unwrap_err();
    assert!(err.to_string().contains("Nothing to shadow"), "{err}");
}

#[test]
fn the_shadow_never_writes() {
    let (store, _) = enforced_history();
    let head = store.latest_tx_id().unwrap();
    let facts: i64 = store
        .prepare("SELECT COUNT(*) FROM facts")
        .unwrap()
        .query_row([], |r| r.get(0))
        .unwrap();
    run(
        &store,
        &candidate(&[(P_COLOR, REQUIRE_COLOR)]),
        &all(&store, Mode::Add),
    )
    .unwrap();
    assert_eq!(store.latest_tx_id().unwrap(), head);
    let after: i64 = store
        .prepare("SELECT COUNT(*) FROM facts")
        .unwrap()
        .query_row([], |r| r.get(0))
        .unwrap();
    assert_eq!(after, facts);
}

#[test]
fn durations_parse_and_garbage_is_refused() {
    assert_eq!(parse_duration("90m").unwrap(), 5_400);
    assert_eq!(parse_duration("24h").unwrap(), 86_400);
    assert_eq!(parse_duration("7d").unwrap(), 604_800);
    assert_eq!(parse_duration("30").unwrap(), 30);
    assert!(parse_duration("7w").is_err());
    assert!(parse_duration("").is_err());
}

#[test]
fn since_selects_recent_transactions_and_an_empty_window_is_empty() {
    let mut store = signed_store(false);
    let old = store
        .transact(
            &[a(&store, "http://ex/o", LABEL, Value::Str("x".into()))],
            "2000-01-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    let now = crate::time::now_iso();
    let recent = store
        .transact(
            &[a(&store, "http://ex/r", LABEL, Value::Str("x".into()))],
            &now,
            None,
            None,
        )
        .unwrap();
    let w = window_since(&store, 3_600).unwrap();
    assert_eq!((w.from_tx, w.to_tx), (recent, recent));
    assert!(old < w.from_tx);
    let empty = Store::open_in_memory().unwrap();
    let w = window_since(&empty, 3_600).unwrap();
    assert!(w.from_tx > w.to_tx, "no transactions -> empty range");
}
