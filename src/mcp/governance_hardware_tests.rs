//! End-to-end: hardware verdict schemes through `quipu_verdict_verify` and the
//! registration write gate, including the off-by-default safety gate.

use std::sync::Arc;

use serde_json::json;

use super::super::governance::{tool_policy_check, tool_verdict_verify};
use crate::store::Store;
use crate::verdict_schemes::sshsig_tests::{MESSAGE, SkKey};
use crate::verdict_schemes::webauthn_tests::{Authenticator, Ceremony, ORIGIN, RP_ID};

const PREFIX: &str = "@prefix a: <http://aegis.gastown.local/ontology/> .\n";

fn ingest(store: &mut Store, ttl: &str) -> crate::error::Result<()> {
    crate::rdf::ingest_rdf(
        store,
        format!("{PREFIX}{ttl}").as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .map(|_| ())
}

fn enabled_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().hardware_verdict_schemes = true;
    store
}

/// The verdict whose canonical message is [`MESSAGE`].
fn verdict() -> serde_json::Value {
    json!({
        "predicate_id": "human-approval",
        "target_ref": "http://example.org/decision/1",
        "outcome": "satisfied",
        "evidence_hash": "sha256:00ff",
        "tier": "human",
        "verifier": "approver",
    })
}

fn webauthn_registration(auth: &Authenticator, extra: &str) -> String {
    format!(
        "a:passkey a a:VerifierRegistration ; a:verifier \"approver\" ; \
         a:attests \"human-approval\" ; a:signatureScheme \"{}\" ; \
         a:publicKey \"{}\" ; a:webauthnRpId \"{RP_ID}\" ; \
         a:webauthnOrigin \"{ORIGIN}\" {extra}.\n",
        auth.scheme().tag(),
        auth.cose_key_b64url()
    )
}

fn webauthn_verdict(auth: &Authenticator, ceremony: &Ceremony) -> serde_json::Value {
    let (ad, cdj, sig) = auth.sign(ceremony).wire();
    let mut v = verdict();
    v["scheme"] = json!(auth.scheme().tag());
    v["authenticator_data"] = json!(ad);
    v["client_data_json"] = json!(cdj);
    v["signature"] = json!(sig);
    v
}

#[test]
fn hardware_registrations_are_refused_while_the_gate_is_off() {
    let mut store = Store::open_in_memory().unwrap();
    assert!(
        !store.governance_config().hardware_verdict_schemes,
        "the gate must default to off"
    );
    let auth = Authenticator::es256();
    let err = ingest(&mut store, &webauthn_registration(&auth, ""))
        .unwrap_err()
        .to_string();
    assert!(err.contains("disabled"), "{err}");

    let key = SkKey::new();
    let ttl = format!(
        "a:yubikey a a:VerifierRegistration ; a:verifier \"approver\" ; \
         a:signatureScheme \"sshsig-sk-ed25519\" ; a:publicKey \"{}\" .\n",
        key.openssh_line()
    );
    assert!(ingest(&mut store, &ttl).is_err());

    // The control: an explicit ed25519 scheme and an ordinary write still land.
    ingest(
        &mut store,
        "a:r1 a a:VerifierRegistration ; a:verifier \"quipu\" ; a:signatureScheme \"ed25519\" .\n",
    )
    .unwrap();

    // And with the gate on, the same hardware registration lands.
    store.governance_config_mut().hardware_verdict_schemes = true;
    ingest(&mut store, &webauthn_registration(&auth, "")).unwrap();
    ingest(&mut store, &ttl).unwrap();
}

#[test]
fn an_unknown_scheme_is_refused_even_with_the_gate_on() {
    let mut store = enabled_store();
    let err = ingest(
        &mut store,
        "a:r a a:VerifierRegistration ; a:verifier \"x\" ; a:signatureScheme \"rsa-pkcs1\" .\n",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("unknown signature scheme"), "{err}");
}

#[test]
fn webauthn_verdict_is_refused_while_off_and_trusted_when_on() {
    for auth in [Authenticator::es256(), Authenticator::ed25519()] {
        let mut store = enabled_store();
        ingest(&mut store, &webauthn_registration(&auth, "")).unwrap();
        let v = webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE));

        let ok = tool_verdict_verify(&store, &v).unwrap();
        assert_eq!(ok["trusted"], true, "{ok:#}");
        assert_eq!(ok["scheme"], auth.scheme().tag());
        assert_eq!(ok["user_verified"], true);
        assert_eq!(ok["sign_count"], 7);

        // Gate off: refused outright, with the operator-facing message.
        store.governance_config_mut().hardware_verdict_schemes = false;
        let err = tool_verdict_verify(&store, &v).unwrap_err().to_string();
        assert!(err.contains("hardware_verdict_schemes"), "{err}");

        // Tamper the verdict's outcome: the challenge no longer matches.
        store.governance_config_mut().hardware_verdict_schemes = true;
        let mut forged = v.clone();
        forged["outcome"] = json!("unsatisfied");
        let bad = tool_verdict_verify(&store, &forged).unwrap();
        assert_eq!(bad["trusted"], false, "{bad:#}");
        assert!(bad["reasons"].to_string().contains("challenge"), "{bad:#}");
    }
}

