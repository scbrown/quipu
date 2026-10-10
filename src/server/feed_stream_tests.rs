//! Native-log tests, including named graphs and outer-savepoint rollback.

use super::*;
use axum::extract::State;
use futures::StreamExt;
use quipu::{
    Store,
    store::Datum,
    types::{Op, Value as FactValue},
};

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
        store.latest_event_offset().unwrap(),
        0,
        "legacy ROOT event scope unchanged"
    );
    println!(
        "DEMO legacy ROOT event offset: {}",
        store.latest_event_offset().unwrap()
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
