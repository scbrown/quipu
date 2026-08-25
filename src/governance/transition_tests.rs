//! Transition-signature gate tests (quipu-8cc), in the Q-SARC-PLACEMENT
//! pattern: pure `Transition::violation` units, `Store::transact` liveness
//! through the real write path, and a flag-off control that makes every
//! rejection attributable to this gate and nothing else.

use ring::rand::SystemRandom;
use ring::signature::Ed25519KeyPair;

use super::*;
use crate::signing::{public_key_hex, sign_hex};
use crate::store::Store;
use crate::types::Op;

const TS: &str = "2026-08-25T00:00:00Z";
const RUN: &str = "urn:shuttle:run:triage-7";
const STEP_IRI: &str = "urn:shuttle:workflow:triage/step/review";
const AGENT_IRI: &str = "urn:shuttle:agent:alice";

fn keypair() -> Ed25519KeyPair {
    let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

/// Sign the canonical message the way shuttle's `advance` does, with the
/// field derivations `verify_write` re-applies: step and agent as the last
/// IRI segments.
fn sign(kp: &Ed25519KeyPair, from: &str, to: &str) -> String {
    sign_hex(
        kp,
        &transition_message(RUN, "review", from, to, TS, "alice"),
    )
}

// ── Pure-rule units ──────────────────────────────────────────────────────────

/// A complete event whose signature the closure-supplied registry can check.
fn event(signature: Option<&str>) -> Transition {
    Transition {
        in_run: vec![RUN.to_string()],
        at_step: vec![STEP_IRI.to_string()],
        from_state: vec!["open".to_string()],
        to_state: vec!["reviewed".to_string()],
        ended_at: vec![TS.to_string()],
        performed_by: vec![AGENT_IRI.to_string()],
        signature: signature.map(str::to_string).into_iter().collect(),
    }
}

fn keys_of(pk: &str) -> impl Fn(&str) -> crate::error::Result<Vec<String>> + '_ {
    move |agent: &str| {
        Ok(if agent == "alice" {
            vec![pk.to_string()]
        } else {
            Vec::new()
        })
    }
}

#[test]
fn the_message_format_is_shuttles_verbatim() {
    // The cross-repo contract. shuttle/signing.py::transition_message joins
    // the same seven slots with '|'; drift here refuses every genuine
    // transition.
    assert_eq!(
        transition_message(RUN, "review", "open", "reviewed", TS, "alice"),
        format!("shuttle-transition-v1|{RUN}|review|open|reviewed|{TS}|alice").into_bytes()
    );
}

#[test]
fn an_unsigned_transition_is_rejected() {
    let kp = keypair();
    let pk = public_key_hex(&kp);
    let why = event(None)
        .violation("urn:ev", &keys_of(&pk))
        .unwrap()
        .unwrap();
    assert!(why.contains("urn:ev"), "names the event: {why}");
    assert!(
        why.contains("aegis:signature"),
        "names the missing field: {why}"
    );
    assert!(
        why.contains("Sign the transition"),
        "names the remedy: {why}"
    );
}

#[test]
fn a_genuine_signature_verifies() {
    let kp = keypair();
    let pk = public_key_hex(&kp);
    let sig = sign(&kp, "open", "reviewed");
    assert!(
        event(Some(&sig))
            .violation("urn:ev", &keys_of(&pk))
            .unwrap()
            .is_none(),
        "a signature by the registered key over the exact fields must verify"
    );
}

#[test]
fn an_unregistered_signer_is_rejected() {
    let kp = keypair();
    let sig = sign(&kp, "open", "reviewed");
    let none = |_: &str| Ok(Vec::new());
    let why = event(Some(&sig))
        .violation("urn:ev", &none)
        .unwrap()
        .unwrap();
    assert!(
        why.contains("alice") && why.contains("VerifierRegistration"),
        "the refusal names the agent and the missing registration: {why}"
    );
    assert!(why.contains("register"), "names the remedy: {why}");
}

#[test]
fn a_signature_under_the_wrong_key_is_rejected() {
    let signer = keypair();
    let registered = keypair(); // a DIFFERENT identity holds the registration
    let pk = public_key_hex(&registered);
    let sig = sign(&signer, "open", "reviewed");
    let why = event(Some(&sig))
        .violation("urn:ev", &keys_of(&pk))
        .unwrap()
        .unwrap();
    assert!(
        why.contains("does not verify"),
        "a wrong-key signature is a verification failure, not a registry miss: {why}"
    );
}

#[test]
fn a_tampered_payload_is_rejected() {
    // Signed for open→reviewed; the staged facts claim open→approved. The
    // signature is genuine and the signer registered — the CONTENT lies.
    let kp = keypair();
    let pk = public_key_hex(&kp);
    let sig = sign(&kp, "open", "reviewed");
    let mut tampered = event(Some(&sig));
    tampered.to_state = vec!["approved".to_string()];
    let why = tampered
        .violation("urn:ev", &keys_of(&pk))
        .unwrap()
        .unwrap();
    assert!(
        why.contains("does not verify") && why.contains("approved"),
        "the refusal shows the fields actually checked: {why}"
    );
}