#[test]
fn sshsig_sk_verdict_is_trusted_when_on() {
    let mut store = enabled_store();
    let key = SkKey::new();
    ingest(
        &mut store,
        &format!(
            "a:yubikey a a:VerifierRegistration ; a:verifier \"approver\" ; \
             a:attests \"human-approval\" ; a:signatureScheme \"sshsig-sk-ed25519\" ; \
             a:publicKey \"{}\" .\n",
            key.openssh_line()
        ),
    )
    .unwrap();
    let mut v = verdict();
    v["scheme"] = json!("sshsig-sk-ed25519");
    v["signature"] = json!(key.sign("quipu-verdict", MESSAGE, 0x05, 3));
    let ok = tool_verdict_verify(&store, &v).unwrap();
    assert_eq!(ok["trusted"], true, "{ok:#}");
    assert_eq!(ok["user_present"], true);
    assert_eq!(ok["sign_count"], 3);

    v["signature"] = json!(key.sign("git", MESSAGE, 0x05, 4));
    let bad = tool_verdict_verify(&store, &v).unwrap();
    assert_eq!(bad["trusted"], false);
    assert!(bad["reasons"].to_string().contains("namespace"), "{bad:#}");
}

#[test]
fn a_verdict_cannot_choose_a_scheme_its_registration_does_not_declare() {
    // The verifier's only registration is an sk key; a verdict claiming
    // WebAuthn finds no registration of that scheme to verify against.
    let mut store = enabled_store();
    let key = SkKey::new();
    ingest(
        &mut store,
        &format!(
            "a:yubikey a a:VerifierRegistration ; a:verifier \"approver\" ; \
             a:attests \"human-approval\" ; a:signatureScheme \"sshsig-sk-ed25519\" ; \
             a:publicKey \"{}\" .\n",
            key.openssh_line()
        ),
    )
    .unwrap();
    let auth = Authenticator::es256();
    let out = tool_verdict_verify(
        &store,
        &webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE)),
    )
    .unwrap();
    assert_eq!(out["trusted"], false);
    assert_eq!(out["verifier_registered"], false, "{out:#}");
}

#[test]
fn trust_requires_the_verifying_registration_itself_to_attest_the_predicate() {
    let mut store = enabled_store();
    let auth = Authenticator::es256();
    // The passkey registration attests something else; a SECOND registration
    // under the same verifier name grants human-approval. The grant must not
    // be borrowed.
    let ttl = webauthn_registration(&auth, "").replace(
        "a:attests \"human-approval\"",
        "a:attests \"other-predicate\"",
    );
    ingest(&mut store, &ttl).unwrap();
    ingest(
        &mut store,
        "a:grant a a:VerifierRegistration ; a:verifier \"approver\" ; a:attests \"human-approval\" .\n",
    )
    .unwrap();
    let out = tool_verdict_verify(
        &store,
        &webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE)),
    )
    .unwrap();
    assert_eq!(out["signature_valid"], true, "{out:#}");
    assert_eq!(out["verifier_authorized"], true);
    assert_eq!(out["registration_authorized"], false);
    assert_eq!(out["trusted"], false);
}

#[test]
fn a_recorded_sign_count_refuses_a_replayed_or_cloned_assertion() {
    let mut store = enabled_store();
    let auth = Authenticator::es256();
    ingest(
        &mut store,
        &webauthn_registration(&auth, "; a:signCount 10 "),
    )
    .unwrap();
    let out = tool_verdict_verify(
        &store,
        &webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE)),
    )
    .unwrap(); // presents 7
    assert_eq!(out["trusted"], false);
    assert!(out["reasons"].to_string().contains("counter"), "{out:#}");

    let mut c = Ceremony::for_message(MESSAGE);
    c.sign_count = 11;
    let ok = tool_verdict_verify(&store, &webauthn_verdict(&auth, &c)).unwrap();
    assert_eq!(ok["trusted"], true, "{ok:#}");
    assert_eq!(ok["sign_count"], 11);
}

#[test]
fn a_webauthn_verdict_missing_its_parts_is_an_error() {
    let mut store = enabled_store();
    let auth = Authenticator::es256();
    ingest(&mut store, &webauthn_registration(&auth, "")).unwrap();
    let mut v = webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE));
    v.as_object_mut().unwrap().remove("client_data_json");
    assert!(tool_verdict_verify(&store, &v).is_err());
}

