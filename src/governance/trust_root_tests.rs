//! Trust-root gate and bootstrap tests (aegis-kzt0ql.9.4). Size-exempt.
//!
//! Every key here is a TEST key enrolled through the real bootstrap and real
//! signed amendments. There is no bypass to reach for.

use super::*;
use crate::governance::verifier_registry::{Scope, Witness, registered_keys};

const TS: &str = "2026-01-01T00:00:00Z";
const POLICY: &str = "http://ex/policy/wire-funds";

fn keypair() -> ring::signature::Ed25519KeyPair {
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

fn pk(kp: &ring::signature::Ed25519KeyPair) -> String {
    crate::signing::public_key_hex(kp)
}

fn d(store: &Store, s: &str, p: &str, v: Value, op: Op) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op,
    }
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

fn nonce() -> String {
    let mut b = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).unwrap();
    hex::encode(b)
}

/// Enrol `kp` as the first human key "stiwi", attesting POLICY too.
fn enrol(store: &mut Store, kp: &ring::signature::Ed25519KeyPair) -> Enrolled {
    let msg = bootstrap_message(&store.store_id().unwrap(), "stiwi", &pk(kp));
    let pop = crate::signing::sign_hex(kp, &msg);
    bootstrap(store, "stiwi", &pk(kp), &[POLICY.to_string()], &pop, TS).unwrap()
}

/// Write `change` together with an amendment for `registration`, signed by
/// `signer` as `signer_name`.
fn amend_with(
    store: &mut Store,
    signer: &ring::signature::Ed25519KeyPair,
    signer_name: &str,
    registration: &str,
    change: Vec<Datum>,
    nonce: &str,
    digest: Option<&str>,
) -> crate::error::Result<i64> {
    let digest = match digest {
        Some(x) => x.to_string(),
        None => digest_after(store, registration, &change).unwrap(),
    };
    let msg = amendment_message(&store.store_id().unwrap(), registration, &digest, nonce);
    let sig = crate::signing::sign_hex(signer, &msg);
    let a = format!("http://ex/amendment/{}", self::nonce());
    let mut datums = change;
    datums.extend([
        d(
            store,
            &a,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("RegistryAmendment")).unwrap()),
            Op::Assert,
        ),
        d(
            store,
            &a,
            &ns("amends"),
            Value::Ref(store.intern(registration).unwrap()),
            Op::Assert,
        ),
        d(store, &a, &ns("amendedDigest"), s(&digest), Op::Assert),
        d(
            store,
            &a,
            &ns("amendmentSigner"),
            s(signer_name),
            Op::Assert,
        ),
        d(store, &a, &ns("amendmentNonce"), s(nonce), Op::Assert),
        d(store, &a, &ns("amendmentSignature"), s(&sig), Op::Assert),
    ]);
    store.transact(&datums, TS, Some("agent"), Some("episode"))
}

fn amend(
    store: &mut Store,
    signer: &ring::signature::Ed25519KeyPair,
    registration: &str,
    change: Vec<Datum>,
) -> crate::error::Result<i64> {
    amend_with(store, signer, "stiwi", registration, change, &nonce(), None)
}

/// A new human registration's datums.
fn human_registration(store: &Store, iri: &str, verifier: &str, key: &str) -> Vec<Datum> {
    vec![
        d(
            store,
            iri,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("VerifierRegistration")).unwrap()),
            Op::Assert,
        ),
        d(store, iri, &ns("verifier"), s(verifier), Op::Assert),
        d(store, iri, &ns("publicKey"), s(key), Op::Assert),
        d(store, iri, &ns("attests"), s(POLICY), Op::Assert),
        d(store, iri, &ns("attests"), s(TRUST_ROOT_POLICY), Op::Assert),
        d(store, iri, TRUST_TIER, s(HUMAN_TIER), Op::Assert),
    ]
}

fn human_keys(store: &Store) -> Vec<String> {
    registered_keys(
        store,
        "stiwi",
        Some(POLICY),
        &Witness::now(),
        Scope::HumanTier,
    )
    .unwrap()
}

fn refused(r: crate::error::Result<i64>) -> String {
    match r {
        Err(Error::PolicyDenied(m)) => m,
        other => panic!("expected a trust-root refusal, got {other:?}"),
    }
}

