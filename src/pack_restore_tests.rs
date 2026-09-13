//! The verb split: which verb reads which pack, and in what ORDER it decides.
//!
//! Deliberately separate from `pack_full_tests.rs` (which asserts LOSSLESS) and
//! from `pack_tests.rs` (which asserts the scrub), for the reason sattler gave
//! about contracts 1 and 2: one test over two contracts passes while shipping
//! either of them wrongly.
//!
//! The ordering test is the one that matters most here. Every refusal below is
//! individually obvious; what is NOT obvious, and what a suite of
//! one-refusal-per-test would never catch, is that a pack can fail TWO checks at
//! once and the operator must be told about the one they can act on.

use super::*;
use crate::pack::PackOptions;
use crate::share_scrub::ShareDestination;
use crate::store::Store;
use crate::types::{Op, Value};

const TS: &str = "2026-09-13T00:00:00Z";
const GRAPH: &str = "urn:g:restore";

fn tmpdir(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("quipu-restore-{name}-"))
        .tempdir()
        .unwrap()
}

fn at(dir: &tempfile::TempDir, leaf: &str) -> String {
    dir.path().join(leaf).to_string_lossy().into_owned()
}

fn internal() -> PackOptions {
    PackOptions {
        destination: ShareDestination::Internal,
        ..Default::default()
    }
}

/// A store with real history, so a lossy restore is detectable at all.
fn store_with_history() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let g = store.overlay_create(GRAPH, 0).unwrap();
    let alice = store.intern("http://example.org/alice").unwrap();
    let bob = store.intern("http://example.org/bob").unwrap();
    let role = store.intern("http://example.org/role").unwrap();
    for (e, v) in [(alice, "principal"), (bob, "deputy")] {
        store
            .overlay_write(g, Op::Assert, e, role, Value::Str(v.into()), TS)
            .unwrap();
    }
    store
        .overlay_write(g, Op::Retract, bob, role, Value::Str("deputy".into()), TS)
        .unwrap();
    store
}

fn full_pack(dir: &tempfile::TempDir, leaf: &str) -> String {
    let store = store_with_history();
    let out = at(dir, leaf);
    crate::pack_full::pack_full(&store, &out, &internal(), TS).unwrap();
    out
}

fn published_pack(dir: &tempfile::TempDir, leaf: &str) -> String {
    let store = store_with_history();
    let out = at(dir, leaf);
    crate::pack::pack(&store, GRAPH, &out, &PackOptions::default(), TS).unwrap();
    out
}

/// Overwrite a pack's declared format, to reach formats no producer emits.
fn restamp_format(pack: &str, format: &str) {
    let conn = Connection::open(pack).unwrap();
    conn.execute(
        "UPDATE pack_manifest SET pack_format = ?1 WHERE id = 1",
        rusqlite::params![format],
    )
    .unwrap();
}

/// Remove one fact row, so a pack fails its own content hash.
///
/// A DELETE rather than a byte-flip on purpose: writing `x'00'` into `v`
/// makes a `Ref` payload unparseable, so the PUBLISHED pack's hash path dies
/// with `bad ref length` before it can disagree about content — which tests
/// the parser, not the hash. Removing a row leaves every remaining row valid
/// and changes only what the hash is over, which is the fault this fixture is
/// supposed to inject.
fn corrupt_a_fact(pack: &str) {
    let conn = Connection::open(pack).unwrap();
    let removed = conn
        .execute(
            "DELETE FROM facts WHERE rowid = (SELECT MIN(rowid) FROM facts)",
            [],
        )
        .unwrap();
    assert_eq!(removed, 1, "the fixture must actually remove a row");
}