#[test]
fn an_incomplete_event_is_rejected_not_waved_through() {
    // No fromState => the canonical message cannot be re-derived. An
    // uncheckable signature must refuse, never pass.
    let kp = keypair();
    let pk = public_key_hex(&kp);
    let sig = sign(&kp, "open", "reviewed");
    let mut partial = event(Some(&sig));
    partial.from_state = Vec::new();
    let why = partial.violation("urn:ev", &keys_of(&pk)).unwrap().unwrap();
    assert!(
        why.contains("aegis:fromState") && why.contains("cannot be re-derived"),
        "names the missing field and why it is fatal: {why}"
    );
}

#[test]
fn an_ambiguous_field_is_rejected() {
    // Two toState values: one slot in the message, two candidates. Refusing
    // is the only reading that cannot be wrong (the placement discipline).
    let kp = keypair();
    let pk = public_key_hex(&kp);
    let sig = sign(&kp, "open", "reviewed");
    let mut two_valued = event(Some(&sig));
    two_valued.to_state.push("approved".to_string());
    let why = two_valued
        .violation("urn:ev", &keys_of(&pk))
        .unwrap()
        .unwrap();
    assert!(
        why.contains("distinct values") && why.contains("aegis:toState"),
        "names the ambiguity: {why}"
    );
}

#[test]
fn a_pipe_inside_a_field_is_rejected_as_shuttles_signer_would() {
    let kp = keypair();
    let pk = public_key_hex(&kp);
    let sig = sign(&kp, "open", "reviewed");
    let mut evil = event(Some(&sig));
    evil.from_state = vec!["open|reviewed".to_string()];
    let why = evil.violation("urn:ev", &keys_of(&pk)).unwrap().unwrap();
    assert!(why.contains("ambiguous"), "{why}");
}

// ── Liveness through the real write path ─────────────────────────────────────

fn assert_datum(store: &mut Store, entity: i64, attr: &str, value: Value) -> Datum {
    Datum {
        entity,
        attribute: store.intern(attr).unwrap(),
        value,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

/// The datums shuttle's export stages for one `TransitionEvent` — type, run,
/// step, states, time, agent, and (optionally) the signature.
fn transition_datums(store: &mut Store, to: &str, signature: Option<&str>) -> Vec<Datum> {
    let ev = store.intern("urn:shuttle:event:triage-7:0").unwrap();
    let run = store.intern(RUN).unwrap();
    let step = store.intern(STEP_IRI).unwrap();
    let agent = store.intern(AGENT_IRI).unwrap();
    let class = store
        .intern(&format!("{DEFAULT_BASE_NS}TransitionEvent"))
        .unwrap();
    let rdf_type = store.intern(RDF_TYPE).unwrap();
    let mut datums = vec![
        Datum {
            entity: ev,
            attribute: rdf_type,
            value: Value::Ref(class),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        },
        assert_datum(
            store,
            ev,
            &format!("{DEFAULT_BASE_NS}inRun"),
            Value::Ref(run),
        ),
        assert_datum(
            store,
            ev,
            &format!("{DEFAULT_BASE_NS}atStep"),
            Value::Ref(step),
        ),
        assert_datum(
            store,
            ev,
            &format!("{DEFAULT_BASE_NS}fromState"),
            Value::Str("open".into()),
        ),
        assert_datum(
            store,
            ev,
            &format!("{DEFAULT_BASE_NS}toState"),
            Value::Str(to.into()),
        ),
        assert_datum(
            store,
            ev,
            &format!("{PROV}endedAtTime"),
            Value::Str(TS.into()),
        ),
        assert_datum(
            store,
            ev,
            &format!("{DEFAULT_BASE_NS}performedBy"),
            Value::Ref(agent),
        ),
    ];
    if let Some(sig) = signature {
        datums.push(assert_datum(
            store,
            ev,
            &format!("{DEFAULT_BASE_NS}signature"),
            Value::Str(sig.into()),
        ));
    }
    datums
}

/// Register `pk` for alice — the fact shape `shuttle register` prints for a
/// human to load.
fn register(store: &mut Store, pk: &str, graph: i64) {
    let reg = store.intern("urn:shuttle:registration:alice").unwrap();
    let class = store
        .intern(&format!("{DEFAULT_BASE_NS}VerifierRegistration"))
        .unwrap();
    let rdf_type = store.intern(RDF_TYPE).unwrap();
    let datums = vec![
        Datum {
            entity: reg,
            attribute: rdf_type,
            value: Value::Ref(class),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        },
        assert_datum(
            store,
            reg,
            &format!("{DEFAULT_BASE_NS}verifier"),
            Value::Str("alice".into()),
        ),
        assert_datum(
            store,
            reg,
            &format!("{DEFAULT_BASE_NS}publicKey"),
            Value::Str(pk.into()),
        ),
    ];
    store
        .transact_to_graph(&datums, TS, None, None, graph)
        .expect("the registration must land");
}

#[test]
fn an_unsigned_transition_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let datums = transition_datums(&mut store, "reviewed", None);
    let err = store.transact(&datums, TS, None, None);
    let Err(Error::PolicyDenied(why)) = err else {
        panic!("an unsigned TransitionEvent must be refused at write, got {err:?}");
    };
    assert!(why.contains("aegis:signature"), "{why}");
    // The rollback contract: a refused transition leaves nothing behind.
    assert!(
        !matches!(
            crate::sparql::query(&store, "ASK { <urn:shuttle:event:triage-7:0> ?p ?o }").unwrap(),
            crate::sparql::QueryResult::Ask(true)
        ),
        "a refused transition must leave the store byte-identical"
    );
}

#[test]
fn a_signed_registered_transition_lands_with_the_flag_on() {
    // The GREEN case, end to end: real key, real registration, real gate.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let kp = keypair();
    register(&mut store, &public_key_hex(&kp), 0);
    let sig = sign(&kp, "open", "reviewed");
    let datums = transition_datums(&mut store, "reviewed", Some(&sig));
    store
        .transact(&datums, TS, None, None)
        .expect("a genuinely signed transition by a registered agent must land");
}

#[test]
fn the_registration_is_found_in_a_named_identity_graph() {
    // Shuttle's convention: registrations live in a dataKind=identity NAMED
    // graph, not ROOT. The registry read must span graphs or the gate refuses
    // every genuine transition in that deployment shape.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let identity = store.graph_create("urn:app:identity").unwrap();
    let kp = keypair();
    register(&mut store, &public_key_hex(&kp), identity);
    let sig = sign(&kp, "open", "reviewed");
    let datums = transition_datums(&mut store, "reviewed", Some(&sig));
    store
        .transact(&datums, TS, None, None)
        .expect("a registration in the identity graph must authenticate the write");
}

#[test]
fn an_unregistered_signer_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let kp = keypair(); // never registered
    let sig = sign(&kp, "open", "reviewed");
    let datums = transition_datums(&mut store, "reviewed", Some(&sig));
    let err = store.transact(&datums, TS, None, None);
    let Err(Error::PolicyDenied(why)) = err else {
        panic!("an unregistered signer must be refused at write, got {err:?}");
    };
    assert!(
        why.contains("alice") && why.contains("VerifierRegistration"),
        "{why}"
    );
}

