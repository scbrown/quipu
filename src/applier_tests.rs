//! The shared applier against the shipped feed: equal replicas from whole
//! pages and from chunked transactions, and a refusal (with nothing written)
//! for every case the contract says must not splice.
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::store::changes::Capture;

const T: &str = "2026-10-24T00:00:00Z";

fn d(store: &Store, e: &str, a: &str, v: Value, op: Op) -> Datum {
    Datum {
        entity: store.intern(e).unwrap(),
        attribute: store.intern(a).unwrap(),
        value: v,
        valid_from: T.into(),
        valid_to: None,
        op,
    }
}

/// A source with ROOT and named-graph writes, a retraction and bytes.
pub(super) fn source() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    let r = |s: &Store, iri: &str| Value::Ref(s.intern(iri).unwrap());
    let a = d(
        &s,
        "urn:item:1",
        "urn:p:status",
        Value::Str("open".into()),
        Op::Assert,
    );
    s.transact(&[a], T, None, None).unwrap();
    let b = d(
        &s,
        "urn:item:1",
        "urn:p:blob",
        Value::Bytes(vec![0, 1, 2, 255]),
        Op::Assert,
    );
    s.transact(&[b], T, None, None).unwrap();
    let g = s.graph_create("urn:graph:board").unwrap();
    let link = r(&s, "urn:item:2");
    let c = d(&s, "urn:item:1", "urn:p:blocks", link, Op::Assert);
    s.transact_to_graph(&[c], T, None, None, g).unwrap();
    let retract = d(
        &s,
        "urn:item:1",
        "urn:p:status",
        Value::Str("open".into()),
        Op::Retract,
    );
    let closed = d(
        &s,
        "urn:item:1",
        "urn:p:status",
        Value::Str("closed".into()),
        Op::Assert,
    );
    s.transact(&[retract, closed], T, None, None).unwrap();
    s
}

/// The feed one transaction per page, as `/changes/stream` sends it.
pub(super) fn pages(source: &Store) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    loop {
        let page = source
            .changes_after(cursor, 1, Capture::OldAndNewValues, None)
            .unwrap();
        if page.records.is_empty() {
            return out;
        }
        cursor = page.next_tx;
        out.push(page.to_json().to_string());
    }
}

fn replay(replica: &mut Store, applier: &mut Applier, pages: &[String]) -> Vec<Outcome> {
    pages
        .iter()
        .map(|p| applier.apply_frame(replica, "quipu.changes", p, 1).unwrap())
        .collect()
}

#[test]
fn the_feed_builds_a_replica_equal_to_its_source() {
    let src = source();
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let outcomes = replay(&mut replica, &mut applier, &pages(&src));
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Outcome::Applied { .. })),
        "{outcomes:?}"
    );
    assert_eq!(live_digest(&src), live_digest(&replica));
    assert_eq!(
        applier.progress().unwrap().source_tx,
        src.latest_tx_id().unwrap()
    );
}

#[test]
fn a_repeated_feed_is_a_duplicate_and_changes_nothing() {
    let src = source();
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let feed = pages(&src);
    replay(&mut replica, &mut applier, &feed);
    let before = live_digest(&replica);
    let again = replay(&mut replica, &mut applier, &feed);
    assert!(again.iter().all(|o| *o == Outcome::Duplicate), "{again:?}");
    assert_eq!(before, live_digest(&replica));
}

#[test]
fn a_new_source_epoch_means_rebootstrap_never_splice() {
    let src = source();
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let feed = pages(&src);
    replay(&mut replica, &mut applier, &feed[..2]);
    let before = live_digest(&replica);
    let mut page: Json = serde_json::from_str(&feed[2]).unwrap();
    page["source"]["log_epoch"] = json!("another-epoch");
    let outcome = applier
        .apply_frame(&mut replica, "quipu.changes", &page.to_string(), 2)
        .unwrap();
    assert!(
        matches!(outcome, Outcome::Rebootstrap { .. }),
        "{outcome:?}"
    );
    assert_eq!(before, live_digest(&replica), "nothing may be spliced in");
}

#[test]
fn a_page_without_its_source_is_refused() {
    let src = source();
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let mut page: Json = serde_json::from_str(&pages(&src)[0]).unwrap();
    page.as_object_mut().unwrap().remove("source");
    let outcome = applier
        .apply_frame(&mut replica, "quipu.changes", &page.to_string(), 1)
        .unwrap();
    assert!(matches!(outcome, Outcome::Refused { .. }), "{outcome:?}");
    assert!(applier.progress().is_none());
}

