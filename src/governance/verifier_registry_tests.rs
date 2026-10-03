//! Bitemporal verifier registry tests (signing-plane S1). Size-exempt.

use super::*;
use crate::store::Datum;
use crate::types::Op;

const T0: &str = "2026-01-01T00:00:00Z";
const T1: &str = "2026-01-10T00:00:00Z";
const T2: &str = "2026-01-20T00:00:00Z";
const T3: &str = "2026-01-30T00:00:00Z";
const P: &str = "http://ex/policy";

fn assert_fact(store: &mut Store, e: &str, p: &str, v: Value, from: &str, ts: &str) -> i64 {
    let datum = Datum {
        entity: store.intern(e).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: from.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[datum], ts, None, None).unwrap()
}

fn ns(name: &str) -> String {
    format!("{DEFAULT_BASE_NS}{name}")
}

/// A registration for `verifier` holding `key`, attesting `attests`.
fn register(store: &mut Store, reg: &str, verifier: &str, key: &str, attests: &str, from: &str) {
    let class = Value::Ref(store.intern(&ns("VerifierRegistration")).unwrap());
    let datums = [
        (RDF_TYPE.to_string(), class),
        (ns("verifier"), Value::Str(verifier.into())),
        (ns("publicKey"), Value::Str(key.into())),
        (ns("attests"), Value::Str(attests.into())),
    ]
    .into_iter()
    .map(|(p, v)| Datum {
        entity: store.intern(reg).unwrap(),
        attribute: store.intern(&p).unwrap(),
        value: v,
        valid_from: from.to_string(),
        valid_to: None,
        op: Op::Assert,
    })
    .collect::<Vec<_>>();
    store.transact(&datums, from, None, None).unwrap();
}

/// Close `key` on `reg` with `valid_to = until`, in a transaction stamped `ts`.
fn close_key(store: &mut Store, reg: &str, key: &str, until: &str) {
    let e = store.lookup(reg).unwrap().unwrap();
    let a = store.lookup(&ns("publicKey")).unwrap();
    let (_, n) = store
        .retract_triples(
            e,
            a,
            Some(&Value::Str(key.into())),
            until,
            None,
            false,
            None,
        )
        .unwrap();
    assert_eq!(n, 1, "the key fact must exist to be closed");
}

/// Record a signature on `subject` at `ts`, returning the witness the store gives it.
fn sign(store: &mut Store, subject: &str, sig: &str, ts: &str) -> Witness {
    assert_fact(
        store,
        subject,
        &ns("signature"),
        Value::Str(sig.into()),
        ts,
        ts,
    );
    Witness::of_fact(store, subject, &ns("signature"), sig)
        .unwrap()
        .unwrap()
}

fn keys(store: &Store, w: &Witness) -> Vec<String> {
    registered_keys(store, "stiwi", Some(P), w, Scope::Root).unwrap()
}

#[test]
fn rotation_keeps_what_the_old_key_signed_and_refuses_it_afterwards() {
    let mut store = Store::open_in_memory().unwrap();
    register(&mut store, "http://ex/reg", "stiwi", "AAAA", P, T0);
    let before = sign(&mut store, "http://ex/v1", "sig1", T1);

    // close-then-insert on the same registration
    close_key(&mut store, "http://ex/reg", "AAAA", T2);
    assert_fact(
        &mut store,
        "http://ex/reg",
        &ns("publicKey"),
        Value::Str("BBBB".into()),
        T2,
        T2,
    );
    let after = sign(&mut store, "http://ex/v2", "sig2", T3);

    assert_eq!(
        keys(&store, &before),
        vec!["AAAA"],
        "recorded before the rotation"
    );
    assert_eq!(
        keys(&store, &after),
        vec!["BBBB"],
        "recorded after: only the new key"
    );
    assert_eq!(keys(&store, &Witness::now()), vec!["BBBB"]);
}