#[test]
fn a_forged_human_registration_is_refused() {
    // The baseline: this landed at tx 1 before the gate.
    let mut store = Store::open_in_memory().unwrap();
    let mallory = keypair();
    let datums = human_registration(&store, "http://ex/reg/forged", "stiwi", &pk(&mallory));
    let m = refused(store.transact(&datums, TS, Some("mallory"), Some("episode")));
    assert!(m.contains("trust root"), "{m}");
    assert!(human_keys(&store).is_empty());
}

#[test]
fn an_agent_registration_still_lands_and_cannot_verify_a_human_decision() {
    let mut store = Store::open_in_memory().unwrap();
    let mallory = keypair();
    let mut datums = human_registration(&store, "http://ex/reg/agent", "stiwi", &pk(&mallory));
    datums.pop(); // no trustTier: an ordinary agent-tier registration
    store
        .transact(&datums, TS, Some("mallory"), Some("episode"))
        .unwrap();
    assert!(
        human_keys(&store).is_empty(),
        "agent tier never verifies a human decision"
    );
    assert_eq!(
        registered_keys(&store, "stiwi", Some(POLICY), &Witness::now(), Scope::Root).unwrap(),
        vec![pk(&mallory)],
        "control: the agent registration IS there, the tier filter is what excludes it"
    );
}

#[test]
fn bootstrap_enrols_once_with_fingerprint_and_never_again() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    let e = enrol(&mut store, &stiwi);
    assert_eq!(human_keys(&store), vec![pk(&stiwi)]);
    assert_eq!(e.fingerprint, fingerprint(&pk(&stiwi)).unwrap());
    let record = strings(
        &store,
        &ns(&format!(
            "trust_root_bootstrap_{}",
            &e.registration[e.registration.len() - 16..]
        )),
        &ns("keyFingerprint"),
    )
    .unwrap();
    assert_eq!(
        record,
        vec![e.fingerprint.clone()],
        "the ceremony record carries the fingerprint"
    );

    let other = keypair();
    let pop = crate::signing::sign_hex(
        &other,
        &bootstrap_message(&store.store_id().unwrap(), "stiwi", &pk(&other)),
    );
    let err = bootstrap(&mut store, "stiwi", &pk(&other), &[], &pop, TS).unwrap_err();
    assert!(err.to_string().contains("already been enrolled"), "{err}");
}

#[test]
fn bootstrap_stays_closed_after_the_human_key_is_revoked() {
    // EVER existed, from the full history: revoking every human key must not
    // reopen the bootstrap.
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    let e = enrol(&mut store, &stiwi);
    let revoke = vec![d(
        &store,
        &e.registration,
        TRUST_TIER,
        s(HUMAN_TIER),
        Op::Retract,
    )];
    amend(&mut store, &stiwi, &e.registration, revoke).unwrap();
    assert!(human_keys(&store).is_empty());
    let other = keypair();
    let pop = crate::signing::sign_hex(
        &other,
        &bootstrap_message(&store.store_id().unwrap(), "stiwi", &pk(&other)),
    );
    assert!(bootstrap(&mut store, "stiwi", &pk(&other), &[], &pop, TS).is_err());
}

#[test]
fn bootstrap_requires_proof_of_possession() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    let wrong = crate::signing::sign_hex(
        &keypair(),
        &bootstrap_message(&store.store_id().unwrap(), "stiwi", &pk(&stiwi)),
    );
    assert!(bootstrap(&mut store, "stiwi", &pk(&stiwi), &[], &wrong, TS).is_err());
    // Signed for another store: refused too.
    let elsewhere = crate::signing::sign_hex(
        &stiwi,
        &bootstrap_message("another-store", "stiwi", &pk(&stiwi)),
    );
    assert!(bootstrap(&mut store, "stiwi", &pk(&stiwi), &[], &elsewhere, TS).is_err());
    assert!(
        !ever_bootstrapped(&store).unwrap(),
        "a refused bootstrap writes nothing"
    );
}

#[test]
fn a_second_device_is_an_amendment_signed_by_the_first() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let yubikey = keypair();
    let reg = "http://ex/reg/yubikey";
    let datums = human_registration(&store, reg, "stiwi", &pk(&yubikey));
    amend(&mut store, &stiwi, reg, datums).unwrap();
    let keys = human_keys(&store);
    assert!(
        keys.contains(&pk(&yubikey)) && keys.contains(&pk(&stiwi)),
        "{keys:?}"
    );
}

