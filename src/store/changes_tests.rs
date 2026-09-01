//! Acceptance tests for the change feed (quipu-2ae). Every contract line in
//! `changes.rs` is asserted here against a real store — including the ones
//! about what a record does NOT carry.

use super::*;
use crate::store::{Datum, Store};
use crate::types::Op;

fn assert_datum(e: i64, a: i64, value: Value) -> Datum {
    Datum {
        entity: e,
        attribute: a,
        value,
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op: Op::Assert,
    }
}

fn retract_datum(e: i64, a: i64, value: Value) -> Datum {
    Datum {
        op: Op::Retract,
        ..assert_datum(e, a, value)
    }
}

#[test]
fn asserts_arrive_in_commit_order_and_pages_end_on_tx_boundaries() {
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/koror").unwrap();
    let a = store.intern("http://example.org/cpuCores").unwrap();
    let tx1 = store
        .transact(
            &[assert_datum(e, a, Value::Int(4))],
            "2026-04-01",
            Some("t"),
            None,
        )
        .unwrap();
    let tx2 = store
        .transact(
            &[assert_datum(e, a, Value::Int(8))],
            "2026-04-03",
            Some("t"),
            None,
        )
        .unwrap();

    // One tx per page: the cursor lands on a tx boundary, never inside one.
    let page = store.changes_after(0, 1, Capture::NewValues, None).unwrap();
    assert_eq!(page.next_tx, tx1);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0]["op"], "assert");
    assert_eq!(page.records[0]["value"], 4);
    assert_eq!(page.records[0]["entity"], "http://example.org/koror");
    assert_eq!(page.watermark_tx, tx2);

    let page2 = store
        .changes_after(page.next_tx, 10, Capture::NewValues, None)
        .unwrap();
    assert_eq!(page2.next_tx, tx2);
    // No implicit supersede in the fact log: asserting 8 does not retract 4,
    // so tx2 is exactly one assert record. (An explicit retract is its own
    // record — see the capture-modes test.)
    assert_eq!(page2.records.len(), 1, "{:#?}", page2.records);
    assert_eq!(page2.records[0]["value"], 8);

    // Fixpoint: an empty page keeps the cursor put and still reports the
    // watermark — the idle-vs-broken signal.
    let page3 = store
        .changes_after(page2.next_tx, 10, Capture::NewValues, None)
        .unwrap();
    assert!(page3.records.is_empty());
    assert_eq!(page3.next_tx, tx2);
    assert_eq!(page3.watermark_tx, tx2);
}

#[test]
fn capture_modes_carry_exactly_what_they_promise() {
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/koror").unwrap();
    let a = store.intern("http://example.org/cpuCores").unwrap();
    let name = store.intern("http://example.org/name").unwrap();
    store
        .transact(
            &[
                assert_datum(e, a, Value::Int(4)),
                assert_datum(e, name, Value::Str("koror".into())),
            ],
            "2026-04-01",
            None,
            None,
        )
        .unwrap();
    let tx2 = store
        .transact(
            &[retract_datum(e, a, Value::Int(4))],
            "2026-04-02",
            None,
            None,
        )
        .unwrap();

    // new_values: the retract identifies the fact, but the ended value is
    // withheld — this mode mirrors current state only.
    let lean = store
        .changes_after(tx2 - 1, 10, Capture::NewValues, None)
        .unwrap();
    assert_eq!(lean.records.len(), 1);
    assert_eq!(lean.records[0]["op"], "retract");
    assert!(lean.records[0].get("value").is_none());
    assert!(lean.records[0].get("old_value").is_none());

    // old_and_new_values: the ended value rides along.
    let full = store
        .changes_after(tx2 - 1, 10, Capture::OldAndNewValues, None)
        .unwrap();
    assert_eq!(full.records[0]["old_value"], 4);

    // new_row: the record carries the entity's state AS OF ITS OWN tx —
    // cpuCores is gone, name survives — so a consumer needs no read-back.
    let with_row = store
        .changes_after(tx2 - 1, 10, Capture::NewRow, None)
        .unwrap();
    let row = &with_row.records[0]["row"];
    assert!(row.get("http://example.org/cpuCores").is_none(), "{row:#?}");
    assert_eq!(row["http://example.org/name"], "koror");
}

#[test]
fn graph_scope_filters_records_but_the_watermark_still_advances() {
    let mut store = Store::open_in_memory().unwrap();
    let g_iri = "http://example.org/graph/tenant-a";
    let g = store.graph_create(g_iri).unwrap();
    let e = store.intern("http://example.org/thing").unwrap();
    let a = store.intern("http://example.org/label").unwrap();
    let tx1 = store
        .transact_to_graph(
            &[assert_datum(e, a, Value::Str("in-tenant".into()))],
            "2026-04-01",
            None,
            None,
            g,
        )
        .unwrap();
    // An unrelated ROOT write: invisible to the tenant scope, but it moves
    // the watermark — which is how a scoped consumer tells idle from broken.
    let tx2 = store
        .transact(
            &[assert_datum(e, a, Value::Str("in-root".into()))],
            "2026-04-02",
            None,
            None,
        )
        .unwrap();

    let page = store
        .changes_after(0, 10, Capture::NewValues, Some(g))
        .unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0]["graph"], g_iri);
    assert_eq!(page.next_tx, tx1);
    assert_eq!(page.watermark_tx, tx2);

    let scoped_idle = store
        .changes_after(tx1, 10, Capture::NewValues, Some(g))
        .unwrap();
    assert!(scoped_idle.records.is_empty());
    assert_eq!(scoped_idle.watermark_tx, tx2);
}

#[test]
fn per_entity_records_are_in_commit_order_and_refs_resolve() {
    let mut store = Store::open_in_memory().unwrap();
    let alice = store.intern("http://example.org/alice").unwrap();
    let bob = store.intern("http://example.org/bob").unwrap();
    let knows = store.intern("http://example.org/knows").unwrap();
    store
        .transact(
            &[assert_datum(alice, knows, Value::Ref(bob))],
            "2026-04-01",
            None,
            None,
        )
        .unwrap();
    store
        .transact(
            &[retract_datum(alice, knows, Value::Ref(bob))],
            "2026-04-02",
            None,
            None,
        )
        .unwrap();

    let page = store
        .changes_after(0, 10, Capture::OldAndNewValues, None)
        .unwrap();
    let for_alice: Vec<_> = page
        .records
        .iter()
        .filter(|r| r["entity"] == "http://example.org/alice")
        .collect();
    assert_eq!(for_alice.len(), 2);
    // Commit order per entity: the assert precedes the retract, and both
    // resolve the Ref to an IRI a consumer can use.
    assert_eq!(for_alice[0]["op"], "assert");
    assert_eq!(for_alice[0]["value"]["ref"], "http://example.org/bob");
    assert_eq!(for_alice[1]["op"], "retract");
    assert_eq!(for_alice[1]["old_value"]["ref"], "http://example.org/bob");
    assert!(for_alice[0]["tx"].as_i64() < for_alice[1]["tx"].as_i64());
}

#[test]
fn unknown_capture_names_are_refused_not_defaulted() {
    assert!(Capture::parse("new_values").is_some());
    assert!(Capture::parse("NEW_VALUES").is_none());
    assert!(Capture::parse("everything").is_none());
}