#[test]
fn a_revocation_can_reach_back_to_the_compromise() {
    let mut store = Store::open_in_memory().unwrap();
    register(&mut store, "http://ex/reg", "stiwi", "AAAA", P, T0);
    let honest = sign(&mut store, "http://ex/v1", "sig1", T1);
    let suspect = sign(&mut store, "http://ex/v2", "sig2", T3);
    assert_eq!(
        keys(&store, &suspect),
        vec!["AAAA"],
        "trusted until we learn otherwise"
    );

    // Revoked later, effective from T2: the key was compromised at T2.
    close_key(&mut store, "http://ex/reg", "AAAA", T2);
    assert_eq!(
        keys(&store, &honest),
        vec!["AAAA"],
        "before the compromise: still good"
    );
    assert!(
        keys(&store, &suspect).is_empty(),
        "after the compromise: distrusted"
    );
}

#[test]
fn a_revoked_key_cannot_back_date_its_way_in() {
    let mut store = Store::open_in_memory().unwrap();
    register(&mut store, "http://ex/reg", "stiwi", "AAAA", P, T0);
    close_key(&mut store, "http://ex/reg", "AAAA", T2);
    // The holder of the revoked key writes a signature whose transaction is
    // stamped T1, inside the key's old validity. The tx that recorded it came
    // after the revocation, and that ordering is the store's, not the writer's.
    let forged = sign(&mut store, "http://ex/v1", "sig1", T1);
    assert!(keys(&store, &forged).is_empty());
}

#[test]
fn as_of_tx_replays_what_the_store_knew_then() {
    let mut store = Store::open_in_memory().unwrap();
    let early = sign(&mut store, "http://ex/v0", "sig0", T1);
    register(&mut store, "http://ex/reg", "stiwi", "AAAA", P, T0);
    // valid-time covers T1, but at that tx the registration did not exist yet
    assert!(keys(&store, &early).is_empty());
    let replay_now = Witness {
        tx: None,
        at: T1.into(),
    };
    assert_eq!(
        keys(&store, &replay_now),
        vec!["AAAA"],
        "valid-time alone says yes"
    );
}

#[test]
fn the_key_and_the_scope_must_be_the_same_registration() {
    let mut store = Store::open_in_memory().unwrap();
    // AAAA may attest only another policy; P is granted to BBBB.
    register(
        &mut store,
        "http://ex/reg_a",
        "stiwi",
        "AAAA",
        "http://ex/other",
        T0,
    );
    register(&mut store, "http://ex/reg_b", "stiwi", "BBBB", P, T0);
    assert_eq!(keys(&store, &Witness::now()), vec!["BBBB"]);
    let all = registered_keys(&store, "stiwi", None, &Witness::now(), Scope::Root).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn activation_and_expiry_follow_the_valid_interval() {
    let mut store = Store::open_in_memory().unwrap();
    register(
        &mut store,
        "http://ex/reg_future",
        "stiwi",
        "FUTR",
        P,
        "2999-01-01T00:00:00Z",
    );
    assert!(keys(&store, &Witness::now()).is_empty(), "not active yet");

    register(&mut store, "http://ex/reg", "stiwi", "AAAA", P, T0);
    close_key(&mut store, "http://ex/reg", "AAAA", "2999-01-01T00:00:00Z");
    assert_eq!(
        keys(&store, &Witness::now()),
        vec!["AAAA"],
        "a scheduled expiry has not arrived"
    );
}

#[test]
fn scope_root_ignores_named_graph_registrations() {
    let mut store = Store::open_in_memory().unwrap();
    let g = store.graph_create("http://ex/identity").unwrap();
    let class = Value::Ref(store.intern(&ns("VerifierRegistration")).unwrap());
    let datums: Vec<Datum> = [
        (RDF_TYPE.to_string(), class),
        (ns("verifier"), Value::Str("stiwi".into())),
        (ns("publicKey"), Value::Str("NAMED".into())),
        (ns("attests"), Value::Str(P.into())),
    ]
    .into_iter()
    .map(|(p, v)| Datum {
        entity: store.intern("http://ex/reg_named").unwrap(),
        attribute: store.intern(&p).unwrap(),
        value: v,
        valid_from: T0.to_string(),
        valid_to: None,
        op: Op::Assert,
    })
    .collect();
    store.transact_to_graph(&datums, T0, None, None, g).unwrap();
    assert!(keys(&store, &Witness::now()).is_empty());
    let all = registered_keys(&store, "stiwi", Some(P), &Witness::now(), Scope::AllGraphs).unwrap();
    assert_eq!(all, vec!["NAMED"]);
}