#[test]
fn a_replayed_amendment_nonce_is_refused() {
    // The real replay: revoke a device, then re-submit its captured enrolment
    // amendment unchanged. The digest matches and the signature is genuine;
    // only the spent nonce stops it.
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let old = keypair();
    let reg = "http://ex/reg/old-phone";
    let n = nonce();
    let enrolment = human_registration(&store, reg, "stiwi", &pk(&old));
    let digest = digest_after(&store, reg, &enrolment).unwrap();
    amend_with(
        &mut store,
        &stiwi,
        "stiwi",
        reg,
        enrolment.clone(),
        &n,
        Some(&digest),
    )
    .unwrap();
    let revoke = vec![d(&store, reg, TRUST_TIER, s(HUMAN_TIER), Op::Retract)];
    amend(&mut store, &stiwi, reg, revoke).unwrap();
    assert_eq!(human_keys(&store), vec![pk(&stiwi)]);
    let m = refused(amend_with(
        &mut store,
        &stiwi,
        "stiwi",
        reg,
        enrolment,
        &n,
        Some(&digest),
    ));
    assert!(m.contains("already used"), "{m}");
    assert_eq!(
        human_keys(&store),
        vec![pk(&stiwi)],
        "the revoked key stays revoked"
    );
}

#[test]
fn unsigned_edits_and_revocations_are_refused_signed_ones_accepted() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    let e = enrol(&mut store, &stiwi);
    let mallory = keypair();
    // An extra key on the human registration, unsigned.
    let add = vec![d(
        &store,
        &e.registration,
        &ns("publicKey"),
        s(&pk(&mallory)),
        Op::Assert,
    )];
    refused(store.transact(&add, TS, Some("mallory"), Some("episode")));
    // The same edit in a NAMED graph does not reach the human tier at all.
    let g = store.intern("http://ex/graph/other").unwrap();
    store
        .transact_to_graph(&add, TS, Some("mallory"), Some("episode"), g)
        .unwrap();
    assert_eq!(human_keys(&store), vec![pk(&stiwi)]);
    // An unsigned revocation (DoS) is refused.
    let revoke = vec![d(
        &store,
        &e.registration,
        TRUST_TIER,
        s(HUMAN_TIER),
        Op::Retract,
    )];
    refused(store.transact(&revoke, TS, Some("mallory"), Some("episode")));
    assert_eq!(human_keys(&store), vec![pk(&stiwi)]);
    // Signed by the enrolled key: accepted.
    amend(&mut store, &stiwi, &e.registration, revoke).unwrap();
    assert!(human_keys(&store).is_empty());
}

#[test]
fn promoting_an_agent_registration_by_adding_the_marker_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    let mallory = keypair();
    let mut datums = human_registration(&store, "http://ex/reg/agent", "stiwi", &pk(&mallory));
    datums.pop();
    store
        .transact(&datums, TS, Some("mallory"), Some("episode"))
        .unwrap();
    let promote = vec![d(
        &store,
        "http://ex/reg/agent",
        TRUST_TIER,
        s(HUMAN_TIER),
        Op::Assert,
    )];
    refused(store.transact(&promote, TS, Some("mallory"), Some("episode")));
    // The marker asserted OUTSIDE ROOT does not count.
    let g = store.intern("http://ex/graph/identity").unwrap();
    store
        .transact_to_graph(&promote, TS, Some("mallory"), Some("episode"), g)
        .unwrap();
    assert!(human_keys(&store).is_empty());
}

#[test]
fn an_agent_tier_key_cannot_sign_an_amendment() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    // mallory holds an agent-tier registration that even attests trust-root.
    let mallory = keypair();
    let mut agent = human_registration(&store, "http://ex/reg/mallory", "mallory", &pk(&mallory));
    agent.pop();
    store
        .transact(&agent, TS, Some("mallory"), Some("episode"))
        .unwrap();
    let forged = human_registration(&store, "http://ex/reg/forged", "stiwi", &pk(&mallory));
    refused(amend_with(
        &mut store,
        &mallory,
        "mallory",
        "http://ex/reg/forged",
        forged,
        &nonce(),
        None,
    ));
}

#[test]
fn a_new_key_cannot_sign_its_own_enrolment() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let mallory = keypair();
    // The registration and the amendment in one write, signed by the NEW key.
    let reg = human_registration(&store, "http://ex/reg/self", "stiwi", &pk(&mallory));
    refused(amend_with(
        &mut store,
        &mallory,
        "stiwi",
        "http://ex/reg/self",
        reg,
        &nonce(),
        None,
    ));
}

