use super::{Answer, Object, Request, Truth, judgment::*, ledger::ns, query};
use crate::{Datum, Op, Store, Value, governance::decision_seal};

const TS: &str = "2026-01-01T00:00:00Z";
const NOW: i64 = 1_767_225_600;
const POLICY: &str = "urn:policy:referee";
const AUTHORITY: &str = "test-court";

fn put(store: &mut Store, subject: &str, predicate: &str, value: Value) {
    let datum = Datum {
        entity: store.intern(subject).unwrap(),
        attribute: store.intern(predicate).unwrap(),
        value,
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[datum], TS, None, Some("fixture")).unwrap();
}

fn setup(positive: bool, negative: bool) -> (Store, Request, ring::signature::Ed25519KeyPair) {
    let mut store = Store::open_in_memory().unwrap();
    store.load_shapes("rule-answers", PROFILE, TS).unwrap();
    let request = Request {
        subject: "urn:claim".into(),
        positive_predicate: "urn:positive".into(),
        negative_predicate: "urn:negative".into(),
        object: Object::Iri("urn:target".into()),
        judgment_policy: POLICY.into(),
        witness_limit: 64,
    };
    let target = store.intern("urn:target").unwrap();
    for (present, predicate) in [(positive, "urn:positive"), (negative, "urn:negative")] {
        if present {
            put(&mut store, "urn:claim", predicate, Value::Ref(target));
        }
    }
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let key = ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    crate::governance::trust_root::test_support::register_human(
        &mut store,
        "urn:registration",
        AUTHORITY,
        &[POLICY],
        &crate::signing::public_key_hex(&key),
        TS,
    );
    (store, request, key)
}

fn judgment(
    store: &mut Store,
    request: &Request,
    key: &ring::signature::Ed25519KeyPair,
    decision: &str,
    kind: Kind,
    predecessor: Option<Predecessor>,
) -> (String, String) {
    let claim = Claim::from(request);
    let record = Record {
        protocol: "quipu.rule-judgment/v1".into(),
        claim: claim.clone(),
        authority: AUTHORITY.into(),
        reason: "Recorded referee disposition; no action permission".into(),
        disposition: kind,
        predecessor,
    };
    let class = store.intern(&ns("Decision")).unwrap();
    put(
        store,
        decision,
        crate::namespace::RDF_TYPE,
        Value::Ref(class),
    );
    put(
        store,
        decision,
        "http://www.w3.org/2000/01/rdf-schema#label",
        Value::Str("judgment".into()),
    );
    put(store, decision, MARKER, Value::Str(claim.digest().unwrap()));
    put(
        store,
        decision,
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#value",
        Value::Str(serde_json::to_string(&record).unwrap()),
    );
    put(
        store,
        decision,
        &ns("decisionPolicy"),
        Value::Str(POLICY.into()),
    );
    // Existing DecisionShape is an approve/reject/changes carrier. The
    // disposition itself is inside the sealed purpose-specific JSON.
    let outcome = "approve";
    put(store, decision, &ns("outcome"), Value::Str(outcome.into()));
    put(store, decision, &ns("by"), Value::Str(AUTHORITY.into()));
    put(
        store,
        decision,
        &ns("evidenceHash"),
        Value::Str(format!("sha256:{}", claim.digest().unwrap())),
    );
    put(store, decision, &ns("option"), Value::Str(outcome.into()));
    #[cfg(feature = "shacl")]
    {
        let subject = store.lookup(decision).unwrap().unwrap();
        let subject_term = crate::rdf::value_to_term(store, &Value::Ref(subject)).unwrap();
        let data = store
            .entity_facts(subject)
            .unwrap()
            .into_iter()
            .map(|fact| {
                format!(
                    "{subject_term} <{}> {} .\n",
                    store.resolve(fact.attribute).unwrap(),
                    crate::rdf::value_to_term(store, &fact.value).unwrap()
                )
            })
            .collect::<String>();
        let shapes = format!("{}\n{PROFILE}", include_str!("../../shapes/governance.ttl"));
        let report = crate::validate_shapes(&shapes, &data).unwrap();
        assert!(report.conforms, "Decision/profile intersection: {report:?}");
    }
    let presentation = decision_seal::present(store, decision, 3600, NOW).unwrap();
    let signature = crate::signing::sign_hex(key, &presentation.challenge(outcome));
    let verdict = decision_seal::attest(
        store,
        &presentation.nonce,
        outcome,
        AUTHORITY,
        &signature,
        NOW + 1,
    )
    .unwrap()
    .unwrap();
    (verdict, presentation.digest)
}

