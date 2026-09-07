//! Every write path must stamp a transaction source (aegis-byn4fn).
//!
//! The defect these guard is not a wrong value, it is a MISSING one:
//! `plan_source_retraction` is `WHERE t.source = ?1`, and SQL `NULL = ?1` is
//! never true, so a null-sourced fact can never be retracted by anything. 1,008
//! such transactions were measured on the live log, all from `/knot` with the
//! `source` field omitted — the one path among /knot, /set, /retract and
//! /episode that could produce one.
//!
//! The second guard is against a CONSTANT source. `/set` and `/retract` did
//! carry a source, but the same one for every caller ("set", 26,008
//! transactions; "retract", 36,409) — attributable to nobody, and a retraction
//! handle meaning "every correction the fleet has ever made".

use crate::store::Store;

const TS: &str = "2026-09-07T02:00:00Z";

fn sources(store: &Store) -> Vec<Option<String>> {
    store
        .list_transactions()
        .unwrap()
        .into_iter()
        .map(|t| t.source)
        .collect()
}

fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    crate::mcp::tool_knot(
        &mut store,
        &serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\n\
                       ex:a a ex:Thing ; ex:name \"one\" ; ex:tag \"x\" .",
            "source": "seed", "actor": "malcolm", "timestamp": TS
        }),
    )
    .unwrap();
    store
}

#[test]
fn knot_without_a_source_no_longer_writes_null() {
    // THE defect: this exact call minted 1,008 permanently unretractable
    // transactions on the live store.
    let mut store = seeded();
    crate::mcp::tool_knot(
        &mut store,
        &serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:b a ex:Thing .",
            "actor": "malcolm", "timestamp": TS
        }),
    )
    .unwrap();
    let s = sources(&store);
    assert!(
        s.iter().all(Option::is_some),
        "no transaction may carry a NULL source: {s:?}"
    );
    assert_eq!(s.last().unwrap().as_deref(), Some("knot:malcolm"));
}

#[test]
fn knot_still_honours_a_caller_supplied_source() {
    let store = seeded();
    assert_eq!(sources(&store).last().unwrap().as_deref(), Some("seed"));
}

#[test]
fn set_and_retract_are_per_actor_not_one_global_bucket() {
    let mut store = seeded();
    crate::mcp::tools::tool_set(
        &mut store,
        &serde_json::json!({"entity": "http://example.org/a",
            "predicate": "http://example.org/name", "value": "two",
            "actor": "malcolm", "timestamp": TS}),
    )
    .unwrap();
    crate::mcp::tools::tool_retract(
        &mut store,
        &serde_json::json!({"entity": "http://example.org/a",
            "predicate": "http://example.org/tag", "actor": "kelly", "timestamp": TS}),
    )
    .unwrap();
    let s: Vec<String> = sources(&store).into_iter().flatten().collect();
    assert!(s.contains(&"set:malcolm".to_string()), "{s:?}");
    assert!(s.contains(&"retract:kelly".to_string()), "{s:?}");
    // The constant keys are what made a correction attributable to nobody.
    assert!(
        !s.contains(&"set".to_string()),
        "constant 'set' key returned: {s:?}"
    );
    assert!(
        !s.contains(&"retract".to_string()),
        "constant key returned: {s:?}"
    );
}

#[test]
fn an_actorless_write_still_gets_a_distinct_non_null_key() {
    let mut store = seeded();
    crate::mcp::tools::tool_set(
        &mut store,
        &serde_json::json!({"entity": "http://example.org/a",
            "predicate": "http://example.org/name", "value": "three", "timestamp": TS}),
    )
    .unwrap();
    let s: Vec<String> = sources(&store).into_iter().flatten().collect();
    assert!(s.contains(&"set:anonymous".to_string()), "{s:?}");
}

#[test]
fn the_new_keys_are_actually_retractable_which_is_the_whole_point() {
    // A source that cannot be named is a fact that cannot be removed. Prove the
    // derived key reaches the retraction planner end to end, because "it is not
    // NULL any more" is not the property that matters.
    let mut store = seeded();
    crate::mcp::tool_knot(
        &mut store,
        &serde_json::json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:c a ex:Thing ; ex:n \"1\" .",
            "actor": "malcolm", "timestamp": TS
        }),
    )
    .unwrap();
    let plan = crate::mcp::tool_retract_source(
        &mut store,
        &serde_json::json!({"source": "knot:malcolm", "repair": "aegis-byn4fn"}),
    )
    .unwrap();
    assert_eq!(plan["planned"], 2, "the appended facts are addressable");

    let applied = crate::mcp::tool_retract_source(
        &mut store,
        &serde_json::json!({"source": "knot:malcolm", "repair": "aegis-byn4fn",
                            "apply": true, "expect": 2}),
    )
    .unwrap();
    assert_eq!(applied["remaining"], 0);
    assert!(
        !store
            .current_facts()
            .unwrap()
            .iter()
            .any(|f| store.resolve(f.entity).unwrap() == "http://example.org/c"),
        "the previously unretractable facts are gone"
    );
    // And the seeded producer is untouched.
    assert!(
        store
            .current_facts()
            .unwrap()
            .iter()
            .any(|f| store.resolve(f.entity).unwrap() == "http://example.org/a"),
    );
}
