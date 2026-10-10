//! Sealed-decision tests (aegis-kzt0ql.9.3). Size-exempt.

use super::*;

const TS: &str = "2026-01-01T00:00:00Z";
const NOW: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z
const POLICY: &str = "http://ex/policy/wire-funds";
const D: &str = "http://ex/decision/1";

fn keypair() -> ring::signature::Ed25519KeyPair {
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

fn put(store: &mut Store, s: &str, p: &str, v: Value) {
    let datum = Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[datum], TS, None, None).unwrap();
}

fn register(store: &mut Store, verifier: &str, key_hex: &str) {
    let reg = format!("http://ex/reg/{verifier}");
    let class = Value::Ref(store.intern(&ns("VerifierRegistration")).unwrap());
    put(store, &reg, RDF_TYPE, class);
    put(store, &reg, &ns("verifier"), Value::Str(verifier.into()));
    put(store, &reg, &ns("attests"), Value::Str(POLICY.into()));
    put(store, &reg, &ns("publicKey"), Value::Str(key_hex.into()));
}

/// A decision with a question, two options (one as a blank node list item) and scope.
fn seeded() -> (Store, ring::signature::Ed25519KeyPair) {
    let mut store = Store::open_in_memory().unwrap();
    put(
        &mut store,
        D,
        &ns("question"),
        Value::Str("Pay invoice 42 for $120?".into()),
    );
    put(&mut store, D, &ns("option"), Value::Str("approve".into()));
    put(&mut store, D, &ns("option"), Value::Str("reject".into()));
    put(
        &mut store,
        D,
        &ns("decisionPolicy"),
        Value::Str(POLICY.into()),
    );
    put(
        &mut store,
        D,
        &ns("authorizesTx"),
        Value::Str("tx:invoice-42".into()),
    );
    let b = store.intern("_:scope1").unwrap();
    put(&mut store, D, &ns("scope"), Value::Ref(b));
    put(&mut store, "_:scope1", &ns("maxAmount"), Value::Int(120));
    let kp = keypair();
    register(&mut store, "stiwi", &crate::signing::public_key_hex(&kp));
    (store, kp)
}

fn sign(kp: &ring::signature::Ed25519KeyPair, p: &Presentation, outcome: &str) -> String {
    crate::signing::sign_hex(kp, &p.challenge(outcome))
}

#[test]
fn a_signed_presentation_is_accepted_and_reverifies() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    let v = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10)
        .unwrap()
        .unwrap();
    assert_eq!(verify_recorded(&store, &v).unwrap(), Ok(()));
    assert!(store.decision_nonce_spent(&p.nonce).unwrap());
}

#[test]
fn any_changed_byte_of_the_decision_fails() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    // One character of the question, in the store, before attestation.
    let e = store.lookup(D).unwrap().unwrap();
    let q = store.lookup(&ns("question")).unwrap();
    store
        .retract_triples(
            e,
            q,
            Some(&Value::Str("Pay invoice 42 for $120?".into())),
            TS,
            None,
            false,
            None,
        )
        .unwrap();
    put(
        &mut store,
        D,
        &ns("question"),
        Value::Str("Pay invoice 42 for $920?".into()),
    );
    let r = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10).unwrap();
    assert!(matches!(r, Err(Refusal::ContentChanged { .. })), "{r:?}");
    assert!(
        !store.decision_nonce_spent(&p.nonce).unwrap(),
        "a refusal writes nothing"
    );
}

#[test]
fn a_nested_blank_node_is_sealed_too() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    put(
        &mut store,
        "_:scope1",
        &ns("maxAmount"),
        Value::Int(120_000),
    );
    let r = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10).unwrap();
    assert!(matches!(r, Err(Refusal::ContentChanged { .. })), "{r:?}");
}

#[test]
fn an_edit_after_attestation_invalidates_the_recorded_verdict() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    let v = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10)
        .unwrap()
        .unwrap();
    put(
        &mut store,
        D,
        &ns("authorizesTx"),
        Value::Str("tx:something-else".into()),
    );
    assert!(matches!(
        verify_recorded(&store, &v).unwrap(),
        Err(Refusal::ContentChanged { .. })
    ));
}

#[test]
fn a_replayed_nonce_is_refused() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10)
        .unwrap()
        .unwrap();
    let again = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 20).unwrap();
    assert_eq!(again, Err(Refusal::Replayed));
}