#[test]
fn a_wrong_key_signature_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let registered = keypair();
    register(&mut store, &public_key_hex(&registered), 0);
    let forger = keypair();
    let sig = sign(&forger, "open", "reviewed");
    let datums = transition_datums(&mut store, "reviewed", Some(&sig));
    let err = store.transact(&datums, TS, None, None);
    assert!(
        matches!(err, Err(Error::PolicyDenied(_))),
        "a signature under an unregistered key must be refused, got {err:?}"
    );
}

#[test]
fn a_tampered_payload_is_refused_at_the_write_path() {
    // Alice signs open→reviewed; the export claims open→approved. Genuine
    // key, registered signer, lying content.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let kp = keypair();
    register(&mut store, &public_key_hex(&kp), 0);
    let sig = sign(&kp, "open", "reviewed");
    let datums = transition_datums(&mut store, "approved", Some(&sig));
    let err = store.transact(&datums, TS, None, None);
    let Err(Error::PolicyDenied(why)) = err else {
        panic!("a tampered transition must be refused at write, got {err:?}");
    };
    assert!(why.contains("does not verify"), "{why}");
}

#[test]
fn the_control_an_unsigned_transition_lands_with_the_flag_off() {
    // Same datums, flag off => accepted. That is what makes every rejection
    // above attributable to this gate — and it is today's default behaviour,
    // unchanged: turning verification on is a deliberate configuration act.
    let mut store = Store::open_in_memory().unwrap();
    assert!(
        !store.governance_config_mut().verify_transitions,
        "the flag must default to off"
    );
    let datums = transition_datums(&mut store, "reviewed", None);
    store
        .transact(&datums, TS, None, None)
        .expect("with verification off the same write lands");
}

#[test]
fn a_non_transition_write_is_not_touched() {
    // The pre-filter: ordinary traffic costs nothing and is never refused.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().verify_transitions = true;
    let e = store.intern("urn:test:thing").unwrap();
    let datums = vec![assert_datum(
        &mut store,
        e,
        "urn:test:state",
        Value::Str("open".into()),
    )];
    store
        .transact(&datums, TS, None, None)
        .expect("an ordinary write must pass untouched");
}
