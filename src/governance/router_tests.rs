//! Escalation-router tests. Size-exempt (`*tests.rs`).

use super::*;

const TS: &str = "2026-01-01T00:00:00Z";
const NOW: i64 = 1_700_000_000;
const POLICY: &str = "http://ex/P1";
const TARGET: &str = "http://ex/d1";

fn store_with_request(window: i64) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let datums = mint_request(&store, POLICY, TARGET, None, window, NOW, TS).unwrap();
    store.transact(&datums, TS, None, None).unwrap();
    store
}

fn decide(store: &mut Store, outcome: &str, by: &str, hash: &str) {
    let iri = format!("http://ex/decision_{outcome}_{by}");
    let class = Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Decision")).unwrap());
    let d = |store: &Store, p: &str, v: Value| Datum {
        entity: store.intern(&iri).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    let datums = vec![
        d(store, RDF_TYPE, class),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}outcome"),
            Value::Str(outcome.into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}by"),
            Value::Str(by.into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}evidenceHash"),
            Value::Str(hash.into()),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();
}

#[test]
fn no_request_means_nothing_has_escalated_yet() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(resolve(&store, POLICY, TARGET, NOW).unwrap(), None);
}

#[test]
fn an_open_request_is_pending_and_names_what_would_resolve_it() {
    // The whole point of the router: the refusal becomes actionable. Before it,
    // require-approval failed closed with no channel and no bound — a refusal an
    // operator could not act on, which looks like governance and functions as an
    // outage.
    let store = store_with_request(600);
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert_eq!(
        ruling,
        Ruling::Pending {
            expires_at: NOW + 600
        }
    );
    assert!(!ruling.permits());

    let why = ruling.reason(POLICY, TARGET);
    assert!(why.contains("aegis:Decision"), "names the remedy: {why}");
    assert!(why.contains("approve"), "and the outcome needed: {why}");
    assert!(
        why.contains("DENIED"),
        "and what happens if nobody acts: {why}"
    );
}

#[test]
fn an_approval_bound_to_the_evidence_permits_the_write() {
    let mut store = store_with_request(600);
    decide(
        &mut store,
        "approve",
        "stiwi",
        &evidence_hash(POLICY, TARGET),
    );
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert_eq!(ruling, Ruling::Approved { by: "stiwi".into() });
    assert!(ruling.permits());
}

#[test]
fn an_approval_over_different_evidence_does_not_apply() {
    // Content binding, and the approve-then-sneak-in-changes defence. A decision
    // about other evidence is a decision about something else.
    let mut store = store_with_request(600);
    decide(&mut store, "approve", "stiwi", "sha256:something-else");
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(!ruling.permits(), "got {ruling:?}");
}

#[test]
fn a_rejection_is_an_answer_distinct_from_pending() {
    let mut store = store_with_request(600);
    decide(
        &mut store,
        "reject",
        "stiwi",
        &evidence_hash(POLICY, TARGET),
    );
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(matches!(ruling, Ruling::Rejected { .. }));
    assert!(!ruling.permits());
    let why = ruling.reason(POLICY, TARGET);
    assert!(why.contains("stiwi"), "names who refused: {why}");
}

#[test]
fn a_rejection_outranks_an_approval_when_both_exist() {
    // Two humans disagreeing is not a state to resolve by row order, and the
    // safe reading of a disagreement about whether to permit something is "no".
    let mut store = store_with_request(600);
    let hash = evidence_hash(POLICY, TARGET);
    decide(&mut store, "approve", "alice", &hash);
    decide(&mut store, "reject", "bob", &hash);
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(!ruling.permits(), "got {ruling:?}");
}

#[test]
fn an_unserviced_request_expires_into_a_denial() {
    // SARC I4 and §5.3: past the reversibility window the absence of a ruling
    // IS an answer, and the declared answer is deny. A request that quietly
    // stayed open would be the deferred autonomy the invariant forbids.
    let store = store_with_request(600);
    let ruling = resolve(&store, POLICY, TARGET, NOW + 601).unwrap().unwrap();
    assert_eq!(ruling, Ruling::Expired);
    assert!(!ruling.permits());
    let why = ruling.reason(POLICY, TARGET);
    assert!(
        why.contains("default-deny") && why.contains("not a timeout to retry through"),
        "the reason must not read as a transient failure: {why}"
    );
}

#[test]
fn expiry_is_exact_at_the_boundary() {
    let store = store_with_request(600);
    assert!(matches!(
        resolve(&store, POLICY, TARGET, NOW + 599).unwrap().unwrap(),
        Ruling::Pending { .. }
    ));
    // At the window, not after it: the window is the time within which the
    // action can still be undone, so its last instant is already too late.
    assert_eq!(
        resolve(&store, POLICY, TARGET, NOW + 600).unwrap().unwrap(),
        Ruling::Expired
    );
}

