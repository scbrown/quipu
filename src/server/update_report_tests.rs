//! `/update` reports what it committed (aegis-xajsgn).
//!
//! It answered 204 with no body whether or not a conditional WHERE matched, so
//! a compare-and-swap caller (claim if unassigned, release if still mine) could
//! not tell whether it won without a racy read-back, and got no transaction id.
//! Measured on 0.11.0: a no-match DELETE WHERE and a matching update returned
//! the same empty 204.

use std::sync::{Arc, Barrier};

use quipu::Store;

use super::super::{SharedStore, StoreHandle};
use super::apply_update;

const G: &str = "http://ex.org/g/claims";

fn fresh() -> SharedStore {
    Arc::new(StoreHandle::writer_only(Store::open_in_memory().unwrap()))
}

/// Claim `s` for `who` only if nobody holds it.
fn claim(who: &str) -> String {
    format!(
        "INSERT {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"{who}\" }} }} \
         WHERE {{ FILTER NOT EXISTS {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> ?o }} }} }}"
    )
}

/// Release `s` only if `who` still holds it.
fn release(who: &str) -> String {
    format!(
        "DELETE {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"{who}\" }} }} \
         WHERE {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"{who}\" }} }}"
    )
}

fn n(v: &serde_json::Value, k: &str) -> u64 {
    v[k].as_u64()
        .unwrap_or_else(|| panic!("{k} missing in {v}"))
}

#[test]
fn a_where_that_matches_nothing_reports_zero_and_no_tx() {
    let shared = fresh();
    let r = apply_update(&shared, "DELETE WHERE { <urn:x:absent> <urn:x:p> ?o }").unwrap();
    assert_eq!((n(&r, "asserted"), n(&r, "retracted")), (0, 0), "{r}");
    assert!(r["tx"].is_null(), "no write, no tx: {r}");
    assert_eq!(r["graphs"].as_array().unwrap().len(), 0, "{r}");
}

#[test]
fn a_matching_update_reports_its_counts_and_a_real_tx() {
    let shared = fresh();
    let won = apply_update(&shared, &claim("alice")).unwrap();
    assert_eq!((n(&won, "asserted"), n(&won, "retracted")), (1, 0), "{won}");
    let tx = won["tx"].as_i64().expect("a committed update names its tx");
    assert!(
        shared.lock().get_transaction(tx).unwrap().is_some(),
        "tx {tx} must be a transaction the store knows"
    );
    let g = &won["graphs"][0];
    assert_eq!(g["graph"], G, "{won}");
    assert_eq!(g["tx"], tx, "{won}");

    // A replace in one update: one retracted, one asserted, same transaction.
    let moved = apply_update(
        &shared,
        &format!(
            "DELETE {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"alice\" }} }} \
             INSERT {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"bob\" }} }} \
             WHERE {{ GRAPH <{G}> {{ <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"alice\" }} }}"
        ),
    )
    .unwrap();
    assert_eq!(
        (n(&moved, "asserted"), n(&moved, "retracted")),
        (1, 1),
        "{moved}"
    );
    assert!(moved["tx"].as_i64().unwrap() > tx, "{moved}");
}

#[test]
fn a_lost_compare_and_swap_is_distinguishable_from_a_won_one() {
    let shared = fresh();
    assert_eq!(
        n(&apply_update(&shared, &claim("alice")).unwrap(), "asserted"),
        1
    );
    // bob's claim loses: the guard fails, nothing is written, and he can tell.
    let lost = apply_update(&shared, &claim("bob")).unwrap();
    assert_eq!(
        (n(&lost, "asserted"), n(&lost, "retracted")),
        (0, 0),
        "{lost}"
    );
    assert!(lost["tx"].is_null(), "{lost}");
    // bob cannot release alice's claim; alice can.
    assert_eq!(
        n(
            &apply_update(&shared, &release("bob")).unwrap(),
            "retracted"
        ),
        0
    );
    assert_eq!(
        n(
            &apply_update(&shared, &release("alice")).unwrap(),
            "retracted"
        ),
        1
    );
}

#[test]
fn of_two_racing_claims_exactly_one_reports_a_win() {
    let shared = fresh();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|who| {
            let (shared, barrier) = (Arc::clone(&shared), Arc::clone(&barrier));
            std::thread::spawn(move || {
                barrier.wait();
                apply_update(&shared, &claim(who)).unwrap()
            })
        })
        .collect();
    let wins: Vec<u64> = handles
        .into_iter()
        .map(|h| n(&h.join().unwrap(), "asserted"))
        .collect();
    assert_eq!(wins.iter().sum::<u64>(), 1, "exactly one winner: {wins:?}");
}
