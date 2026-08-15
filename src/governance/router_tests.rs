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

fn keypair() -> ring::signature::Ed25519KeyPair {
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

/// Register `by` as a decider for `policy` — the human-authored root-of-trust
/// fact a real deployment writes when it appoints an operator.
fn register_decider(store: &mut Store, by: &str, policy: &str, public_key_hex: &str) {
    let iri = format!(
        "http://ex/reg_{by}_{}",
        policy.replace(['/', ':', '#'], "_")
    );
    let class = Value::Ref(
        store
            .intern(&format!("{DEFAULT_BASE_NS}VerifierRegistration"))
            .unwrap(),
    );
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
            &format!("{DEFAULT_BASE_NS}verifier"),
            Value::Str(by.into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}attests"),
            Value::Str(policy.into()),
        ),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}publicKey"),
            Value::Str(public_key_hex.into()),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();
}

/// Write a decision fact, optionally carrying `signature`. No registration —
/// callers set that up (or deliberately don't).
fn write_decision(store: &mut Store, outcome: &str, by: &str, hash: &str, signature: Option<&str>) {
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
    let mut datums = vec![
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
    if let Some(sig) = signature {
        datums.push(d(
            store,
            &format!("{DEFAULT_BASE_NS}signature"),
            Value::Str(sig.into()),
        ));
    }
    store.transact(&datums, TS, None, None).unwrap();
}

/// A properly attested ruling: register `by` as a decider for `policy`, sign
/// the canonical decision message, and write the decision.
fn decide(store: &mut Store, outcome: &str, by: &str, hash: &str, policy: &str) {
    let kp = keypair();
    register_decider(store, by, policy, &crate::signing::public_key_hex(&kp));
    let sig = crate::signing::sign_hex(&kp, &decision_message(hash, outcome, by));
    write_decision(store, outcome, by, hash, Some(&sig));
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
        POLICY,
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
    decide(
        &mut store,
        "approve",
        "stiwi",
        "sha256:something-else",
        POLICY,
    );
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
        POLICY,
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
    decide(&mut store, "approve", "alice", &hash, POLICY);
    decide(&mut store, "reject", "bob", &hash, POLICY);
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

// ── Signed decisions (quipu-5s5) ─────────────────────────────────────────────
//
// Decisions are ordinary writable facts. Before the signature check, the agent
// whose write was refused could write its own `aegis:Decision "approve"` and
// walk through the gate. Every path below MUST fall through to Pending — a
// forged decision is not a wrong ruling, it is no ruling at all.

#[test]
fn an_unsigned_decision_is_not_a_ruling() {
    let mut store = store_with_request(600);
    let hash = evidence_hash(POLICY, TARGET);
    // The refused agent writes a bare approval — write access it has.
    write_decision(&mut store, "approve", "mallory", &hash, None);
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(
        matches!(ruling, Ruling::Pending { .. }),
        "an unsigned approval must not permit: {ruling:?}"
    );
}

#[test]
fn a_self_signed_decision_without_a_registration_is_ignored() {
    // Mallory can sign — with mallory's own key. Signing proves key
    // possession; the REGISTRATION is what makes it authority, and only a
    // human writes those.
    let mut store = store_with_request(600);
    let hash = evidence_hash(POLICY, TARGET);
    let kp = keypair();
    let sig = crate::signing::sign_hex(&kp, &decision_message(&hash, "approve", "mallory"));
    write_decision(&mut store, "approve", "mallory", &hash, Some(&sig));
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(
        !ruling.permits(),
        "an unregistered signer must not permit: {ruling:?}"
    );
}

#[test]
fn a_decision_signed_by_a_key_other_than_the_registered_one_is_ignored() {
    let mut store = store_with_request(600);
    let hash = evidence_hash(POLICY, TARGET);
    let registered = keypair();
    register_decider(
        &mut store,
        "stiwi",
        POLICY,
        &crate::signing::public_key_hex(&registered),
    );
    // ...but the decision is signed with a different key.
    let other = keypair();
    let sig = crate::signing::sign_hex(&other, &decision_message(&hash, "approve", "stiwi"));
    write_decision(&mut store, "approve", "stiwi", &hash, Some(&sig));
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(
        !ruling.permits(),
        "a wrong-key signature must not permit: {ruling:?}"
    );
}

#[test]
fn a_registration_for_a_different_policy_does_not_authorize() {
    // Authority is scoped: attesting policy A does not let a decider rule on
    // policy B, same as a verifier registration scopes to a predicate.
    let mut store = store_with_request(600);
    let hash = evidence_hash(POLICY, TARGET);
    let kp = keypair();
    register_decider(
        &mut store,
        "stiwi",
        "http://ex/some-other-policy",
        &crate::signing::public_key_hex(&kp),
    );
    let sig = crate::signing::sign_hex(&kp, &decision_message(&hash, "approve", "stiwi"));
    write_decision(&mut store, "approve", "stiwi", &hash, Some(&sig));
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(
        !ruling.permits(),
        "out-of-scope authority must not permit: {ruling:?}"
    );
}

#[test]
fn a_signature_over_a_different_outcome_is_ignored() {
    // The signature seals the outcome. Re-pointing a signed rejection at
    // "approve" must break the seal — the flip-the-outcome forgery.
    let mut store = store_with_request(600);
    let hash = evidence_hash(POLICY, TARGET);
    let kp = keypair();
    register_decider(
        &mut store,
        "stiwi",
        POLICY,
        &crate::signing::public_key_hex(&kp),
    );
    let sig = crate::signing::sign_hex(&kp, &decision_message(&hash, "reject", "stiwi"));
    write_decision(&mut store, "approve", "stiwi", &hash, Some(&sig));
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    assert!(
        !ruling.permits(),
        "a re-pointed signature must not permit: {ruling:?}"
    );
}

// ── Re-minting after expiry (quipu-fu0) ──────────────────────────────────────

#[test]
fn a_fresh_mint_supersedes_an_expired_request() {
    // Expiry denies the REQUEST, not the (policy, target) pair forever. A
    // re-mint at a later instant reopens the window: resolve takes the newest
    // expiry, so the pair is Pending again rather than Expired for good.
    let mut store = store_with_request(600);
    assert_eq!(
        resolve(&store, POLICY, TARGET, NOW + 601).unwrap().unwrap(),
        Ruling::Expired
    );
    let again = mint_request(&store, POLICY, TARGET, None, 600, NOW + 601, TS).unwrap();
    store.transact(&again, TS, None, None).unwrap();
    assert_eq!(
        resolve(&store, POLICY, TARGET, NOW + 601).unwrap().unwrap(),
        Ruling::Pending {
            expires_at: NOW + 1201
        }
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
    decide(&mut store, "approve", "stiwi", &hash, "http://ex/PE");

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
    // quipu-fu0: the expiry denied the REQUEST, not the pair forever — the
    // refusal says a fresh request was opened, and one exists (same
    // deterministic IRI, so still exactly one).
    assert!(
        why.contains("fresh"),
        "the refusal must say the channel reopened: {why}"
    );
    assert_eq!(open_requests(&store), 1);

    // And the reopened channel is live: a signed approval recorded after the
    // expiry lets a later attempt through.
    let hash = evidence_hash("http://ex/PE", "http://ex/d9");
    decide(&mut store, "approve", "stiwi", &hash, "http://ex/PE");
    store
        .transact(&noncompliant_doc(&store, "http://ex/d9"), TS, None, None)
        .expect("an approval after expiry still lets the write through");
}

// ── Escalation precedent (quipu-8dk) ─────────────────────────────────────────
//
// Design #4 (docs/design/semantic-grounded-edit-policies.md): a minted request
// carries its nearest prior DECIDED requests, scored and method-named, so the
// operator sees precedent with the similarity claim falsifiable on record.
// Advisory throughout: every degraded path must still MINT, with no precedent
// and no fabricated score.

use std::sync::Arc;

/// Deterministic test embedder: dimensions count "alpha"/"beta"/"gamma"
/// occurrences, so similarity is legible from the target IRIs — same word,
/// cosine 1.0; disjoint words, cosine 0.0; mixed, in between.
struct CountEmbedder;

impl crate::embedding::EmbeddingProvider for CountEmbedder {
    fn embed_text(&self, text: &str) -> crate::error::Result<Vec<f32>> {
        Ok(["alpha", "beta", "gamma"]
            .iter()
            .map(|w| text.matches(w).count() as f32)
            .collect())
    }
    fn dimension(&self) -> usize {
        3
    }
}

/// An embedder that always fails — the advisory-path-must-not-block case.
struct FailingEmbedder;

impl crate::embedding::EmbeddingProvider for FailingEmbedder {
    fn embed_text(&self, _text: &str) -> crate::error::Result<Vec<f32>> {
        Err(crate::error::Error::Store(
            "the model is on fire".to_string(),
        ))
    }
    fn dimension(&self) -> usize {
        3
    }
}

/// Mint a request for `target` under [`POLICY`] and transact it.
fn mint_for(store: &mut Store, target: &str) {
    let datums = mint_request(store, POLICY, target, None, 600, NOW, TS).unwrap();
    store.transact(&datums, TS, None, None).unwrap();
}

/// Every `(prior-request-iri, score, method)` precedent link in the store.
fn precedent_links(store: &Store) -> Vec<(String, f64, String)> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?prior ?score ?method WHERE {{ \
            ?r a a:DecisionRequest ; a:precedent ?l . \
            ?l a:precedentRequest ?prior ; a:similarityScore ?score ; \
               a:similarityMethod ?method . \
         }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &q).unwrap() else {
        panic!("select")
    };
    rows.iter()
        .filter_map(|row| {
            let Some(Value::Ref(id)) = row.get("prior") else {
                return None;
            };
            let Some(Value::Float(score)) = row.get("score") else {
                return None;
            };
            let Some(Value::Str(method)) = row.get("method") else {
                return None;
            };
            Some((store.resolve(*id).unwrap(), *score, method.clone()))
        })
        .collect()
}

#[test]
fn minting_attaches_the_nearest_decided_request_with_its_score_on_record() {
    // THE case: a near prior with a signed ruling becomes precedent; a far one
    // does not; and the link carries score + method, so the nearness claim is
    // an experiment a reader can re-run, not an assertion.
    let mut store = Store::open_in_memory().unwrap();
    mint_for(&mut store, "http://ex/alpha-1");
    mint_for(&mut store, "http://ex/beta-1");
    decide(
        &mut store,
        "reject",
        "alice",
        &evidence_hash(POLICY, "http://ex/alpha-1"),
        POLICY,
    );
    decide(
        &mut store,
        "reject",
        "bob",
        &evidence_hash(POLICY, "http://ex/beta-1"),
        POLICY,
    );
    store.set_embedding_provider(Arc::new(CountEmbedder));

    mint_for(&mut store, "http://ex/alpha-2");

    let links = precedent_links(&store);
    assert_eq!(
        links.len(),
        1,
        "only the near prior is precedent — beta scores 0.0, and zero \
         similarity is no precedent: {links:?}"
    );
    let (prior, score, method) = &links[0];
    let alpha_hash = evidence_hash(POLICY, "http://ex/alpha-1");
    assert!(
        prior.contains(&alpha_hash[7..20]),
        "the link names the near prior's deterministic request IRI: {prior}"
    );
    assert!(
        (*score - 1.0).abs() < 1e-9,
        "identical embedding direction must score 1.0: {score}"
    );
    assert!(
        method.starts_with("embedding:"),
        "the method identity rides the score — without it the score is \
         unfalsifiable: {method}"
    );
}

#[test]
fn a_request_is_not_its_own_precedent() {
    // The re-mint case: a decided request under the SAME (policy, target)
    // shares the new mint's evidence hash, and citing it would tell the
    // operator "you already ruled on exactly this" about the very ruling
    // that made the re-mint necessary.
    let mut store = Store::open_in_memory().unwrap();
    mint_for(&mut store, "http://ex/alpha-1");
    decide(
        &mut store,
        "reject",
        "alice",
        &evidence_hash(POLICY, "http://ex/alpha-1"),
        POLICY,
    );
    store.set_embedding_provider(Arc::new(CountEmbedder));
    // Re-mint the same pair (same deterministic IRI, fresh expiry).
    mint_for(&mut store, "http://ex/alpha-1");
    assert!(
        precedent_links(&store).is_empty(),
        "a request must not cite its own prior incarnation as precedent"
    );
}

#[test]
fn an_unsigned_decision_does_not_make_a_prior_precedent() {
    // The signed-decider rule, REUSED not restated (decision_verifies): a
    // ruling forgeable by any writer is no ruling for resolve, and no
    // precedent here — otherwise mallory curates what the operator sees.
    let mut store = Store::open_in_memory().unwrap();
    mint_for(&mut store, "http://ex/alpha-1");
    write_decision(
        &mut store,
        "reject",
        "mallory",
        &evidence_hash(POLICY, "http://ex/alpha-1"),
        None,
    );
    store.set_embedding_provider(Arc::new(CountEmbedder));
    mint_for(&mut store, "http://ex/alpha-2");
    assert!(
        precedent_links(&store).is_empty(),
        "an unsigned decision must not turn its request into precedent"
    );
}

#[test]
fn without_a_similarity_method_minting_is_clean_and_attaches_nothing() {
    // The degraded state: no embedding provider means no method, and the
    // advisory is ABSENT — not an error, and not a cheaper heuristic quietly
    // scoring under a label it cannot honour.
    let mut store = Store::open_in_memory().unwrap();
    mint_for(&mut store, "http://ex/alpha-1");
    decide(
        &mut store,
        "reject",
        "alice",
        &evidence_hash(POLICY, "http://ex/alpha-1"),
        POLICY,
    );
    mint_for(&mut store, "http://ex/alpha-2");
    assert!(
        precedent_links(&store).is_empty(),
        "no provider must mean no precedent, never a fabricated score"
    );
    // And the mint itself is whole: the new request resolves as Pending.
    assert!(matches!(
        resolve(&store, POLICY, "http://ex/alpha-2", NOW).unwrap(),
        Some(Ruling::Pending { .. })
    ));
}

#[test]
fn a_failing_provider_cannot_block_minting() {
    // Minting is load-bearing; precedent is advice. An advisory path that can
    // veto the mint has the authority relation backwards, so a provider that
    // errors degrades to no precedent — the request still lands.
    let mut store = Store::open_in_memory().unwrap();
    mint_for(&mut store, "http://ex/alpha-1");
    decide(
        &mut store,
        "reject",
        "alice",
        &evidence_hash(POLICY, "http://ex/alpha-1"),
        POLICY,
    );
    store.set_embedding_provider(Arc::new(FailingEmbedder));
    let datums = mint_request(&store, POLICY, "http://ex/alpha-2", None, 600, NOW, TS)
        .expect("a failing advisory path must not fail the mint");
    store.transact(&datums, TS, None, None).unwrap();
    assert!(precedent_links(&store).is_empty());
    assert!(matches!(
        resolve(&store, POLICY, "http://ex/alpha-2", NOW).unwrap(),
        Some(Ruling::Pending { .. })
    ));
}

#[test]
fn precedent_is_capped_at_the_nearest_three() {
    // Three shows a pattern; an uncapped list buries the nearest ruling
    // under its own tail. The far prior both exceeds nothing and proves the
    // cap keeps the NEAREST three, not the first three found.
    let mut store = Store::open_in_memory().unwrap();
    for (i, target) in [
        "http://ex/alpha-a",
        "http://ex/alpha-b",
        "http://ex/alpha-c",
        "http://ex/alpha-d",
    ]
    .iter()
    .enumerate()
    {
        mint_for(&mut store, target);
        decide(
            &mut store,
            "reject",
            &format!("decider{i}"),
            &evidence_hash(POLICY, target),
            POLICY,
        );
    }
    store.set_embedding_provider(Arc::new(CountEmbedder));
    mint_for(&mut store, "http://ex/alpha-new");
    assert_eq!(
        precedent_links(&store).len(),
        crate::governance::precedent::MAX_PRECEDENTS,
        "four decided near priors must attach as exactly the capped three"
    );
}

#[test]
fn an_undecided_prior_request_is_not_precedent() {
    // An open question is not precedent for anything: the prior request
    // exists, is near, and has NO ruling — nothing attaches.
    let mut store = Store::open_in_memory().unwrap();
    mint_for(&mut store, "http://ex/alpha-1");
    store.set_embedding_provider(Arc::new(CountEmbedder));
    mint_for(&mut store, "http://ex/alpha-2");
    assert!(
        precedent_links(&store).is_empty(),
        "an undecided request must not be cited as precedent"
    );
}

#[test]
fn a_rejection_carries_the_reject_to_policy_offer() {
    // The escalation seam is where a human already rules on instances, and a
    // rejection is the "not this" signal (policy-by-example design, step 4).
    // The refusal must therefore carry the offer to widen the ruling into a
    // standing rule — naming the request record as the exemplar a draft would
    // cite, and the gesture (draft, then backtest) to take it up. Advisory
    // text only: nothing is created here.
    let mut store = store_with_request(600);
    decide(
        &mut store,
        "reject",
        "stiwi",
        &evidence_hash(POLICY, TARGET),
        POLICY,
    );
    let ruling = resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap();
    let why = ruling.reason(POLICY, TARGET);
    assert!(
        why.contains(&request_iri(POLICY, TARGET)),
        "the offer must name the exemplar-candidate record by IRI: {why}"
    );
    assert!(
        why.contains("quipu policy draft --exemplar"),
        "and the gesture that takes it up: {why}"
    );
    assert!(
        why.contains("backtest") && why.contains("warn"),
        "and the born-advisory, backtest-first contract: {why}"
    );
}

#[test]
fn only_a_rejection_makes_the_offer() {
    // The paired green case. Pending and Expired are ABSENCES of a ruling —
    // there is no "not this" signal to widen, and offering policy drafting on
    // every unserviced escalation would train operators to ignore the offer
    // where it means something.
    let store = store_with_request(600);
    for ruling in [
        resolve(&store, POLICY, TARGET, NOW).unwrap().unwrap(),
        resolve(&store, POLICY, TARGET, NOW + 601).unwrap().unwrap(),
    ] {
        let why = ruling.reason(POLICY, TARGET);
        assert!(
            !why.contains("policy draft"),
            "{ruling:?} must not carry the offer: {why}"
        );
    }
}

#[test]
fn the_cited_request_iri_is_the_minted_one() {
    // The offer names an IRI derived without a store in hand; mint_request
    // interns one derived with it. If the two functions drifted, every offer
    // would cite a record that does not exist — checked here by minting and
    // resolving the derived IRI against what actually landed.
    let store = store_with_request(600);
    let iri = request_iri(POLICY, TARGET);
    let q = format!("ASK {{ <{iri}> ?p ?o }}");
    assert!(
        matches!(
            crate::sparql::query(&store, &q).unwrap(),
            crate::sparql::QueryResult::Ask(true)
        ),
        "the offered IRI must be the minted request's: {iri}"
    );
}