#[test]
fn a_length_only_byte_value_is_refused_and_nothing_is_written() {
    let src = source();
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let feed = pages(&src);
    replay(&mut replica, &mut applier, &feed[..1]);
    let before = (live_digest(&replica), applier.progress().cloned());
    // The second source transaction carries the bytes: strip them to the
    // length-only form a pre-F4 source sends.
    let mut page: Json = serde_json::from_str(&feed[1]).unwrap();
    for record in page["records"].as_array_mut().unwrap() {
        if record["value"].get("base64").is_some() {
            record["value"] = json!({"bytes_len": 4});
        }
    }
    let outcome = applier
        .apply_frame(&mut replica, "quipu.changes", &page.to_string(), 2)
        .unwrap();
    assert!(
        matches!(&outcome, Outcome::Refused { reason } if reason.contains("length only")),
        "{outcome:?}"
    );
    assert_eq!(before, (live_digest(&replica), applier.progress().cloned()));
}

/// The chunked frames for one page, in the shape the server sends them
/// (begin, parts with a running digest of the parts as sent, commit). The
/// server builds these in `feed_stream::chunk_transaction`; this mirrors its
/// output format for a library test.
pub(super) fn chunk_frames(
    page: &str,
    per_part: usize,
    with_source: bool,
) -> Vec<(&'static str, String)> {
    let page: Json = serde_json::from_str(page).unwrap();
    let records = page["records"].as_array().unwrap().clone();
    let tx = page["next_tx"].as_i64().unwrap();
    let mut hasher = Sha256::new();
    let mut parts = Vec::new();
    for (index, chunk) in records.chunks(per_part).enumerate() {
        let before = hex::encode(hasher.clone().finalize());
        let data =
            json!({"source_tx": tx, "index": index, "digest_before": before, "records": chunk})
                .to_string();
        hasher.update(data.as_bytes());
        parts.push(data);
    }
    let digest = hex::encode(hasher.finalize());
    let mut frames = vec![(
        "quipu.changes.begin",
        json!({"source_tx": tx, "parts": parts.len(), "records": records.len(), "digest": digest, "since": page["since"]})
            .to_string(),
    )];
    frames.extend(parts.into_iter().map(|p| ("quipu.changes.part", p)));
    let mut commit = json!({"source_tx": tx, "parts": frames.len() - 1, "records": records.len(), "digest": digest, "since": page["since"]});
    if with_source {
        commit["source"] = page["source"].clone();
    }
    frames.push(("quipu.changes.commit", commit.to_string()));
    frames
}

fn apply(replica: &mut Store, applier: &mut Applier, frames: &[(&str, String)]) -> Vec<Outcome> {
    frames
        .iter()
        .map(|(e, data)| applier.apply_frame(replica, e, data, 1).unwrap())
        .collect()
}

#[test]
fn a_dropped_page_resumes_with_nothing_applied() {
    let src = source();
    let feed = pages(&src);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..1]);
    let cursor = applier.progress().unwrap().source_tx;
    let before = live_digest(&replica);
    // Page 1 is lost in transit; page 2 answers a cursor ahead of ours.
    let o = applier
        .apply_frame(&mut replica, "quipu.changes", &feed[2], 2)
        .unwrap();
    assert_eq!(o, Outcome::Resume { from: cursor });
    assert_eq!(before, live_digest(&replica), "nothing may be spliced in");
    assert_eq!(applier.progress().unwrap().source_tx, cursor);
    // Resuming from the cursor converges.
    replay(&mut replica, &mut applier, &feed[1..]);
    assert_eq!(live_digest(&src), live_digest(&replica));
}

#[test]
fn a_new_replica_starts_at_transaction_zero() {
    let src = source();
    let feed = pages(&src);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let o = applier
        .apply_frame(&mut replica, "quipu.changes", &feed[1], 1)
        .unwrap();
    assert_eq!(o, Outcome::Resume { from: 0 });
    assert!(applier.progress().is_none());
}

#[test]
fn a_page_that_does_not_name_its_cursor_is_refused() {
    let src = source();
    let mut page: Json = serde_json::from_str(&pages(&src)[0]).unwrap();
    page.as_object_mut().unwrap().remove("since");
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    let o = applier
        .apply_frame(&mut replica, "quipu.changes", &page.to_string(), 1)
        .unwrap();
    assert!(matches!(o, Outcome::Refused { .. }), "{o:?}");
    assert!(applier.progress().is_none());
}

