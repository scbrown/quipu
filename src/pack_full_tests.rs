//! Contract 1: the `--full` pack asserts LOSSLESS.
//!
//! sattler's design contract is explicit that this is a DIFFERENT test from the
//! published pack's scrub test, and that collapsing them into one over both
//! passes while shipping either wrongly. So nothing here asserts anything about
//! the scrub, and nothing in `pack_tests.rs` asserts anything about losslessness.

use super::*;
use crate::types::{Op, Value};

const TS: &str = "2026-09-11T00:00:00Z";

fn tmp(name: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("quipu-full-{name}-"))
        .tempdir()
        .unwrap();
    let path = dir
        .path()
        .join("out.qpack.db")
        .to_string_lossy()
        .into_owned();
    (dir, path)
}

/// A store with REAL HISTORY: an entity that is asserted and then retracted.
///
/// The retraction is the whole point. A current-facts export drops it AND the
/// term it referenced, so "was there ever a bob?" becomes unanswerable — which
/// is the loss that falls exactly on what somebody decided to remove.
fn store_with_history() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let g = store.overlay_create("urn:g:full", 0).unwrap();
    let alice = store.intern("http://example.org/alice").unwrap();
    let bob = store.intern("http://example.org/bob").unwrap();
    let role = store.intern("http://example.org/role").unwrap();
    store
        .overlay_write(
            g,
            Op::Assert,
            alice,
            role,
            Value::Str("principal".into()),
            TS,
        )
        .unwrap();
    store
        .overlay_write(g, Op::Assert, bob, role, Value::Str("deputy".into()), TS)
        .unwrap();
    store
        .overlay_write(g, Op::Retract, bob, role, Value::Str("deputy".into()), TS)
        .unwrap();
    store
}

fn counts(conn: &rusqlite::Connection) -> std::collections::BTreeMap<String, i64> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    names
        .into_iter()
        .map(|n| {
            let c: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{n}\""), [], |r| r.get(0))
                .unwrap();
            (n, c)
        })
        .collect()
}

fn internal() -> PackOptions {
    PackOptions {
        destination: ShareDestination::Internal,
        ..Default::default()
    }
}

#[test]
fn the_full_pack_carries_every_row_of_every_carried_table() {
    let store = store_with_history();
    let before = counts(&store.conn);

    // ANTI-VACUITY: the fixture must actually HAVE history, or "nothing was
    // lost" is true of a store with nothing to lose. Derived FROM the fixture
    // rather than hardcoded — an earlier draft pasted 6/10/6 from a probe built
    // on a different fixture, which is how a number stops describing the thing
    // it is asserted about.
    let retractions: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM facts WHERE op = 1", [], |r| r.get(0))
        .unwrap();
    assert!(retractions > 0, "fixture must carry retraction history");
    assert!(
        before["facts"] > 1,
        "fixture must carry more than one fact row"
    );
    assert!(
        before["transactions"] > 1,
        "fixture must carry more than one transaction"
    );

    let (_d, out) = tmp("lossless");
    pack_full(&store, &out, &internal(), TS).unwrap();
    let packed = rusqlite::Connection::open(&out).unwrap();
    let after = counts(&packed);

    let excluded_set = excluded();
    let mut lost = Vec::new();
    for (table, n) in &before {
        if excluded_set.contains(&table.as_str()) {
            continue;
        }
        let got = after.get(table).copied().unwrap_or(-1);
        if got != *n {
            lost.push(format!("{table}: {n} -> {got}"));
        }
    }
    assert!(
        lost.is_empty(),
        "the --full pack is NOT lossless; carried table(s) changed row count: {lost:?}. \
         Verified to discriminate: aiming this same assertion at the published \
         `pack()` reports facts/terms/transactions all shrinking, which is the \
         behaviour this artifact exists NOT to have (aegis-9f899e contract 1)."
    );

    // Named explicitly as well as by the loop, because these three ARE the
    // contract and a loop over a map would still pass if the map were empty.
    // Compared against the SOURCE, which is what "lossless" means.
    assert_eq!(
        after["facts"], before["facts"],
        "every fact row, not just current ones"
    );
    assert_eq!(
        after["terms"], before["terms"],
        "the retracted entity's IRI must survive"
    );
    assert_eq!(
        after["transactions"], before["transactions"],
        "who wrote it must survive"
    );
}

#[test]
fn the_full_pack_carries_the_retraction_itself_not_just_the_row_count() {
    // Row counts can agree while the rows are wrong. This asserts the columns
    // that the published pack's re-intern discards: `op`, and the transaction
    // linkage. `pack_into` hardcodes `op: Op::Assert`, so a retraction arriving
    // as an assertion would keep every count identical and invert the meaning.
    let store = store_with_history();
    let (_d, out) = tmp("retraction");
    pack_full(&store, &out, &internal(), TS).unwrap();
    let packed = rusqlite::Connection::open(&out).unwrap();

    let retractions: i64 = packed
        .query_row("SELECT COUNT(*) FROM facts WHERE op = 1", [], |r| r.get(0))
        .unwrap();
    let source_retractions: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM facts WHERE op = 1", [], |r| r.get(0))
        .unwrap();
    assert!(
        source_retractions > 0,
        "fixture has no retraction — this test would prove nothing"
    );
    assert_eq!(
        retractions, source_retractions,
        "the pack lost the RETRACTION while keeping the row count: op was \
         rewritten, which is exactly what re-interning through the write path does"
    );

    let retracted_tx: i64 = packed
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE retracted_tx IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        retracted_tx > 0,
        "no row carries retracted_tx — the pack cannot say WHEN something was removed"
    );
}

#[test]
fn the_full_pack_prunes_exactly_the_declared_exclusions() {
    // The other direction, and it is what makes "lossless" honest rather than
    // absolute: the artifact is lossless with respect to a DECLARED set, so the
    // exclusions must actually be gone.
    let store = store_with_history();
    let (_d, out) = tmp("pruned");
    pack_full(&store, &out, &internal(), TS).unwrap();
    let packed = rusqlite::Connection::open(&out).unwrap();

    // ANTI-VACUITY: the prune list must be non-empty, or "all exclusions are
    // empty" is a statement about nothing.
    assert!(
        excluded().len() >= 5,
        "the declared exclusion set looks empty"
    );

    for table in excluded() {
        let exists: bool = packed
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")
            .unwrap()
            .exists([table])
            .unwrap();
        if exists {
            let n: i64 = packed
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(
                n, 0,
                "{table} is declared Excluded but the full pack carries {n} row(s). \
                 For this group the declared set is a SECURITY boundary, not a \
                 convenience (docs/design/standard-share-artifact.md)."
            );
        }
    }
}

#[test]
fn a_full_pack_bound_outward_is_refused() {
    // The one-way door. Publishing an events-inclusive pack is held for the
    // operator, so the full path must not acquire a publish route by
    // convenience — enforced rather than documented.
    let store = store_with_history();
    let (_d, out) = tmp("outward");
    let error = pack_full(&store, &out, &PackOptions::default(), TS)
        .expect_err("a full pack bound outward must be refused");
    let text = error.to_string();
    assert!(
        text.contains("INTERNAL ONLY"),
        "the refusal must say what it refused: {text}"
    );
    assert!(
        text.contains("--destination internal"),
        "the refusal must name the escape hatch: {text}"
    );
    assert!(
        !std::path::Path::new(&out).exists(),
        "a refused full pack left an artifact behind"
    );
}
