//! The as-of rule (signing-plane S1) on the hardware verdict path: a
//! `scheme = webauthn-* | sshsig-*` verdict is read against the registry as it
//! stood when its signature was recorded, exactly like an ed25519 one. Without
//! this a hardware verdict would bypass the recorded-basis rule.

use serde_json::json;

use super::super::governance::tool_verdict_verify;
use crate::governance::verifier_registry::Witness;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};
use crate::verdict_schemes::sshsig_tests::{MESSAGE, SkKey};

const T0: &str = "2026-01-01T00:00:00Z";
const T1: &str = "2026-01-10T00:00:00Z";
const T2: &str = "2026-01-20T00:00:00Z";
const NS: &str = "http://aegis.gastown.local/ontology/";
const REG: &str = "http://aegis.gastown.local/ontology/yubikey";
const VERDICT: &str = "http://aegis.gastown.local/ontology/verdict1";

fn datum(store: &mut Store, e: &str, p: &str, value: Value, from: &str) -> Datum {
    Datum {
        entity: store.intern(e).unwrap(),
        attribute: store.intern(p).unwrap(),
        value,
        valid_from: from.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn register(store: &mut Store, key: &SkKey, from: &str) {
    let class = Value::Ref(store.intern(&format!("{NS}VerifierRegistration")).unwrap());
    let facts = [
        (
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".to_string(),
            class,
        ),
        (format!("{NS}verifier"), Value::Str("approver".into())),
        (format!("{NS}attests"), Value::Str("human-approval".into())),
        (
            format!("{NS}signatureScheme"),
            Value::Str("sshsig-sk-ed25519".into()),
        ),
        (format!("{NS}publicKey"), Value::Str(key.openssh_line())),
    ];
    let datums: Vec<Datum> = facts
        .into_iter()
        .map(|(p, v)| datum(store, REG, &p, v, from))
        .collect();
    store.transact(&datums, from, None, None).unwrap();
}

/// Record `signature` on the stored verdict at `ts`.
fn record(store: &mut Store, signature: &str, ts: &str) {
    let d = datum(
        store,
        VERDICT,
        &format!("{NS}signature"),
        Value::Str(signature.into()),
        ts,
    );
    store.transact(&[d], ts, None, None).unwrap();
}

fn revoke(store: &mut Store, key: &SkKey, until: &str) {
    let e = store.lookup(REG).unwrap().unwrap();
    let a = store.lookup(&format!("{NS}publicKey")).unwrap();
    let (_, n) = store
        .retract_triples(
            e,
            a,
            Some(&Value::Str(key.openssh_line())),
            until,
            None,
            false,
            None,
        )
        .unwrap();
    assert_eq!(n, 1);
}

fn verdict(signature: &str) -> serde_json::Value {
    json!({
        "predicate_id": "human-approval",
        "target_ref": "http://example.org/decision/1",
        "outcome": "satisfied",
        "evidence_hash": "sha256:00ff",
        "tier": "human",
        "verifier": "approver",
        "scheme": "sshsig-sk-ed25519",
        "signature": signature,
    })
}

fn setup() -> (Store, SkKey, String) {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().hardware_verdict_schemes = true;
    let key = SkKey::new();
    let sig = key.sign("quipu-verdict", MESSAGE, 0x05, 3);
    (store, key, sig)
}

#[test]
fn a_key_revoked_after_the_signature_still_verifies_what_it_signed() {
    let (mut store, key, sig) = setup();
    register(&mut store, &key, T0);
    record(&mut store, &sig, T1);
    revoke(&mut store, &key, T2);

    let mut recorded = verdict(&sig);
    recorded["verdict"] = json!(VERDICT);
    let out = tool_verdict_verify(&store, &recorded).unwrap();
    assert_eq!(out["trusted"], true, "{out:#}");
    assert_eq!(out["as_of"]["basis"], "recorded");

    // CONTROL: the revocation is real. Read now, the key is gone.
    let now = tool_verdict_verify(&store, &verdict(&sig)).unwrap();
    assert_eq!(now["as_of"]["basis"], "now");
    assert_eq!(now["trusted"], false, "{now:#}");
    assert_eq!(now["verifier_registered"], false, "{now:#}");
}

#[test]
fn a_key_enrolled_after_the_signature_cannot_vouch_for_it() {
    let (mut store, key, sig) = setup();
    record(&mut store, &sig, T1);
    register(&mut store, &key, T2);

    let mut recorded = verdict(&sig);
    recorded["verdict"] = json!(VERDICT);
    let out = tool_verdict_verify(&store, &recorded).unwrap();
    assert_eq!(out["trusted"], false, "{out:#}");
    assert_eq!(out["verifier_registered"], false, "{out:#}");
    assert_eq!(out["verifier_authorized"], false, "{out:#}");

    // CONTROL: the same verdict verifies now, so the refusal above is the
    // as-of rule and not a broken signature.
    let now = tool_verdict_verify(&store, &verdict(&sig)).unwrap();
    assert_eq!(now["trusted"], true, "{now:#}");
}

#[test]
fn a_caller_supplied_instant_is_a_what_if_on_the_hardware_path_too() {
    let (mut store, key, sig) = setup();
    register(&mut store, &key, T0);
    record(&mut store, &sig, T1);
    let w = Witness::of_fact(&store, VERDICT, &format!("{NS}signature"), &sig)
        .unwrap()
        .unwrap();

    let mut what_if = verdict(&sig);
    what_if["tx"] = json!(w.tx);
    what_if["signed_at"] = json!(w.at);
    let out = tool_verdict_verify(&store, &what_if).unwrap();
    assert_eq!(out["as_of"]["basis"], "caller-supplied");
    assert_eq!(out["trusted"], false, "{out:#}");
    assert_eq!(out["would_verify_as_of_supplied_instant"], true, "{out:#}");
}