#[test]
fn a_page_inside_an_open_chunk_discards_it_and_resumes() {
    let src = source();
    let feed = pages(&src);
    let last = feed.last().unwrap();
    let frames = chunk_frames(last, 1, true);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 1]);
    let cursor = applier.progress().unwrap().source_tx;
    let before = live_digest(&replica);
    apply(&mut replica, &mut applier, &frames[..2]);
    // Without this rule the page would apply the chunk's transaction, move
    // the cursor past it, and the commit would be dropped as a duplicate.
    let o = applier
        .apply_frame(&mut replica, "quipu.changes", last, 2)
        .unwrap();
    assert_eq!(o, Outcome::Resume { from: cursor });
    assert_eq!(before, live_digest(&replica));
    let rest = apply(&mut replica, &mut applier, &frames[2..]);
    assert!(
        rest.iter().all(|o| *o == Outcome::Resume { from: cursor }),
        "{rest:?}"
    );
    assert_eq!(before, live_digest(&replica));
    replay(&mut replica, &mut applier, &feed[feed.len() - 1..]);
    assert_eq!(live_digest(&src), live_digest(&replica));
}

#[test]
fn an_interrupted_chunk_restarts_from_a_new_begin() {
    let src = source();
    let feed = pages(&src);
    let frames = chunk_frames(feed.last().unwrap(), 1, true);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 1]);
    // begin, part 0, then the reconnect replays from begin.
    apply(&mut replica, &mut applier, &frames[..2]);
    let outcomes = apply(&mut replica, &mut applier, &frames);
    assert!(
        matches!(outcomes.last(), Some(Outcome::Applied { .. })),
        "{outcomes:?}"
    );
    assert_eq!(live_digest(&src), live_digest(&replica));
}

#[test]
fn a_reopened_applier_holds_no_staged_parts() {
    let src = source();
    let feed = pages(&src);
    let frames = chunk_frames(feed.last().unwrap(), 1, true);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 1]);
    let cursor = applier.progress().unwrap().source_tx;
    let before = live_digest(&replica);
    apply(&mut replica, &mut applier, &frames[..2]);
    // The page restarts: the staged part was memory only.
    let mut applier = Applier::open(&replica).unwrap();
    let rest = apply(&mut replica, &mut applier, &frames[2..]);
    assert!(
        rest.iter().all(|o| *o == Outcome::Resume { from: cursor }),
        "{rest:?}"
    );
    assert_eq!(before, live_digest(&replica));
}

#[test]
fn a_chunk_that_skips_a_transaction_resumes_at_its_begin() {
    let src = source();
    let feed = pages(&src);
    let frames = chunk_frames(feed.last().unwrap(), 1, true);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 2]);
    let cursor = applier.progress().unwrap().source_tx;
    let before = live_digest(&replica);
    let outcomes = apply(&mut replica, &mut applier, &frames);
    assert!(
        outcomes
            .iter()
            .all(|o| *o == Outcome::Resume { from: cursor }),
        "{outcomes:?}"
    );
    assert_eq!(before, live_digest(&replica));
}

#[test]
fn a_chunked_transaction_applies_only_when_its_commit_verifies() {
    let src = source();
    let feed = pages(&src);
    // The last source transaction has two records: retract + assert.
    let frames = chunk_frames(feed.last().unwrap(), 1, true);
    assert_eq!(frames.len(), 4);

    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 1]);
    let before = live_digest(&replica);
    for (event, data) in &frames[..3] {
        let o = applier.apply_frame(&mut replica, event, data, 3).unwrap();
        assert!(matches!(o, Outcome::Buffered { .. }), "{o:?}");
        assert_eq!(
            before,
            live_digest(&replica),
            "a buffered part is not visible"
        );
    }
    let (event, data) = &frames[3];
    let o = applier.apply_frame(&mut replica, event, data, 4).unwrap();
    assert!(matches!(o, Outcome::Applied { .. }), "{o:?}");
    assert_eq!(live_digest(&src), live_digest(&replica));
}

#[test]
fn a_part_out_of_order_resumes_with_nothing_applied() {
    let src = source();
    let feed = pages(&src);
    let frames = chunk_frames(feed.last().unwrap(), 1, true);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 1]);
    let before = live_digest(&replica);
    let cursor = applier.progress().unwrap().source_tx;
    applier
        .apply_frame(&mut replica, frames[0].0, &frames[0].1, 1)
        .unwrap();
    // Skip part 0: part 1 arrives first.
    let o = applier
        .apply_frame(&mut replica, frames[2].0, &frames[2].1, 2)
        .unwrap();
    assert_eq!(o, Outcome::Resume { from: cursor });
    let o = applier
        .apply_frame(&mut replica, frames[3].0, &frames[3].1, 3)
        .unwrap();
    assert_eq!(
        o,
        Outcome::Resume { from: cursor },
        "a commit without its chunk resumes"
    );
    assert_eq!(before, live_digest(&replica));
}