#[test]
fn the_signature_covers_the_chosen_outcome() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "reject");
    assert_eq!(
        attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10).unwrap(),
        Err(Refusal::BadSignature)
    );
    assert_eq!(
        attest(
            &mut store,
            &p.nonce,
            "wire it anyway",
            "stiwi",
            &sig,
            NOW + 10
        )
        .unwrap(),
        Err(Refusal::NotAnOption)
    );
}

#[test]
fn a_signature_for_another_purpose_or_scheme_fails() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let wrong =
        String::from_utf8(p.challenge("approve"))
            .unwrap()
            .replacen(PURPOSE, "quipu-share", 1);
    let sig = crate::signing::sign_hex(&kp, wrong.as_bytes());
    assert_eq!(
        attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10).unwrap(),
        Err(Refusal::BadSignature)
    );
}

#[test]
fn an_expired_presentation_is_refused() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 60, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    assert_eq!(
        attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 60).unwrap(),
        Err(Refusal::Expired)
    );
}

#[test]
fn an_unregistered_or_unauthorized_key_fails() {
    let (mut store, _) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let stranger = keypair();
    let sig = sign(&stranger, &p, "approve");
    assert_eq!(
        attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10).unwrap(),
        Err(Refusal::BadSignature)
    );
    assert_eq!(
        attest(&mut store, &p.nonce, "approve", "mallory", &sig, NOW + 10).unwrap(),
        Err(Refusal::BadSignature)
    );
}

#[test]
fn the_digest_ignores_blank_node_labels_and_fact_order() {
    let build = |label: &str, reverse: bool| {
        let mut store = Store::open_in_memory().unwrap();
        let facts: Vec<(&str, Value)> = vec![
            ("question", Value::Str("q".into())),
            ("option", Value::Str("approve".into())),
            ("option", Value::Str("reject".into())),
        ];
        let facts: Vec<_> = if reverse {
            facts.into_iter().rev().collect()
        } else {
            facts
        };
        for (p, v) in facts {
            put(&mut store, D, &ns(p), v);
        }
        let b = store.intern(label).unwrap();
        put(&mut store, D, &ns("scope"), Value::Ref(b));
        put(&mut store, label, &ns("maxAmount"), Value::Int(1));
        decision_digest(&store, D).unwrap()
    };
    assert_eq!(build("_:a", false), build("_:zzz", true));
    assert_ne!(build("_:a", false), {
        let mut store = Store::open_in_memory().unwrap();
        put(&mut store, D, &ns("question"), Value::Str("q2".into()));
        decision_digest(&store, D).unwrap()
    });
}

#[test]
fn attestation_fields_do_not_change_the_digest() {
    let (mut store, _) = seeded();
    let before = decision_digest(&store, D).unwrap();
    put(
        &mut store,
        D,
        &ns("signature"),
        Value::Str("deadbeef".into()),
    );
    assert_eq!(decision_digest(&store, D).unwrap(), before);
}

#[test]
fn a_rotated_key_still_reverifies_what_it_sealed() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    let v = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10)
        .unwrap()
        .unwrap();
    // Rotate stiwi's key after the verdict was recorded (S1).
    let reg = store.lookup("http://ex/reg/stiwi").unwrap().unwrap();
    let pk = store.lookup(&ns("publicKey")).unwrap();
    let old = crate::signing::public_key_hex(&kp);
    store
        .retract_triples(
            reg,
            pk,
            Some(&Value::Str(old)),
            "2999-01-01T00:00:00Z",
            None,
            false,
            None,
        )
        .unwrap();
    assert_eq!(verify_recorded(&store, &v).unwrap(), Ok(()));
}