#[test]
fn rule_answer_states_all_four_independently_deferred_and_authorized_resolution_retains_history() {
    for (positive, negative, expected) in [
        (true, false, Truth::Supported),
        (false, true, Truth::Refuted),
        (true, true, Truth::Both),
        (false, false, Truth::Neither),
    ] {
        let (mut store, request, key) = setup(positive, negative);
        let before = query(&store, &request).unwrap();
        assert_eq!(
            crate::tool_rule_query(&store, &serde_json::to_value(&request).unwrap()).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        assert!(matches!(
            before,
            Answer::Complete {
                judgment: Judgment::NoneRecorded,
                ..
            }
        ));
        let (verdict, digest) = judgment(
            &mut store,
            &request,
            &key,
            "urn:decision:defer",
            Kind::Deferred,
            None,
        );
        let deferred = query(&store, &request).unwrap();
        assert_eq!(
            crate::tool_rule_query(&store, &serde_json::to_value(&request).unwrap()).unwrap(),
            serde_json::to_value(&deferred).unwrap()
        );
        let Answer::Complete {
            evidence: super::Evidence::Complete { truth, .. },
            judgment: Judgment::Deferred { current },
            ..
        } = deferred
        else {
            panic!("{deferred:?}");
        };
        assert_eq!(truth, expected);
        assert!(current.verdicts.contains(&verdict));
        judgment(
            &mut store,
            &request,
            &key,
            "urn:decision:resolve",
            Kind::Resolved,
            Some(Predecessor {
                verdict,
                decision_digest: digest,
            }),
        );
        let resolved = query(&store, &request).unwrap();
        assert_eq!(
            crate::tool_rule_query(&store, &serde_json::to_value(&request).unwrap()).unwrap(),
            serde_json::to_value(&resolved).unwrap()
        );
        let Answer::Complete {
            evidence: super::Evidence::Complete { truth, .. },
            judgment: Judgment::Resolved { earlier, .. },
            ..
        } = &resolved
        else {
            panic!("{resolved:?}");
        };
        assert_eq!(*truth, expected);
        assert_eq!(earlier, &current);
        assert_eq!(query(&store, &request).unwrap(), resolved);
        let wire = serde_json::to_value(&resolved).unwrap();
        assert_eq!(wire["action_authority"], "not_granted");
        assert_eq!(serde_json::from_value::<Answer>(wire).unwrap(), resolved);
    }
}

#[test]
fn rule_answer_states_wrong_policy_tamper_or_missing_profile_is_unknown_not_none() {
    let (mut store, request, key) = setup(true, true);
    judgment(
        &mut store,
        &request,
        &key,
        "urn:decision:defer",
        Kind::Deferred,
        None,
    );
    let mut wrong_policy = request.clone();
    wrong_policy.judgment_policy = "urn:other-policy".into();
    assert!(matches!(
        query(&store, &wrong_policy).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
    put(
        &mut store,
        "urn:decision:defer",
        &ns("question"),
        Value::Str("tampered after signing".into()),
    );
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
    store.remove_shapes("rule-answers").unwrap();
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
}

#[test]
fn rule_answer_states_signed_orphan_resolution_and_competing_deferrals_are_unknown() {
    let (mut store, request, key) = setup(false, false);
    judgment(
        &mut store,
        &request,
        &key,
        "urn:orphan",
        Kind::Resolved,
        Some(Predecessor {
            verdict: "urn:missing".into(),
            decision_digest: "sha256:missing".into(),
        }),
    );
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
    let (mut store, request, key) = setup(true, false);
    judgment(
        &mut store,
        &request,
        &key,
        "urn:first",
        Kind::Deferred,
        None,
    );
    judgment(
        &mut store,
        &request,
        &key,
        "urn:second",
        Kind::Deferred,
        None,
    );
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
}

#[test]
fn rule_answer_states_copied_signed_verdict_without_nonce_admission_is_unknown() {
    let (mut store, request, key) = setup(true, true);
    let (verdict, _) = judgment(
        &mut store,
        &request,
        &key,
        "urn:decision",
        Kind::Deferred,
        None,
    );
    let original = store.lookup(&verdict).unwrap().unwrap();
    let facts = store.entity_facts(original).unwrap();
    for fact in facts {
        let predicate = store.resolve(fact.attribute).unwrap();
        put(&mut store, "urn:copied-verdict", &predicate, fact.value);
    }
    // The genuine one still verifies; a copied signature cannot be silently ignored.
    assert!(
        decision_seal::verify_recorded(&store, &verdict)
            .unwrap()
            .is_ok()
    );
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
}

#[test]
fn rule_answer_states_configured_quality_floor_refusal_cannot_become_neither() {
    let (mut store, request, _) = setup(false, false);
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Complete { .. }
    ));
    store.labels_config_mut().min_freshness = Some("fresh".into());
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
}

#[test]
fn rule_answer_states_redirected_presentation_nonce_refuses_before_seal_traversal() {
    let (mut store, request, key) = setup(true, false);
    let (verdict, _) = judgment(
        &mut store,
        &request,
        &key,
        "urn:decision",
        Kind::Deferred,
        None,
    );
    let verdict_facts = super::ledger::entity(&store, &verdict).unwrap();
    let presentation = super::ledger::reference(&verdict_facts, &ns("forPresentation")).unwrap();
    for fact in store.entity_facts(presentation).unwrap() {
        let predicate = store.resolve(fact.attribute).unwrap();
        put(
            &mut store,
            "urn:redirected-presentation",
            &predicate,
            fact.value,
        );
    }
    let response = query(&store, &request).unwrap();
    let Answer::Unknown {
        reason,
        truth: None,
        ..
    } = response
    else {
        panic!("{response:?}");
    };
    assert!(reason.contains("presentation identity does not match native nonce"));
    assert!(
        decision_seal::verify_recorded(&store, &verdict)
            .unwrap()
            .is_ok()
    );
}

#[test]
fn rule_answer_states_withdrawing_marker_cannot_clear_a_deferral() {
    let (mut store, request, key) = setup(true, true);
    judgment(
        &mut store,
        &request,
        &key,
        "urn:decision",
        Kind::Deferred,
        None,
    );
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Complete {
            judgment: Judgment::Deferred { .. },
            ..
        }
    ));
    let decision = store.lookup("urn:decision").unwrap().unwrap();
    let marker = store.lookup(MARKER).unwrap();
    store
        .retract_triples(decision, marker, None, TS, None, false, None)
        .unwrap();
    assert!(matches!(
        query(&store, &request).unwrap(),
        Answer::Unknown { truth: None, .. }
    ));
}
