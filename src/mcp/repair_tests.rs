//! Tests for `quipu_retract_source` (aegis-rz75m6).
//!
//! Each test here is written to FAIL if the property it names is removed. The
//! two that matter most are `no_assertion_can_reach_the_store_through_this_path`
//! (the reason this is a new tool instead of a raw `source_tag` on `/knot`) and
//! `duplicate_rows_under_one_source_retract_exactly_once` (which fails with a
//! UNIQUE constraint error if the dedupe is dropped).

use super::repair::tool_retract_source;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const TS: &str = "2026-09-07T00:00:00Z";

/// A source string of the shape that motivated this tool: minted by a hand-run
/// `yupana promote`, matching no `snapshot:` prefix, therefore previously
/// unretractable by any surface.
const LEGACY: &str = "yupana promote quipu@7256def58f6197fbd0a80d586fb38b45fc8925f4 (cli)";
const CANONICAL: &str = "snapshot:code:quipu";

fn seed(store: &mut Store, turtle: &str, source: &str) {
    crate::rdf::ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        Some("test"),
        Some(source),
    )
    .unwrap();
}

fn store_with_two_producers() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    seed(
        &mut store,
        r#"@prefix ex: <http://example.org/> .
           ex:gone a ex:CodeModule ; ex:path "src/deleted.rs" ."#,
        LEGACY,
    );
    seed(
        &mut store,
        r#"@prefix ex: <http://example.org/> .
           ex:live a ex:CodeModule ; ex:path "src/live.rs" ."#,
        CANONICAL,
    );
    store
}

fn live_count(store: &Store) -> usize {
    store.current_facts().unwrap().len()
}

#[test]
fn plan_is_the_default_and_writes_nothing() {
    let mut store = store_with_two_producers();
    let before = live_count(&store);
    let head_before = store.list_transactions().unwrap().len();

    let out = tool_retract_source(
        &mut store,
        &serde_json::json!({"source": LEGACY, "repair": "aegis-rz75m6"}),
    )
    .unwrap();

    assert_eq!(out["planned"], 2, "both of the legacy producer's facts");
    assert_eq!(out["entities"], 1);
    assert_eq!(out["applied"], false);
    assert!(out["tx_id"].is_null());
    assert_eq!(out["repair_source"], "repair:aegis-rz75m6");
    assert_eq!(live_count(&store), before, "plan must not retract");
    assert_eq!(
        store.list_transactions().unwrap().len(),
        head_before,
        "plan must open no transaction at all"
    );
}

#[test]
fn a_source_that_owns_nothing_plans_zero_instead_of_failing() {
    // The answer that was previously unobtainable: `/knot` reported
    // `replaced: true, count: 0` for this case AND for one that emptied a
    // graph, so "does this source own anything?" had no answer.
    let mut store = store_with_two_producers();
    let out = tool_retract_source(
        &mut store,
        &serde_json::json!({"source": "no producer ever wrote this", "repair": "t"}),
    )
    .unwrap();
    assert_eq!(out["planned"], 0);
    assert_eq!(out["entities"], 0);
    assert_eq!(out["sample"].as_array().unwrap().len(), 0);
}

#[test]
fn apply_clears_the_named_source_and_leaves_every_other_producer() {
    let mut store = store_with_two_producers();
    let out = tool_retract_source(
        &mut store,
        &serde_json::json!({
            "source": LEGACY, "repair": "aegis-rz75m6", "apply": true, "expect": 2
        }),
    )
    .unwrap();

    assert_eq!(out["applied"], true);
    assert_eq!(out["retracted"], 2);
    assert_eq!(out["remaining"], 0, "post-state read, not a request echo");
    assert!(out["tx_id"].as_i64().unwrap() > 0);

    // The other producer is untouched — measured against the store, because the
    // response cannot testify to what it did not touch.
    let mut still = tool_retract_source(
        &mut store,
        &serde_json::json!({"source": CANONICAL, "repair": "probe"}),
    )
    .unwrap();
    assert_eq!(still["planned"], 2, "canonical producer's facts survive");
    still = tool_retract_source(
        &mut store,
        &serde_json::json!({"source": LEGACY, "repair": "probe"}),
    )
    .unwrap();
    assert_eq!(still["planned"], 0, "legacy producer owns nothing now");
}

#[test]
fn the_retraction_is_stamped_repair_not_the_source_it_cleared() {
    // The whole audit argument: a repair must not be attributable to the
    // producer whose facts it removed.
    let mut store = store_with_two_producers();
    let out = tool_retract_source(
        &mut store,
        &serde_json::json!({
            "source": LEGACY, "repair": "aegis-rz75m6", "apply": true, "expect": 2,
            "actor": "malcolm"
        }),
    )
    .unwrap();
    let tx_id = out["tx_id"].as_i64().unwrap();
    let tx = store
        .list_transactions()
        .unwrap()
        .into_iter()
        .find(|t| t.id == tx_id)
        .expect("the retraction transaction");
    assert_eq!(tx.source.as_deref(), Some("repair:aegis-rz75m6"));
    assert_ne!(
        tx.source.as_deref(),
        Some(LEGACY),
        "a repair stamped with the cleared source would re-attribute the removal"
    );
    assert_eq!(tx.actor.as_deref(), Some("malcolm"));
}