#[test]
fn a_retry_updates_the_same_request_rather_than_accumulating_one_per_attempt() {
    // The IRI is derived from the evidence, so an agent that retries every few
    // seconds does not bury the operator's queue under identical requests.
    let mut store = store_with_request(600);
    let again = mint_request(&store, POLICY, TARGET, None, 600, NOW + 10, TS).unwrap();
    store.transact(&again, TS, None, None).unwrap();

    let q = format!("PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?r WHERE {{ ?r a a:DecisionRequest }}");
    let QueryResult::Select { rows, .. } = sparql::query(&store, &q).unwrap() else {
        panic!("select")
    };
    assert_eq!(rows.len(), 1, "one request, not one per attempt");
}

#[test]
fn only_an_approval_permits() {
    // The invariant that keeps Pending and Expired from reading as passes —
    // the default-allow-under-load failure the design exists to exclude.
    assert!(Ruling::Approved { by: "a".into() }.permits());
    assert!(!Ruling::Pending { expires_at: 0 }.permits());
    assert!(!Ruling::Expired.permits());
    assert!(
        !Ruling::Rejected {
            by: "a".into(),
            outcome: "changes".into()
        }
        .permits()
    );
}

#[test]
fn the_evidence_hash_separates_targets_under_one_policy() {
    assert_ne!(
        evidence_hash(POLICY, "http://ex/a"),
        evidence_hash(POLICY, "http://ex/b")
    );
    assert_eq!(evidence_hash(POLICY, TARGET), evidence_hash(POLICY, TARGET));
}

// ── Liveness: the router fires through the REAL write gate ───────────────────

const DOC_TYPE: &str = "http://ex/Doc";
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";

/// Define an escalating policy over `DOC_TYPE` with a reversibility window.
fn define_escalating(store: &mut Store, iri: &str, window: i64) {
    let class = Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Policy")).unwrap());
    let d = |store: &Store, p: &str, v: Value| Datum {
        entity: store.intern(iri).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    let datums = vec![
        d(store, RDF_TYPE, class),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}targets"),
            Value::Str(DOC_TYPE.into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}claim"),
            Value::Str(REQUIRE_LABEL.into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}boundary"),
            Value::Str("action".into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}effect"),
            Value::Str("require-approval".into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}reversibilityWindowSeconds"),
            Value::Int(window),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();
}

fn noncompliant_doc(store: &Store, iri: &str) -> Vec<Datum> {
    vec![Datum {
        entity: store.intern(iri).unwrap(),
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern(DOC_TYPE).unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }]
}

fn open_requests(store: &Store) -> usize {
    let q = format!("PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?r WHERE {{ ?r a a:DecisionRequest }}");
    match sparql::query(store, &q).unwrap() {
        QueryResult::Select { rows, .. } => rows.len(),
        _ => 0,
    }
}

#[test]
fn a_refused_write_opens_a_request_that_survives_its_own_rollback() {
    // THE case. The refusal rolls the savepoint back, and a request written in
    // place would go with it — leaving an operator a refusal with nothing to act
    // on, which is the state the router exists to end.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_escalating(&mut store, "http://ex/PE", 600);

    let err = store.transact(&noncompliant_doc(&store, "http://ex/d9"), TS, None, None);
    assert!(matches!(err, Err(crate::error::Error::PolicyDenied(_))));

    // The write is gone...
    let gone = sparql::query(&store, "ASK { <http://ex/d9> ?p ?o }").unwrap();
    assert!(matches!(gone, QueryResult::Ask(false)));
    // ...and the request survived the rollback that refusal caused.
    assert_eq!(open_requests(&store), 1);
}

#[test]
fn an_approval_lets_the_next_attempt_through() {
    // The channel `require-approval` never had. Before the router this effect
    // was a permanent refusal: no record said what would un-refuse it.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_escalating(&mut store, "http://ex/PE", 600);

    // First attempt opens the request and is refused.
    let _ = store.transact(&noncompliant_doc(&store, "http://ex/d9"), TS, None, None);
    assert_eq!(open_requests(&store), 1);

    // A human rules on it, bound to the same evidence.
    let hash = evidence_hash("http://ex/PE", "http://ex/d9");
    decide(&mut store, "approve", "stiwi", &hash);

    // The retry succeeds — the same write, now permitted.
    store
        .transact(&noncompliant_doc(&store, "http://ex/d9"), TS, None, None)
        .expect("an approved escalation lets the write through");
    let landed = sparql::query(&store, "ASK { <http://ex/d9> ?p ?o }").unwrap();
    assert!(matches!(landed, QueryResult::Ask(true)));
}

#[test]
fn a_zero_window_expires_immediately_rather_than_inventing_a_bound() {
    // The placement check requires a reversibility window on an escalation at
    // definition time, so reaching here without one means that check was off.
    // Treating it as already expired refuses; inventing a default would be
    // inventing the bound I4 requires be declared.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_escalating(&mut store, "http://ex/PE", 0);

    let _ = store.transact(&noncompliant_doc(&store, "http://ex/d9"), TS, None, None);
    // Second attempt sees an already-expired request: denied, not pending.
    let err = store.transact(&noncompliant_doc(&store, "http://ex/d9"), TS, None, None);
    let Err(crate::error::Error::PolicyDenied(why)) = err else {
        panic!("expected denial");
    };
    assert!(why.contains("default-deny"), "{why}");
}