#[test]
fn ed25519_verdicts_are_unaffected_by_a_hardware_registration_beside_them() {
    // The hardware registration is written FIRST under the same verifier name,
    // so a v1 lookup that ignored the scheme would pick its (non-ed25519) key.
    let mut store = enabled_store();
    let auth = Authenticator::es256();
    ingest(
        &mut store,
        &webauthn_registration(&auth, "").replace("\"approver\"", "\"quipu\""),
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let id = crate::signing::SigningIdentity::load(&dir.path().join("k.pk8"), "quipu").unwrap();
    let pubkey = id.public_key_hex();
    store.set_signing_identity(Arc::new(id));
    ingest(
        &mut store,
        &format!(
            "a:reg a a:VerifierRegistration ; a:verifier \"quipu\" ; a:attests \"has-test\" ; \
             a:publicKey \"{pubkey}\" .\na:sym1 a a:CodeSymbol ; a:hasTest a:t1 .\n"
        ),
    )
    .unwrap();
    let v = tool_policy_check(
        &store,
        &json!({
            "claim": "PREFIX a: <http://aegis.gastown.local/ontology/> ASK { $target a:hasTest ?t }",
            "target": "http://aegis.gastown.local/ontology/sym1",
            "predicate_id": "has-test"
        }),
    )
    .unwrap();
    let ok = tool_verdict_verify(&store, &v).unwrap();
    assert_eq!(ok["trusted"], true, "{ok:#}");

    // Naming the default scheme explicitly takes the same v1 path.
    let mut explicit = v.clone();
    explicit["scheme"] = json!("ed25519");
    assert_eq!(
        tool_verdict_verify(&store, &explicit).unwrap()["trusted"],
        true
    );

    // And with the gate OFF, ed25519 verification still works.
    store.governance_config_mut().hardware_verdict_schemes = false;
    assert_eq!(tool_verdict_verify(&store, &v).unwrap()["trusted"], true);
}

#[test]
fn an_unknown_verdict_scheme_is_an_error() {
    let store = enabled_store();
    let mut v = verdict();
    v["scheme"] = json!("rsa-pkcs1");
    v["signature"] = json!("00");
    assert!(tool_verdict_verify(&store, &v).is_err());
}

#[test]
fn an_ambiguous_registration_is_refused() {
    // Two origins on one registration: neither pairing may verify.
    let mut store = enabled_store();
    let auth = Authenticator::es256();
    ingest(
        &mut store,
        &webauthn_registration(&auth, "; a:webauthnOrigin \"https://other.example.org\" "),
    )
    .unwrap();
    let out = tool_verdict_verify(
        &store,
        &webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE)),
    )
    .unwrap();
    assert_eq!(out["trusted"], false, "{out:#}");
    assert!(out["reasons"].to_string().contains("ambiguous"), "{out:#}");
}

#[test]
fn an_unauthorized_registration_does_not_mask_an_authorizing_one() {
    // The same key enrolled twice: once without the grant, once with it.
    // Whichever is read first, the verdict is trusted.
    let mut store = enabled_store();
    let auth = Authenticator::es256();
    let unauthorized = webauthn_registration(&auth, "")
        .replace("a:passkey ", "a:aaa-first ")
        .replace("\"human-approval\"", "\"other-predicate\"");
    ingest(&mut store, &unauthorized).unwrap();
    ingest(
        &mut store,
        &webauthn_registration(&auth, "").replace("a:passkey ", "a:zzz-second "),
    )
    .unwrap();
    let out = tool_verdict_verify(
        &store,
        &webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE)),
    )
    .unwrap();
    assert_eq!(out["trusted"], true, "{out:#}");
    assert!(
        out["registration"]
            .as_str()
            .unwrap()
            .ends_with("zzz-second")
    );
}

/// aegis-9dpcta dual-read: a hardware registration whose class, verifier,
/// attests and publicKey use the Quechua namespace is trusted exactly like a
/// legacy one. signatureScheme and the webauthn terms have no published twin
/// yet, so they stay legacy (`a:`) here, as they would in a real mixed store.
#[test]
fn a_quechua_namespace_registration_is_trusted_like_a_legacy_one() {
    let auth = Authenticator::es256();
    let mut store = enabled_store();
    let legacy = webauthn_registration(&auth, "");
    let quechua = legacy
        .replace("a a:VerifierRegistration", "a q:VerifierRegistration")
        .replace("a:verifier", "q:verifier")
        .replace("a:attests", "q:attests")
        .replace("a:publicKey", "q:publicKey");
    assert_ne!(
        legacy, quechua,
        "control: the fixture really changed namespace"
    );
    ingest(
        &mut store,
        &format!("@prefix q: <https://scbrown.github.io/quechua/ns#> .\n{quechua}"),
    )
    .unwrap();
    let v = webauthn_verdict(&auth, &Ceremony::for_message(MESSAGE));
    let ok = tool_verdict_verify(&store, &v).unwrap();
    assert_eq!(ok["trusted"], true, "{ok:#}");
}