#[test]
fn no_assertion_can_reach_the_store_through_this_path() {
    // THE reason this is a separate tool rather than a raw `source_tag` input on
    // `/knot`: there, one tag stamps a transaction carrying both retractions and
    // assertions, so naming a producer's key writes facts indistinguishable from
    // that producer's own. Here there is no turtle input, so a caller who
    // supplies one gets it IGNORED — the fact count can only fall.
    let mut store = store_with_two_producers();
    let before = live_count(&store);
    let out = tool_retract_source(
        &mut store,
        &serde_json::json!({
            "source": LEGACY, "repair": "aegis-rz75m6", "apply": true, "expect": 2,
            // A caller attempting to smuggle an assertion in, under the
            // canonical producer's identity.
            "turtle": "@prefix ex: <http://example.org/> . ex:forged a ex:CodeModule .",
            "snapshot": CANONICAL,
            "replace_snapshot": true
        }),
    )
    .unwrap();
    assert_eq!(out["retracted"], 2);
    assert_eq!(
        live_count(&store),
        before - 2,
        "exactly the retraction landed; nothing was asserted"
    );
    assert!(
        store.lookup("http://example.org/forged").unwrap().is_none(),
        "the smuggled entity must not exist"
    );
    // And nothing was written under the canonical producer's key.
    let forged_under_canonical = store
        .list_transactions()
        .unwrap()
        .into_iter()
        .filter(|t| t.id == out["tx_id"].as_i64().unwrap())
        .any(|t| t.source.as_deref() == Some(CANONICAL));
    assert!(!forged_under_canonical);
}

#[test]
fn apply_refuses_without_expect_and_names_the_count() {
    let mut store = store_with_two_producers();
    let before = live_count(&store);
    let err = tool_retract_source(
        &mut store,
        &serde_json::json!({"source": LEGACY, "repair": "t", "apply": true}),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("expect"), "{err}");
    assert!(err.contains('2'), "the refusal names the plan count: {err}");
    assert_eq!(live_count(&store), before, "a refused apply writes nothing");
}

#[test]
fn apply_refuses_a_stale_or_mistyped_expect() {
    let mut store = store_with_two_producers();
    let before = live_count(&store);
    let err = tool_retract_source(
        &mut store,
        &serde_json::json!({
            "source": LEGACY, "repair": "t", "apply": true, "expect": 99
        }),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("99") && err.contains('2'), "{err}");
    assert_eq!(live_count(&store), before);
}

#[test]
fn source_and_repair_are_both_required_and_must_be_non_empty() {
    let mut store = store_with_two_producers();
    for input in [
        serde_json::json!({"repair": "t"}),
        serde_json::json!({"source": "   ", "repair": "t"}),
        serde_json::json!({"source": LEGACY}),
        serde_json::json!({"source": LEGACY, "repair": ""}),
    ] {
        assert!(
            tool_retract_source(&mut store, &input).is_err(),
            "must refuse {input}"
        );
    }
}

#[test]
fn dedupe_collapses_duplicate_rows_to_one_retraction_per_triple() {
    // `facts` PRIMARY KEY is (e, a, v, tx), so two retraction datums for one
    // triple in a single transaction fail with a UNIQUE constraint error. The
    // live graph HAS such duplicates — 86 groups over 29 entities, aegis-a0ne —
    // and they are on the oldest, most-re-asserted entities, which is the same
    // population this tool repairs.
    //
    // This tests the function, deliberately, and NOT end-to-end: `stage_and_guard`
    // skips an assertion whose (e, a, v) is already live in the graph, so no
    // fixture built through the assert path can produce a duplicated plan. An
    // end-to-end test here PASSES WITH THE DEDUPE REMOVED — measured — which
    // would make it a guard that cannot fail.
    let dup = |e: i64, a: i64, v: &str| Datum {
        entity: e,
        attribute: a,
        value: Value::Str(v.to_string()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Retract,
    };
    let plan = vec![
        dup(1, 2, "same"),
        dup(1, 2, "same"),
        dup(1, 2, "same"),
        dup(1, 2, "different"),
        dup(9, 2, "same"),
    ];
    let out = super::repair::dedupe(plan);
    assert_eq!(
        out.len(),
        3,
        "one datum per distinct (entity, attribute, value)"
    );
    assert_eq!(out[0].value, Value::Str("same".into()));
    assert_eq!(out[1].value, Value::Str("different".into()));
    assert_eq!(out[2].entity, 9);
}

#[test]
fn an_unknown_graph_is_refused_rather_than_silently_targeting_root() {
    let mut store = store_with_two_producers();
    let before = live_count(&store);
    let err = tool_retract_source(
        &mut store,
        &serde_json::json!({
            "source": LEGACY, "repair": "t", "graph": "http://example.org/nope"
        }),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("unknown graph"), "{err}");
    assert_eq!(live_count(&store), before);
}

#[test]
fn the_sample_is_bounded_and_says_when_it_is_truncated() {
    let mut store = Store::open_in_memory().unwrap();
    let mut turtle = String::from("@prefix ex: <http://example.org/> .\n");
    for i in 0..30 {
        turtle.push_str(&format!("ex:m{i} a ex:CodeModule .\n"));
    }
    seed(&mut store, &turtle, LEGACY);
    let out = tool_retract_source(
        &mut store,
        &serde_json::json!({"source": LEGACY, "repair": "t"}),
    )
    .unwrap();
    assert_eq!(out["planned"], 30);
    assert_eq!(out["sample"].as_array().unwrap().len(), 20);
    assert_eq!(out["sample_truncated"], true);
}
