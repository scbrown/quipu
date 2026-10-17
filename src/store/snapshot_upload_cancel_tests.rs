//! Cancellation after actual SQL staging must roll back the complete snapshot.
use super::*;
use crate::promotion_control::{PromotionControl, scoped};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[test]
fn cancelled_staged_rows_roll_back_and_fresh_control_can_promote() {
    let mut store = Store::open_in_memory().unwrap();
    let old = "<http://ex/old> <http://www.w3.org/2000/01/rdf-schema#label> \"old\" .";
    for producer in ["bobbin-chunks:r", "other"] {
        crate::mcp::knot::tool_knot(
            &mut store,
            &serde_json::json!({
                "turtle":old,"snapshot":producer,"replace_snapshot":true,
            }),
        )
        .unwrap();
    }
    let turtle: String = (0..16)
        .map(|n| {
            format!(
                "<http://ex/new{n}> <http://www.w3.org/2000/01/rdf-schema#label> \"new{n}\" .\n"
            )
        })
        .collect();
    let hash = crate::share::sha256(turtle.as_bytes());
    let id = snapshot_upload_id("bobbin-chunks:r", &hash);
    stage_snapshot_part(
        &mut store,
        &serde_json::json!({
            "upload_id":id,"snapshot":"bobbin-chunks:r","content_hash":hash,
            "part_hash":hash,"payload":turtle,"part_number":0,"total_parts":1,
            "total_bytes":turtle.len(),
        }),
    )
    .unwrap();
    let before = store.current_fact_count(0).unwrap();
    let tx_before = store.latest_tx_id().unwrap();
    // Validation may queue advisory events before transaction staging begins.
    store.queue_write_event(crate::store::PendingWriteEvent {
        event_type: "test.cancelled-upload".into(),
        subject: None,
        payload: serde_json::json!({"publication":"cancelled"}),
    });
    let control = PromotionControl::new(Duration::from_secs(30));
    let cancelled = control.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let observed = writes.clone();
    store
        .conn
        .create_scalar_function(
            "test_cancel_promotion",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                if observed.fetch_add(1, Ordering::SeqCst) == 2 {
                    cancelled.cancel();
                }
                Ok(0i64)
            },
        )
        .unwrap();
    store
        .conn
        .execute_batch(
            "CREATE TEMP TRIGGER test_cancel AFTER INSERT ON facts
        BEGIN SELECT test_cancel_promotion(); END;",
        )
        .unwrap();
    let input = serde_json::json!({"upload_id":id});
    let error = scoped(control, || promote_snapshot_upload(&mut store, &input)).unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert!(
        writes.load(Ordering::SeqCst) >= 3,
        "must cancel after real staged writes"
    );
    store
        .conn
        .execute_batch("DROP TRIGGER test_cancel")
        .unwrap();
    assert_eq!(store.current_fact_count(0).unwrap(), before);
    assert_eq!(store.latest_tx_id().unwrap(), tx_before);
    let old_id = store.lookup("http://ex/old").unwrap().unwrap();
    assert!(!store.entity_facts(old_id).unwrap().is_empty());

    let result = scoped(PromotionControl::new(Duration::from_secs(30)), || {
        promote_snapshot_upload(&mut store, &input)
    })
    .unwrap();
    assert_eq!(result["promoted"], true);
    assert_eq!(result["count"], 16);
    assert!(store.current_fact_count(0).unwrap() >= 17);
    for n in 0..16 {
        let id = store.lookup(&format!("http://ex/new{n}")).unwrap().unwrap();
        assert!(!store.entity_facts(id).unwrap().is_empty());
    }
    assert!(
        !store.entity_facts(old_id).unwrap().is_empty(),
        "other producer survives"
    );
    let leaked: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type='test.cancelled-upload'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        leaked, 0,
        "a cancelled upload's advisory event leaked into a later write"
    );
}