#[test]
fn a_chunked_commit_that_does_not_name_its_source_is_refused() {
    let src = source();
    let feed = pages(&src);
    let frames = chunk_frames(feed.last().unwrap(), 1, false);
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    replay(&mut replica, &mut applier, &feed[..feed.len() - 1]);
    let before = live_digest(&replica);
    let outcomes: Vec<Outcome> = frames
        .iter()
        .map(|(e, data)| applier.apply_frame(&mut replica, e, data, 1).unwrap())
        .collect();
    assert!(
        matches!(outcomes.last(), Some(Outcome::Refused { .. })),
        "{outcomes:?}"
    );
    assert_eq!(before, live_digest(&replica));
}

#[test]
fn the_cursor_survives_reopening_and_freshness_is_never_bounded() {
    let src = source();
    let mut replica = Store::open_in_memory().unwrap();
    let mut applier = Applier::open(&replica).unwrap();
    assert_eq!(applier.freshness(10).last_frame_age_ms, None);
    replay(&mut replica, &mut applier, &pages(&src));
    let progress = applier.progress().cloned();
    let reopened = Applier::open(&replica).unwrap();
    assert_eq!(reopened.progress().cloned(), progress);
    let f = applier.freshness(101);
    assert_eq!(f.last_frame_age_ms, Some(100));
    assert!(!f.bounded);
    assert!(matches!(
        applier
            .apply_frame(&mut replica, "quipu.heartbeat", "{}", 5)
            .unwrap(),
        Outcome::Ignored
    ));
}

/// The oracle the replica tests rely on: stores that differ ONLY in valid
/// time, only in byte content, or only in graph must digest differently, and
/// identically built stores the same. Without this, an equality assertion
/// could pass because the digest is blind to the difference under test.
#[test]
fn the_digest_sees_valid_time_bytes_and_graph() {
    let build = |valid_to: Option<&str>, bytes: Vec<u8>, graph: Option<&str>| {
        let mut s = Store::open_in_memory().unwrap();
        let mut a = d(
            &s,
            "urn:item:1",
            "urn:p:status",
            Value::Str("open".into()),
            Op::Assert,
        );
        a.valid_to = valid_to.map(String::from);
        let b = d(
            &s,
            "urn:item:1",
            "urn:p:blob",
            Value::Bytes(bytes),
            Op::Assert,
        );
        match graph {
            None => s.transact(&[a, b], T, None, None).unwrap(),
            Some(g) => {
                let g = s.graph_create(g).unwrap();
                s.transact_to_graph(&[a, b], T, None, None, g).unwrap()
            }
        };
        live_digest(&s)
    };
    let base = build(None, vec![1, 2, 3], None);
    assert_eq!(
        base,
        build(None, vec![1, 2, 3], None),
        "same content, same digest"
    );
    assert_ne!(
        base,
        build(Some("2027-01-01T00:00:00Z"), vec![1, 2, 3], None),
        "valid_to"
    );
    assert_ne!(base, build(None, vec![1, 2, 4], None), "byte content");
    assert_ne!(
        base,
        build(None, vec![1, 2, 3], Some("urn:graph:other")),
        "graph"
    );
}

/// The browser bridge's contract: outcomes serialize tagged by `kind`.
#[test]
fn outcomes_serialize_tagged_by_kind() {
    let applied = serde_json::to_value(Outcome::Applied {
        source_txs: vec![3, 4],
    })
    .unwrap();
    assert_eq!(applied, json!({"kind": "applied", "source_txs": [3, 4]}));
    assert_eq!(
        serde_json::to_value(Outcome::Duplicate).unwrap(),
        json!({"kind": "duplicate"})
    );
    assert_eq!(
        serde_json::to_value(Outcome::Resume { from: 7 }).unwrap(),
        json!({"kind": "resume", "from": 7})
    );
    let fresh = Freshness {
        source_tx: Some(9),
        last_frame_age_ms: None,
        bounded: false,
    };
    assert_eq!(
        serde_json::to_value(fresh).unwrap(),
        json!({"source_tx": 9, "last_frame_age_ms": null, "bounded": false})
    );
}
