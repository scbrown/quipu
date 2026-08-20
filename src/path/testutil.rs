//! Shared fixture builders for the path module's tests.
//!
//! The vocabulary base here is deliberately NOT the default namespace — the
//! namespace is a parameter, and these tests prove the module honours that.

use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use super::PathVocab;

pub(crate) const TS: &str = "2026-08-20T00:00:00Z";
pub(crate) const TRAJ: &str = "http://ex/traj";
pub(crate) const PRODUCES: &str = "http://ex/v/produces";
pub(crate) const CONSUMED_BY: &str = "http://ex/v/consumedBy";

pub(crate) fn open_store() -> Store {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    Store::open(tmp.path().to_str().unwrap()).unwrap()
}

pub(crate) fn intern(store: &mut Store, iri: &str) -> i64 {
    store.intern(iri).expect("intern")
}

fn assert_value(store: &mut Store, s: &str, p: &str, v: Value) {
    let sid = intern(store, s);
    let pid = intern(store, p);
    let datum = Datum {
        entity: sid,
        attribute: pid,
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(&[datum], TS, None, Some("test"))
        .expect("transact");
}

pub(crate) fn edge(store: &mut Store, s: &str, p: &str, o: &str) {
    let oid = intern(store, o);
    assert_value(store, s, p, Value::Ref(oid));
}

pub(crate) fn lit(store: &mut Store, s: &str, p: &str, v: &str) {
    assert_value(store, s, p, Value::Str(v.to_string()));
}

pub(crate) fn int(store: &mut Store, s: &str, p: &str, v: i64) {
    assert_value(store, s, p, Value::Int(v));
}

pub(crate) fn seed_empty() -> (Store, PathVocab) {
    (open_store(), PathVocab::new("http://ex/v/"))
}

/// One step, no verification anywhere.
pub(crate) fn seed_unverified_trajectory() -> (Store, PathVocab) {
    let (mut store, vocab) = seed_empty();
    edge(&mut store, "http://ex/s1", &vocab.step_of.clone(), TRAJ);
    int(&mut store, "http://ex/s1", &vocab.step_order.clone(), 1);
    lit(
        &mut store,
        "http://ex/s1",
        &vocab.action_kind.clone(),
        "edit",
    );
    (store, vocab)
}

/// The canonical cone fixture:
///
/// ```text
/// s1-implement -produces->  a1 -consumedBy-> s3-test
/// s2-detour    -produces->  a2                          (nothing consumes a2)
/// s3-test      -produces->  r1 -consumedBy-> s4-verify
/// s4-verify    -verifiedBy-> verif   (falsifier present)
/// s5-mail                              (no derivation edges at all)
/// ```
pub(crate) fn seed_verified_trajectory() -> (Store, PathVocab) {
    let (mut store, vocab) = seed_empty();
    let step_of = vocab.step_of.clone();
    let order = vocab.step_order.clone();
    let kind = vocab.action_kind.clone();

    for (iri, n, k) in [
        ("http://ex/s1-implement", 1, "edit"),
        ("http://ex/s2-detour", 2, "edit"),
        ("http://ex/s3-test", 3, "run"),
        ("http://ex/s4-verify", 4, "verify"),
        ("http://ex/s5-mail", 5, "mail"),
    ] {
        edge(&mut store, iri, &step_of, TRAJ);
        int(&mut store, iri, &order, n);
        lit(&mut store, iri, &kind, k);
    }

    edge(
        &mut store,
        "http://ex/s1-implement",
        PRODUCES,
        "http://ex/a1",
    );
    edge(&mut store, "http://ex/a1", CONSUMED_BY, "http://ex/s3-test");
    edge(&mut store, "http://ex/s2-detour", PRODUCES, "http://ex/a2");
    edge(&mut store, "http://ex/s3-test", PRODUCES, "http://ex/r1");
    edge(
        &mut store,
        "http://ex/r1",
        CONSUMED_BY,
        "http://ex/s4-verify",
    );
    edge(
        &mut store,
        "http://ex/s4-verify",
        &vocab.verified_by.clone(),
        "http://ex/verif",
    );
    lit(
        &mut store,
        "http://ex/verif",
        &vocab.falsifier.clone(),
        "a non-200 from /health",
    );
    (store, vocab)
}
