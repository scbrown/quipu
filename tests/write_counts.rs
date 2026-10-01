//! `quipu_write_facts_total` against real commits (aegis-gwkd76). Its own test
//! binary, one test: the metrics registry and the process mode are global, so
//! no other test may write in this process.

use quipu::write_kind::{WriteKind, scoped, set_serving};
use quipu::{Datum, Op, Store, Value};

/// The value of one series, or 0 when it is absent (zero outcomes are omitted).
fn series(writer: &str, graph: &str, outcome: &str) -> u64 {
    let text = quipu::metrics::metrics().render(0, 0, 0, None);
    let key = format!(
        "quipu_write_facts_total{{writer=\"{writer}\",graph=\"{graph}\",outcome=\"{outcome}\"}} "
    );
    text.lines()
        .find_map(|l| l.strip_prefix(&key))
        .map_or(0, |n| n.parse().unwrap())
}

fn written_total() -> u64 {
    let text = quipu::metrics::metrics().render(0, 0, 0, None);
    text.lines()
        .find_map(|l| l.strip_prefix("quipu_facts_written_total "))
        .unwrap()
        .parse()
        .unwrap()
}

fn datum(store: &mut Store, value: &str, op: Op) -> Datum {
    Datum {
        entity: store.intern("urn:test:s").unwrap(),
        attribute: store.intern("urn:test:p").unwrap(),
        value: Value::Str(value.into()),
        valid_from: "2026-01-01".into(),
        valid_to: None,
        op,
    }
}

#[test]
fn write_outcomes_split_submitted_from_what_changed() {
    let mut store = Store::open_in_memory().unwrap();
    let d = datum(&mut store, "v", Op::Assert);

    // Before the server serves, an unscoped write is STARTUP.
    store
        .transact(std::slice::from_ref(&d), "2026-01-01", None, Some("A"))
        .unwrap();
    assert_eq!(series("startup", "root", "asserted"), 1);
    set_serving();

    // malcolm's sabotage arm: a BYTE-IDENTICAL re-assert is submitted, counted
    // by the old total, and changes nothing. It must be noop, never asserted.
    let before_total = written_total();
    scoped(Some(WriteKind::Knot), || {
        store
            .transact(std::slice::from_ref(&d), "2026-01-02", None, Some("A"))
            .unwrap();
    });
    assert_eq!(
        written_total(),
        before_total + 1,
        "the old total still counts it"
    );
    assert_eq!(series("knot", "root", "submitted"), 1);
    assert_eq!(series("knot", "root", "noop"), 1);
    assert_eq!(
        series("knot", "root", "asserted"),
        0,
        "an identical re-assert asserted nothing"
    );

    // Control: a NEW value IS asserted, by the same path.
    let fresh = datum(&mut store, "w", Op::Assert);
    scoped(Some(WriteKind::Knot), || {
        store
            .transact(&[fresh], "2026-01-03", None, Some("A"))
            .unwrap();
    });
    assert_eq!(series("knot", "root", "asserted"), 1);

    // A retraction of a live fact is retracted, not noop.
    let gone = datum(&mut store, "v", Op::Retract);
    scoped(Some(WriteKind::Retract), || {
        store
            .transact(&[gone], "2026-01-04", None, Some("A"))
            .unwrap();
    });
    assert_eq!(series("retract", "root", "retracted"), 1);
    assert_eq!(series("retract", "root", "noop"), 0);

    // A named graph is not what quipu_graph_facts counts: graph="named".
    let named = store.graph_create("urn:test:graph:named").unwrap();
    let other = datum(&mut store, "x", Op::Assert);
    scoped(Some(WriteKind::Knot), || {
        store
            .transact_to_graph(&[other], "2026-01-05", None, Some("A"), named)
            .unwrap();
    });
    assert_eq!(series("knot", "named", "asserted"), 1);
    assert_eq!(
        series("knot", "root", "asserted"),
        1,
        "the root series did not move"
    );

    // After serving begins, an unscoped write is a finding: unclassified.
    let late = datum(&mut store, "y", Op::Assert);
    store
        .transact(&[late], "2026-01-06", None, Some("A"))
        .unwrap();
    assert_eq!(series("unclassified", "root", "asserted"), 1);
    assert_eq!(
        series("startup", "root", "asserted"),
        1,
        "startup did not move"
    );
}