/// Every table's rows, sorted, so two stores can be compared independently of
/// the hash the producer computes. A round trip checked only by re-running the
/// producer's own hash is checked by the thing under test.
fn rows_by_table(path: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let conn = Connection::open(path).unwrap();
    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    let mut out = std::collections::BTreeMap::new();
    for table in tables {
        let cols: Vec<String> = {
            let mut c = conn
                .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                .unwrap();
            c.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        if cols.is_empty() {
            continue;
        }
        let expr = cols
            .iter()
            .map(|c| format!("quote(\"{c}\")"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let mut stmt = conn
            .prepare(&format!("SELECT {expr} FROM \"{table}\""))
            .unwrap();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| r.get::<_, Option<String>>(0))
            .unwrap()
            .filter_map(std::result::Result::unwrap)
            .collect();
        rows.sort();
        out.insert(table, rows);
    }
    out
}

#[test]
fn unpack_refuses_a_full_pack_and_names_restore() {
    let dir = tmpdir("wrong-verb-unpack");
    let pack = full_pack(&dir, "full.qpack");
    let manifest = crate::pack::read_manifest(&pack).unwrap();

    let error = require_format(&manifest, Verb::Unpack)
        .expect_err("unpack must refuse a full pack — it MERGES, and a full pack REPLACES");
    let text = error.to_string();

    assert!(
        text.contains("quipu restore"),
        "the refusal must name the verb that DOES read it; the operator's next \
         action is to retype the command: {text}"
    );
    assert!(
        text.contains("intact"),
        "the refusal must say the artifact is fine, or it reads as corruption \
         and sends the operator debugging a healthy backup: {text}"
    );
}

#[test]
fn restore_refuses_a_published_pack_and_names_unpack() {
    let dir = tmpdir("wrong-verb-restore");
    let pack = published_pack(&dir, "pub.qpack");
    let dest = at(&dir, "dest.db");

    let error = restore(&pack, &dest, false)
        .expect_err("restore must refuse a published pack — it would REPLACE, not merge");
    let text = error.to_string();

    assert!(
        text.contains("quipu unpack"),
        "the refusal must name `quipu unpack`: {text}"
    );
    assert!(
        !std::path::Path::new(&dest).exists(),
        "a refused restore must not create the destination"
    );
}

#[test]
fn an_unknown_pack_format_is_refused_by_both_verbs() {
    let dir = tmpdir("unknown-format");
    let pack = full_pack(&dir, "future.qpack");
    restamp_format(&pack, "quipu-pack-full/99");
    let manifest = crate::pack::read_manifest(&pack).unwrap();

    for verb in [Verb::Unpack, Verb::Restore] {
        let text = require_format(&manifest, verb)
            .expect_err("an unrecognised format must be refused, never guessed at")
            .to_string();
        assert!(
            text.contains("quipu-pack-full/99"),
            "the refusal must quote the format it could not read: {text}"
        );
    }
}

#[test]
fn an_export_only_format_is_refused_and_says_nothing_reads_it() {
    let dir = tmpdir("export-only");
    let pack = full_pack(&dir, "bundle.qpack");
    restamp_format(&pack, FORMAT_TURTLE);
    let manifest = crate::pack::read_manifest(&pack).unwrap();

    // "Nothing reads this" and "I have never heard of this" are different
    // answers, and only one of them is a reason to look for another verb.
    let text = require_format(&manifest, Verb::Unpack)
        .expect_err("an interop bundle is export-only")
        .to_string();
    assert!(
        text.contains("Nothing loads it back"),
        "an export-only format must say so rather than name a verb that cannot \
         help either: {text}"
    );
}

#[test]
fn a_frozen_archive_is_pointed_at_attach_not_at_the_other_load_verb() {
    let dir = tmpdir("frozen");
    let pack = full_pack(&dir, "frozen.qpack");
    restamp_format(&pack, FORMAT_FROZEN);
    let manifest = crate::pack::read_manifest(&pack).unwrap();

    let text = require_format(&manifest, Verb::Restore)
        .expect_err("a frozen archive graph is composed, not restored")
        .to_string();
    assert!(
        text.contains("attach"),
        "a frozen archive must be pointed at `quipu db attach`: {text}"
    );
}

/// THE ORDERING TEST, through the REAL entry points.
///
/// A pack that is both the wrong format AND corrupt must report the WRONG VERB,
/// because that is the fact the operator can act on. This is the test the module
/// exists for: every other refusal here passes with the checks in either order.
/// A hash-first implementation tells someone holding a byte-perfect backup that
/// it is damaged — measured before the fix as `unknown graph:
/// urn:quipu:whole-store` on an intact pack (aegis-9f899e).
///
/// It calls `unpack_verified` and `restore` rather than `require_format`,
/// because `require_format` never consults a hash: asserting the order against
/// the gate alone would pass in both worlds and prove nothing. The first version
/// of this test did exactly that.
#[test]
fn the_format_gate_decides_before_the_hash_does() {
    let dir = tmpdir("order");
    let dest = at(&dir, "dest.db");

    // ARM 1: a FULL pack, corrupted, handed to `unpack`.
    let full = full_pack(&dir, "both-wrong.qpack");
    corrupt_a_fact(&full);
    let (_c, _r, matches) = crate::pack_full::verify_full(&full).unwrap();
    assert!(!matches, "fixture must be corrupt or this test is vacuous");

    let text = crate::pack_load::unpack_verified(&full, &dest, &Default::default(), TS)
        .expect_err("wrong verb AND corrupt: the wrong verb is the actionable one")
        .to_string();
    assert!(
        text.contains("quipu restore"),
        "unpack must report the wrong verb, not the hash: {text}"
    );
    assert!(
        !text.contains("HASH MISMATCH") && !text.contains("unknown graph"),
        "the hash must not have been consulted yet — and `unknown graph` is the \
         pre-fix symptom, which means the gate did not run first: {text}"
    );

    // ARM 2: a PUBLISHED pack, corrupted, handed to `restore`.
    let published = published_pack(&dir, "pub-wrong.qpack");
    corrupt_a_fact(&published);
    let (_c2, _r2, matches2) = crate::pack::verify(&published).unwrap();
    assert!(!matches2, "fixture must be corrupt or this arm is vacuous");

    let text2 = restore(&published, &dest, false)
        .expect_err("restore must refuse the published pack on its FORMAT")
        .to_string();
    assert!(
        text2.contains("quipu unpack"),
        "restore must report the wrong verb, not the hash: {text2}"
    );
    assert!(
        !text2.contains("HASH MISMATCH"),
        "the hash must not have been consulted yet: {text2}"
    );
    assert!(
        !std::path::Path::new(&dest).exists(),
        "neither refusal may create the destination"
    );
}

#[test]
fn restore_reproduces_every_row_of_every_table() {
    let dir = tmpdir("round-trip");
    let pack = full_pack(&dir, "src.qpack");
    let dest = at(&dir, "restored.db");

    let report = restore(&pack, &dest, false).expect("a full pack must restore into a fresh store");
    assert_eq!(
        report.replaced_facts, 0,
        "a fresh destination destroys nothing"
    );

    let packed = rows_by_table(&pack);
    let restored = rows_by_table(&dest);
    assert!(
        packed["facts"].len() > 2,
        "fixture must carry history for this comparison to be worth making"
    );
    for (table, rows) in &packed {
        // `pack_manifest` describes the ARTIFACT; the restored store carries it
        // as lineage and it is excluded from the pack's own content hash too.
        if table == "pack_manifest" {
            continue;
        }
        assert_eq!(
            restored.get(table),
            Some(rows),
            "table {table} did not survive the restore intact"
        );
    }

    // And the producer agrees: re-packing the restored store reproduces the
    // same content hash. Checked SECOND, because on its own it only proves the
    // hash is stable, not that the rows are there.
    let repacked = at(&dir, "again.qpack");
    let store = Store::open(&dest).unwrap();
    let again = crate::pack_full::pack_full(&store, &repacked, &internal(), TS).unwrap();
    assert_eq!(
        again.content_hash, report.content_hash,
        "pack --full -> restore -> pack --full must be identity"
    );
}

#[test]
fn restore_refuses_a_populated_destination_unless_forced() {
    let dir = tmpdir("populated");
    let pack = full_pack(&dir, "src.qpack");
    let dest = at(&dir, "live.db");

    // A destination with live facts of its own.
    {
        let mut store = Store::open(&dest).unwrap();
        let g = store.overlay_create("urn:g:live", 0).unwrap();
        let e = store.intern("http://example.org/keepme").unwrap();
        let a = store.intern("http://example.org/role").unwrap();
        store
            .overlay_write(g, Op::Assert, e, a, Value::Str("precious".into()), TS)
            .unwrap();
    }

    let text = restore(&pack, &dest, false)
        .expect_err("restore must not silently destroy a populated store")
        .to_string();
    assert!(
        text.contains("--force"),
        "the refusal must name the override: {text}"
    );
    assert!(
        rows_by_table(&dest)["facts"].len() == 1,
        "the refused restore must leave the destination untouched"
    );

    // --force is the DECLARED intent, and then it does replace.
    let report = restore(&pack, &dest, true).expect("--force must proceed");
    assert_eq!(
        report.replaced_facts, 1,
        "the report must say what was destroyed, so it is visible afterwards"
    );
    assert_eq!(
        rows_by_table(&dest)["facts"],
        rows_by_table(&pack)["facts"],
        "a forced restore installs the pack's facts"
    );
}

#[test]
fn restore_refuses_a_tampered_pack_and_writes_nothing() {
    let dir = tmpdir("tampered");
    let pack = full_pack(&dir, "tampered.qpack");
    corrupt_a_fact(&pack);
    let dest = at(&dir, "dest.db");

    let text = restore(&pack, &dest, false)
        .expect_err("a pack whose contents do not match its manifest must not be installed")
        .to_string();
    assert!(text.contains("HASH MISMATCH"), "{text}");
    assert!(
        !std::path::Path::new(&dest).exists(),
        "the hash check must run BEFORE the destructive write"
    );
}

#[test]
fn verify_full_answers_a_question_pack_verify_cannot() {
    let dir = tmpdir("verify");
    let pack = full_pack(&dir, "full.qpack");

    let (claimed, recomputed, matches) = crate::pack_full::verify_full(&pack).unwrap();
    assert!(
        matches,
        "an untouched full pack must verify: {claimed} vs {recomputed}"
    );

    // The control that makes the line above mean something: the GENERIC verify
    // cannot answer this at all, because it hashes current-facts content for
    // `manifest.source_graph` and a full pack's is a sentinel. Before the
    // format dispatch this was the only answer a full pack could get, and it
    // reads as a missing graph rather than as the wrong function.
    let generic = crate::pack::verify(&pack);
    assert!(
        generic.is_err(),
        "if pack::verify ever handles a full pack, the dispatch in cli_pack \
         and the reason for verify_full both need revisiting"
    );
}