#[test]
fn an_amendment_naming_the_wrong_digest_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let reg = human_registration(&store, "http://ex/reg/x", "stiwi", &pk(&keypair()));
    refused(amend_with(
        &mut store,
        &stiwi,
        "stiwi",
        "http://ex/reg/x",
        reg,
        &nonce(),
        Some("sha256:00"),
    ));
}

#[test]
fn a_signature_counter_update_is_not_an_amendment() {
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    let e = enrol(&mut store, &stiwi);
    let count = vec![d(
        &store,
        &e.registration,
        &ns("signCount"),
        Value::Int(7),
        Op::Assert,
    )];
    store
        .transact(&count, TS, Some("quipu"), Some("verdict"))
        .unwrap();
}

#[test]
fn the_gate_runs_while_verdicts_are_being_recorded() {
    let mut store = Store::open_in_memory().unwrap();
    store.recording_verdicts = true;
    let datums = human_registration(&store, "http://ex/reg/forged", "stiwi", &pk(&keypair()));
    refused(store.transact(&datums, TS, Some("quipu"), Some("verdict")));
}

#[test]
fn the_fingerprint_matches_ssh_keygen() {
    // `ssh-keygen -lf` on the OpenSSH public key holding these 32 bytes.
    assert_eq!(
        fingerprint("02fc1559d123cfde3aa3b1c65fdda59f11763b3873655598bdf5f7198340c1dc").unwrap(),
        "SHA256:YKSkl4o4DZwF+rQ3OnL/yP9sIxVW5yTX6TbvsjSh4uA"
    );
}

/// What bootstrap and an amendment write must satisfy the governance shapes.
#[cfg(feature = "shacl")]
#[test]
fn written_trust_root_records_conform_to_the_governance_shapes() {
    const SHAPES: &str = include_str!("../../shapes/governance.ttl");
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let reg = human_registration(&store, "http://ex/reg/yubikey", "stiwi", &pk(&keypair()));
    amend(&mut store, &stiwi, "http://ex/reg/yubikey", reg).unwrap();
    let ttl =
        String::from_utf8(crate::rdf::export_rdf(&store, oxrdfio::RdfFormat::Turtle).unwrap())
            .unwrap();
    let report = crate::shacl::validate_shapes(SHAPES, &ttl).unwrap();
    assert!(report.conforms, "{report:#?}");
    assert!(
        ttl.contains("RegistryAmendment") && ttl.contains("TrustRootBootstrap"),
        "control: the records were exported"
    );
}

#[test]
fn the_recorded_digest_must_match_what_was_signed() {
    // A genuine signature over the real post-write digest, but the amendment
    // RECORDS a different digest: refused, so the record never lies.
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let reg = "http://ex/reg/x";
    let change = human_registration(&store, reg, "stiwi", &pk(&keypair()));
    let real = digest_after(&store, reg, &change).unwrap();
    let n = nonce();
    let sig = crate::signing::sign_hex(
        &stiwi,
        &amendment_message(&store.store_id().unwrap(), reg, &real, &n),
    );
    let a = "http://ex/amendment/lying";
    let mut datums = change;
    datums.extend([
        d(
            &store,
            a,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("RegistryAmendment")).unwrap()),
            Op::Assert,
        ),
        d(
            &store,
            a,
            &ns("amends"),
            Value::Ref(store.intern(reg).unwrap()),
            Op::Assert,
        ),
        d(
            &store,
            a,
            &ns("amendedDigest"),
            s(&format!("sha256:{}", "0".repeat(64))),
            Op::Assert,
        ),
        d(&store, a, &ns("amendmentSigner"), s("stiwi"), Op::Assert),
        d(&store, a, &ns("amendmentNonce"), s(&n), Op::Assert),
        d(&store, a, &ns("amendmentSignature"), s(&sig), Op::Assert),
    ]);
    refused(store.transact(&datums, TS, Some("agent"), Some("episode")));
}

#[test]
fn the_bootstrap_path_cannot_add_to_a_populated_registry() {
    // Defence in depth under bootstrap()'s own history check: the gate admits a
    // bootstrap write only into an EMPTY human registry.
    let mut store = Store::open_in_memory().unwrap();
    let stiwi = keypair();
    enrol(&mut store, &stiwi);
    let reg = "http://ex/reg/second";
    let datums = human_registration(&store, reg, "stiwi", &pk(&keypair()));
    refused(store.transact_trust_root_bootstrap(reg, &datums, TS));
    assert_eq!(human_keys(&store), vec![pk(&stiwi)]);
}
