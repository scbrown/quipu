//! Rule 6 of the applier contract: a redelivered transaction is a duplicate
//! only when its content digest equals the retained one. An exact replay and
//! a changed replay of the same source transaction never answer alike.
use serde_json::{Value as Json, json};

use super::super::tests::{chunk_frames, pages, source};
use super::super::{Applier, Outcome, Stop, live_digest};
use super::*;

fn replica_of(feed: &[String]) -> (Store, Applier) {
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    for page in feed {
        let outcome = applier
            .apply_frame(&mut replica, "quipu.changes", page, 1)
            .unwrap();
        assert!(matches!(outcome, Outcome::Applied { .. }), "{outcome:?}");
    }
    (replica, applier)
}

/// The first page with its first record's value replaced.
fn tampered(page: &str) -> String {
    let mut page: Json = serde_json::from_str(page).unwrap();
    page["records"][0]["value"] = json!("tampered");
    page.to_string()
}

fn first_tx(page: &str) -> i64 {
    serde_json::from_str::<Json>(page).unwrap()["records"][0]["tx"]
        .as_i64()
        .unwrap()
}

#[test]
fn an_exact_redelivery_is_a_duplicate_as_a_page_or_as_a_chunk() {
    let feed = pages(&source());
    let (mut replica, mut applier) = replica_of(&feed);
    let before = live_digest(&replica);
    let page = applier
        .apply_frame(&mut replica, "quipu.changes", &feed[0], 2)
        .unwrap();
    assert_eq!(page, Outcome::Duplicate);
    // The same transaction framed differently has the same content digest.
    let frames = chunk_frames(&feed[3], 1, true);
    let last = frames
        .iter()
        .map(|(e, d)| applier.apply_frame(&mut replica, e, d, 3).unwrap())
        .last()
        .unwrap();
    assert_eq!(last, Outcome::Duplicate);
    assert_eq!(before, live_digest(&replica));
}

#[test]
fn a_changed_redelivery_stops_the_replica_and_writes_nothing() {
    let src = source();
    let feed = pages(&src);
    let (mut replica, mut applier) = replica_of(&feed);
    let before = live_digest(&replica);
    let outcome = applier
        .apply_frame(&mut replica, "quipu.changes", &tampered(&feed[0]), 2)
        .unwrap();
    let Outcome::Stopped {
        reason:
            Stop::DigestMismatch {
                source_tx,
                applied,
                received,
            },
    } = &outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(*source_tx, first_tx(&feed[0]));
    assert_ne!(applied, received);
    assert_eq!(before, live_digest(&replica));
    // Sticky: even an exact redelivery now answers the same stop.
    let again = applier
        .apply_frame(&mut replica, "quipu.changes", &feed[0], 3)
        .unwrap();
    assert_eq!(again, outcome);
    let begin = applier
        .apply_frame(&mut replica, "quipu.changes.begin", "{}", 4)
        .unwrap();
    assert_eq!(begin, outcome);
}

#[test]
fn a_redelivery_with_no_retained_digest_is_unverifiable() {
    let feed = pages(&source());
    let (mut replica, mut applier) = replica_of(&feed);
    let tx = first_tx(&feed[0]);
    replica
        .conn
        .execute("DELETE FROM replica_applied_tx WHERE source_tx = ?1", [tx])
        .unwrap();
    let outcome = applier
        .apply_frame(&mut replica, "quipu.changes", &feed[0], 2)
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Stopped {
            reason: Stop::DigestUnverifiable { source_tx: tx }
        }
    );
}

#[test]
fn a_new_source_after_a_stop_still_means_rebootstrap() {
    let feed = pages(&source());
    let (mut replica, mut applier) = replica_of(&feed);
    applier
        .apply_frame(&mut replica, "quipu.changes", &tampered(&feed[0]), 2)
        .unwrap();
    let mut moved: Json = serde_json::from_str(&feed[0]).unwrap();
    moved["source"]["log_epoch"] = json!("another-epoch");
    let outcome = applier
        .apply_frame(&mut replica, "quipu.changes", &moved.to_string(), 3)
        .unwrap();
    assert!(
        matches!(outcome, Outcome::Rebootstrap { .. }),
        "{outcome:?}"
    );
}

#[test]
fn retained_digests_survive_reopening() {
    let feed = pages(&source());
    let (mut replica, _) = replica_of(&feed);
    let mut reopened = Applier::open(&replica).unwrap();
    let outcome = reopened
        .apply_frame(&mut replica, "quipu.changes", &tampered(&feed[0]), 2)
        .unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Stopped {
                reason: Stop::DigestMismatch { .. }
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn the_digest_ignores_record_order_but_not_identity() {
    let page: Json = serde_json::from_str(&pages(&source())[3]).unwrap();
    let records: Vec<&Json> = page["records"].as_array().unwrap().iter().collect();
    assert!(
        records.len() > 1,
        "the retract+assert transaction has two records"
    );
    let source = Source {
        store_id: "s".into(),
        log_epoch: "e".into(),
    };
    let forward = tx_digest(&source, 4, &records).unwrap();
    let reversed: Vec<&Json> = records.iter().rev().copied().collect();
    assert_eq!(forward, tx_digest(&source, 4, &reversed).unwrap());
    assert_ne!(forward, tx_digest(&source, 5, &records).unwrap());
    let other = Source {
        store_id: "s".into(),
        log_epoch: "f".into(),
    };
    assert_ne!(forward, tx_digest(&other, 4, &records).unwrap());
}

#[test]
fn a_stop_serializes_tagged_by_kind() {
    let stop = Outcome::Stopped {
        reason: Stop::DigestUnverifiable { source_tx: 5 },
    };
    assert_eq!(
        serde_json::to_value(stop).unwrap(),
        json!({"kind": "stopped", "reason": {"kind": "digest_unverifiable", "source_tx": 5}})
    );
}
