//! Native-log tests, including named graphs and outer-savepoint rollback.

use super::*;
use axum::extract::State;
use futures::StreamExt;
use quipu::{
    Store,
    store::Datum,
    types::{Op, Value as FactValue},
};
use sha2::{Digest, Sha256};

fn datum(store: &mut Store, name: &str) -> Datum {
    Datum {
        entity: store.intern(name).unwrap(),
        attribute: store.intern("urn:test:value").unwrap(),
        value: FactValue::Int(1),
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn params(value: Value) -> Query<Params> {
    Query(serde_json::from_value(value).unwrap())
}

#[tokio::test]
async fn malformed_or_conflicting_resume_never_opens_a_stream() {
    let store = Arc::new(super::super::handle::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    for id in ["-1", "garbage", "9223372036854775808"] {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", id.parse().unwrap());
        let result = events_stream(State(store.clone()), headers, params(json!({}))).await;
        assert_eq!(result.status(), StatusCode::BAD_REQUEST);
    }
    let mut headers = HeaderMap::new();
    headers.insert("last-event-id", "2".parse().unwrap());
    assert_eq!(
        events_stream(State(store), headers, params(json!({"since":1})))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn named_graph_changes_replay_without_event_offset_ack() {
    let mut store = Store::open_in_memory().unwrap();
    let graph = store.graph_create("urn:test:seeds-board").unwrap();
    let other = store.graph_create("urn:test:other-board").unwrap();
    let first = datum(&mut store, "urn:test:work-1");
    let second = datum(&mut store, "urn:test:work-2");
    store
        .transact_to_graph(&[second], "2026-01-01", None, None, other)
        .unwrap();
    let tx = store
        .transact_to_graph(&[first], "2026-01-01", None, None, graph)
        .unwrap();
    assert_eq!(
        store.latest_event_offset_in(None).unwrap(),
        0,
        "legacy ROOT event scope unchanged"
    );
    println!(
        "DEMO legacy ROOT event offset: {}",
        store.latest_event_offset_in(None).unwrap()
    );
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(store));
    for replay in 0..2 {
        let response = changes_stream(
            State(shared.clone()),
            HeaderMap::new(),
            params(json!({"since":0,"graph":"urn:test:seeds-board"})),
        )
        .await;
        println!(
            "DEMO named-graph stream replay {} HTTP {}",
            replay + 1,
            response.status().as_u16()
        );
        let mut bytes = response.into_body().into_data_stream();
        let frame = tokio::time::timeout(Duration::from_secs(2), bytes.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = String::from_utf8(frame.to_vec()).unwrap();
        assert!(
            text.contains(&format!("id: {tx}")) && text.contains("urn:test:work-1"),
            "{text}"
        );
        assert!(!text.contains("urn:test:work-2"), "other graph leaked");
        println!("DEMO {}", text.replace('\n', " | "));
    }
    println!(
        "DEMO consumer ACK after delivery: {}",
        shared.read().consumer_committed("test-client").unwrap()
    );
    assert_eq!(
        shared.read().consumer_committed("test-client").unwrap(),
        0,
        "delivery is not ACK"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_stream_wakes_on_real_commit_and_cancellation_releases_slot() {
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    let response = events_stream(
        State(shared.clone()),
        HeaderMap::new(),
        params(json!({"since":0})),
    )
    .await;
    let mut bytes = response.into_body().into_data_stream();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), bytes.next())
            .await
            .is_err(),
        "idle stream emitted data"
    );
    {
        let mut store = shared.lock();
        let d = datum(&mut store, "urn:test:committed");
        store.transact(&[d], "2026-01-01", None, None).unwrap();
    }
    let frame = tokio::time::timeout(Duration::from_secs(2), bytes.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        String::from_utf8(frame.to_vec())
            .unwrap()
            .contains("quipu.events")
    );
    drop(bytes);
    // No detached per-connection task can retain the permit/store after drop.
    assert_eq!(Arc::strong_count(&shared), 1);
}

#[tokio::test]
async fn inner_savepoint_hint_cannot_publish_speculative_rows() {
    let mut store = Store::open_in_memory().unwrap();
    let d = datum(&mut store, "urn:test:rolled-back");
    let signals = super::super::handle::StoreHandle::commit_wake_for(&store);
    let rx = signals.subscribe();
    store
        .speculate(&[d], "2026-01-01", |inside| {
            assert!(
                inside.latest_event_offset()? > 0,
                "positive staged-log control"
            );
            assert!(
                !rx.has_changed().unwrap(),
                "inner RELEASE emitted a commit hint"
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store.latest_event_offset().unwrap(),
        0,
        "rolled-back rows must never deliver"
    );
    assert_eq!(
        store
            .changes_after(0, 1, quipu::store::changes::Capture::NewValues, None)
            .unwrap()
            .records
            .len(),
        0
    );
}

#[tokio::test]
async fn an_event_stream_filtered_to_a_named_graph_carries_only_that_graph() {
    let mut store = Store::open_in_memory().unwrap();
    let board = store.graph_create("urn:test:seeds-board").unwrap();
    let other = store.graph_create("urn:test:other-board").unwrap();
    let mut item = datum(&mut store, "urn:test:item-other");
    store
        .transact_to_graph(&[item], "2026-01-01", None, None, other)
        .unwrap();
    item = datum(&mut store, "urn:test:item-board");
    store
        .transact_to_graph(&[item], "2026-01-01", None, None, board)
        .unwrap();
    assert_eq!(
        store.latest_event_offset_in(None).unwrap(),
        0,
        "ROOT log untouched"
    );
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(store));
    let response = events_stream(
        State(shared),
        HeaderMap::new(),
        params(json!({"since":0,"graph":"urn:test:seeds-board"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut bytes = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(Duration::from_secs(2), bytes.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(frame.to_vec()).unwrap();
    assert!(
        text.contains("quipu.events") && text.contains("entity.added"),
        "{text}"
    );
    assert!(text.contains("urn:test:item-board"), "{text}");
    assert!(
        !text.contains("urn:test:item-other"),
        "other graph leaked: {text}"
    );
}

/// One parsed SSE frame: `(event, id, data)`.
type Frame = (String, Option<String>, String);

/// Read frames until `stop` matches one (inclusive), or fail after 10 s.
async fn read_frames(response: Response, stop: impl Fn(&Frame) -> bool) -> Vec<Frame> {
    let mut bytes = response.into_body().into_data_stream();
    let (mut buffer, mut frames) = (String::new(), Vec::new());
    loop {
        while let Some(end) = buffer.find("\n\n") {
            let block: String = buffer.drain(..end + 2).collect();
            let (mut event, mut id, mut data) = (String::new(), None, String::new());
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event = v.into();
                } else if let Some(v) = line.strip_prefix("id: ") {
                    id = Some(v.into());
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = v.into();
                }
            }
            if event.is_empty() {
                continue; // keep-alive comment
            }
            let frame = (event, id, data);
            let done = stop(&frame);
            frames.push(frame);
            if done {
                return frames;
            }
        }
        let chunk = tokio::time::timeout(Duration::from_secs(10), bytes.next())
            .await
            .expect("stream stalled")
            .expect("stream ended")
            .unwrap();
        buffer.push_str(std::str::from_utf8(&chunk).unwrap());
    }
}

fn wide_datum(store: &mut Store, name: &str, width: usize) -> Datum {
    Datum {
        value: FactValue::Str("x".repeat(width)),
        ..datum(store, name)
    }
}

/// F6 (local-sync-mode.md §6): a transaction larger than one frame used to end
/// the stream with the cursor unmoved, so every reconnect hit the same dead
/// end. It now arrives as begin, parts and a commit barrier that alone carries
/// the id, and the stream continues with the next transaction.
#[tokio::test]
async fn oversized_transaction_streams_as_parts_then_continues() {
    let mut store = Store::open_in_memory().unwrap();
    let big: Vec<Datum> = (0..2500)
        .map(|i| wide_datum(&mut store, &format!("urn:test:wide-{i}"), 600))
        .collect();
    let tx1 = store.transact(&big, "2026-01-01", None, None).unwrap();
    let small = datum(&mut store, "urn:test:after");
    let tx2 = store.transact(&[small], "2026-01-02", None, None).unwrap();
    let expected = store
        .changes_after(0, 1, quipu::store::changes::Capture::OldAndNewValues, None)
        .unwrap()
        .records;
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(store));

    let open = |since: i64| {
        changes_stream(
            State(shared.clone()),
            HeaderMap::new(),
            params(json!({"since": since})),
        )
    };
    let frames = read_frames(open(0).await, |f| f.0 == "quipu.changes").await;
    let names: Vec<&str> = frames.iter().map(|f| f.0.as_str()).collect();
    let parts: Vec<&Frame> = frames
        .iter()
        .filter(|f| f.0 == "quipu.changes.part")
        .collect();
    assert_eq!(names.first(), Some(&"quipu.changes.begin"), "{names:?}");
    assert!(parts.len() >= 2, "expected a split, got {names:?}");
    assert_eq!(names[names.len() - 2], "quipu.changes.commit", "{names:?}");
    // Only the barrier and the next whole transaction carry ids.
    let ids: Vec<Option<&str>> = frames.iter().map(|f| f.1.as_deref()).collect();
    assert!(ids[..ids.len() - 2].iter().all(Option::is_none), "{ids:?}");
    assert_eq!(ids[ids.len() - 2], Some(tx1.to_string().as_str()));
    assert_eq!(ids[ids.len() - 1], Some(tx2.to_string().as_str()));

    // Every part fits; the digests chain over the parts exactly as sent.
    let mut hasher = Sha256::new();
    let mut records = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        assert!(
            fits_frame(&part.2),
            "part {index} is {} bytes",
            part.2.len()
        );
        let body: Value = serde_json::from_str(&part.2).unwrap();
        assert_eq!(body["index"], index);
        assert_eq!(body["source_tx"], tx1);
        assert_eq!(
            body["digest_before"],
            hex::encode(hasher.clone().finalize())
        );
        hasher.update(part.2.as_bytes());
        records.extend(body["records"].as_array().unwrap().iter().cloned());
    }
    let digest = hex::encode(hasher.finalize());
    let begin: Value = serde_json::from_str(&frames[0].2).unwrap();
    let commit: Value = serde_json::from_str(&frames[frames.len() - 2].2).unwrap();
    assert_eq!(begin["digest"], digest);
    assert_eq!(commit["digest"], digest);
    assert_eq!(commit["parts"], parts.len());
    assert_eq!(records, expected, "reassembled parts differ from the page");

    // A reconnect before the barrier resumes from the previous transaction and
    // replays this one from its first part; one after it skips straight on.
    let again = read_frames(open(0).await, |f| f.0 == "quipu.changes.begin").await;
    assert_eq!(again.len(), 1);
    let mut headers = HeaderMap::new();
    headers.insert("last-event-id", tx1.to_string().parse().unwrap());
    let resumed = changes_stream(State(shared.clone()), headers, params(json!({}))).await;
    let next = read_frames(resumed, |_| true).await;
    assert_eq!(next[0].0, "quipu.changes");
    assert_eq!(next[0].1.as_deref(), Some(tx2.to_string().as_str()));
}

/// The split is transport-neutral: `chunk_transaction` returns the frames
/// and SSE only renders them, so another transport (the E2 hub) can send the
/// same names, ids and data bytes, and so the same digests.
#[tokio::test]
async fn sse_sends_the_transport_neutral_frames_unchanged() {
    let mut store = Store::open_in_memory().unwrap();
    let big: Vec<Datum> = (0..2500)
        .map(|i| wide_datum(&mut store, &format!("urn:test:wide-{i}"), 600))
        .collect();
    let tx = store.transact(&big, "2026-01-01", None, None).unwrap();
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(store));
    let page = feed::changes_get(
        State(shared.clone()),
        headers_for_feed(),
        Query(
            serde_json::from_value(
                json!({"since": 0, "limit": 1, "capture": "old_and_new_values"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap()
    .0;
    let neutral: Vec<Frame> = chunk_transaction(&page, tx)
        .unwrap()
        .into_iter()
        .map(|f| (f.event.to_string(), f.id.map(|id| id.to_string()), f.data))
        .collect();
    let response =
        changes_stream(State(shared), HeaderMap::new(), params(json!({"since": 0}))).await;
    let sent = read_frames(response, |f| f.0 == "quipu.changes.commit").await;
    assert!(
        neutral.len() >= 4,
        "expected a split, got {}",
        neutral.len()
    );
    assert_eq!(sent, neutral);
}

/// A chunked transaction names its source on the frames a follower checks
/// before and after the parts: begin and the commit barrier. Without it a
/// follower cannot tell an epoch change inside a chunked transaction, and the
/// contract is "a changed epoch means re-bootstrap, never splice".
#[tokio::test]
async fn a_chunked_transaction_names_its_source_on_begin_and_commit() {
    let mut store = Store::open_in_memory().unwrap();
    let big: Vec<Datum> = (0..2500)
        .map(|i| wide_datum(&mut store, &format!("urn:test:wide-{i}"), 600))
        .collect();
    store.transact(&big, "2026-01-01", None, None).unwrap();
    let expected = json!({
        "store_id": store.store_id().unwrap(),
        "log_epoch": store.log_epoch().unwrap(),
    });
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(store));
    let response =
        changes_stream(State(shared), HeaderMap::new(), params(json!({"since": 0}))).await;
    let frames = read_frames(response, |f| f.0 == "quipu.changes.commit").await;
    for name in ["quipu.changes.begin", "quipu.changes.commit"] {
        let frame = frames.iter().find(|f| f.0 == name).unwrap();
        let body: Value = serde_json::from_str(&frame.2).unwrap();
        assert_eq!(body["source"], expected, "{name}: {body}");
    }
}

/// One record larger than a frame cannot be split. It is refused with no id,
/// so the consumer's cursor stays put, and the stream ends.
#[tokio::test]
async fn a_record_larger_than_a_frame_is_refused_without_advancing() {
    let mut store = Store::open_in_memory().unwrap();
    let huge = wide_datum(&mut store, "urn:test:huge", 1_200_000);
    store.transact(&[huge], "2026-01-01", None, None).unwrap();
    let shared = Arc::new(super::super::handle::StoreHandle::writer_only(store));
    let response =
        changes_stream(State(shared), HeaderMap::new(), params(json!({"since": 0}))).await;
    let mut bytes = response.into_body().into_data_stream();
    let first = tokio::time::timeout(Duration::from_secs(10), bytes.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(first.to_vec()).unwrap();
    assert!(text.contains("event: quipu.changes.tx_too_large"), "{text}");
    assert!(!text.contains("id:"), "{text}");
    let end = tokio::time::timeout(Duration::from_secs(5), bytes.next())
        .await
        .unwrap();
    assert!(end.is_none(), "stream should end after the refusal");
}