/// Write a verdict straight into the graph, the way an `/episode` writer
/// could until S3, skipping `attest` entirely (wu-rev-345 F1).
fn forge_verdict(
    store: &mut Store,
    verdict: &str,
    p: &Presentation,
    outcome: &str,
    sig: &str,
    at: i64,
) {
    let ts = crate::time::format_iso(u64::try_from(at).unwrap());
    let e = store.intern(verdict).unwrap();
    let fields = [
        (
            RDF_TYPE,
            Value::Ref(store.intern(&ns("DecisionVerdict")).unwrap()),
        ),
        (
            &*ns("forPresentation"),
            Value::Ref(store.intern(&p.iri).unwrap()),
        ),
        (&*ns("outcome"), Value::Str(outcome.into())),
        (&*ns("verifier"), Value::Str("stiwi".into())),
        (&*ns("sealSignature"), Value::Str(sig.into())),
    ];
    let datums: Vec<Datum> = fields
        .into_iter()
        .map(|(pr, v)| Datum {
            entity: e,
            attribute: store.intern(pr).unwrap(),
            value: v,
            valid_from: ts.clone(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store.transact(&datums, &ts, None, Some("episode")).unwrap();
}

fn verdict_iri(p: &Presentation) -> String {
    ns(&format!("decision_verdict_{}", p.nonce))
}

#[test]
fn a_hand_written_verdict_with_a_captured_signature_is_refused() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    // Stiwi signs, but the verdict never goes through attest.
    let sig = sign(&kp, &p, "approve");
    let v = verdict_iri(&p);
    forge_verdict(&mut store, &v, &p, "approve", &sig, NOW + 10);
    assert_eq!(
        verify_recorded(&store, &v).unwrap(),
        Err(Refusal::NotAttested)
    );
    // Control: the signature itself is good, so attest admits it, and the
    // hand-written facts do not ride along on the spend.
    assert!(
        attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 20)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        verify_recorded(&store, &v).unwrap(),
        Err(Refusal::NotAttested)
    );
}

#[test]
fn a_verdict_recorded_after_expiry_is_refused() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 60, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    // attest refused it as Expired; the captured signature is written anyway.
    assert_eq!(
        attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 60).unwrap(),
        Err(Refusal::Expired)
    );
    let v = verdict_iri(&p);
    forge_verdict(&mut store, &v, &p, "approve", &sig, NOW + 3600);
    assert_eq!(verify_recorded(&store, &v).unwrap(), Err(Refusal::Expired));
}

#[test]
fn a_second_verdict_on_a_spent_nonce_is_refused() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    let v = attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10)
        .unwrap()
        .unwrap();
    // A second entity over the same presentation and signature.
    let twin = ns("decision_verdict_twin");
    forge_verdict(&mut store, &twin, &p, "approve", &sig, NOW + 20);
    assert_eq!(
        verify_recorded(&store, &twin).unwrap(),
        Err(Refusal::NotAttested)
    );
    // The attested one still verifies.
    assert_eq!(verify_recorded(&store, &v).unwrap(), Ok(()));
}

#[test]
fn rewriting_an_attested_verdict_with_another_captured_signature_is_refused() {
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let approve = sign(&kp, &p, "approve");
    // Stiwi also signed "reject" once; the writer captured it.
    let reject = sign(&kp, &p, "reject");
    let v = attest(&mut store, &p.nonce, "approve", "stiwi", &approve, NOW + 10)
        .unwrap()
        .unwrap();
    let e = store.lookup(&v).unwrap().unwrap();
    for (field, old) in [("outcome", "approve"), ("sealSignature", approve.as_str())] {
        let a = store.lookup(&ns(field)).unwrap();
        store
            .retract_triples(e, a, Some(&Value::Str(old.into())), TS, None, false, None)
            .unwrap();
    }
    put(&mut store, &v, &ns("outcome"), Value::Str("reject".into()));
    put(&mut store, &v, &ns("sealSignature"), Value::Str(reject));
    assert_eq!(
        verify_recorded(&store, &v).unwrap(),
        Err(Refusal::NotAttested)
    );
}

/// What `present` and `attest` actually write must satisfy the governance
/// shapes, and a presentation missing its nonce must not.
#[cfg(feature = "shacl")]
#[test]
fn written_seal_records_conform_to_the_governance_shapes() {
    const SHAPES: &str = include_str!("../../shapes/governance.ttl");
    let (mut store, kp) = seeded();
    let p = present(&mut store, D, 3600, NOW).unwrap();
    let sig = sign(&kp, &p, "approve");
    attest(&mut store, &p.nonce, "approve", "stiwi", &sig, NOW + 10)
        .unwrap()
        .unwrap();
    let ttl =
        String::from_utf8(crate::rdf::export_rdf(&store, oxrdfio::RdfFormat::Turtle).unwrap())
            .unwrap();
    let report = crate::shacl::validate_shapes(SHAPES, &ttl).unwrap();
    assert!(report.conforms, "{report:#?}");

    let e = store.lookup(&p.iri).unwrap().unwrap();
    let n = store.lookup(&ns("nonce")).unwrap();
    store
        .retract_triples(
            e,
            n,
            Some(&Value::Str(p.nonce.clone())),
            TS,
            None,
            true,
            None,
        )
        .unwrap();
    let ttl =
        String::from_utf8(crate::rdf::export_rdf(&store, oxrdfio::RdfFormat::Turtle).unwrap())
            .unwrap();
    assert!(
        !crate::shacl::validate_shapes(SHAPES, &ttl)
            .unwrap()
            .conforms
    );
}
