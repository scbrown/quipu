//! Contract 3: the `--full --format text` pack asserts LOSSLESS **as text**.
//!
//! Deliberately separate from `pack_full_tests.rs` (lossless, binary) and
//! `pack_tests.rs` (the scrub), for the reason sattler's design contract gives:
//! one test over two contracts passes while shipping either one wrongly.
//!
//! The acceptance criterion is Stiwi's own wording on aegis-9f899e — *unpack ==
//! identical sqlite contents* — so every test here ends at a content hash
//! comparison against an independently packed source, never at "the command
//! exited 0".

use super::*;
use crate::pack::PackOptions;
use crate::share_scrub::ShareDestination;
use crate::store::Store;
use crate::types::{Op, Value};

const TS: &str = "2026-09-14T00:00:00Z";

fn internal() -> PackOptions {
    PackOptions {
        destination: ShareDestination::Internal,
        ..Default::default()
    }
}

/// A store with REAL HISTORY *and* a value containing the hash's delimiter.
///
/// Both halves are load-bearing. The retraction is what a current-facts export
/// loses, so it separates lossless from lossy. The `|` is the wire-format
/// hazard: the canonical form the hash uses joins columns with `'|'`, and a
/// format that reused that as its delimiter would reconstruct this row wrongly
/// — only for stores that happen to contain a pipe, which is exactly the bug a
/// fixture without one cannot catch.
fn store_with_history_and_a_pipe() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let g = store.overlay_create("urn:g:text", 0).unwrap();
    let alice = store.intern("http://example.org/alice").unwrap();
    let bob = store.intern("http://example.org/bob").unwrap();
    let role = store.intern("http://example.org/role").unwrap();
    store
        .overlay_write(
            g,
            Op::Assert,
            alice,
            role,
            Value::Str("pipe | inside | value".into()),
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

fn tmpdir(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("quipu-text-{name}-"))
        .tempdir()
        .unwrap()
}

/// Pack `store` as text into a fresh directory, returning (dir, pack path).
fn packed(name: &str, store: &Store) -> (tempfile::TempDir, String) {
    packed_with_options(name, store, &internal())
}

fn packed_with_options(
    name: &str,
    store: &Store,
    opts: &PackOptions,
) -> (tempfile::TempDir, String) {
    let dir = tmpdir(name);
    let out = dir.path().join("pack.d").to_string_lossy().into_owned();
    pack_full_text(store, &out, opts, TS).unwrap();
    (dir, out)
}

#[test]
fn text_in_identical_store_contents_out() {
    let store = store_with_history_and_a_pipe();

    // ANTI-VACUITY: "nothing was lost" is trivially true of a store with
    // nothing to lose. Derived from the fixture rather than hardcoded.
    let retractions: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE op = ?1",
            [Op::Retract as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert!(retractions > 0, "fixture must carry retraction history");

    let (dir, out) = packed("roundtrip", &store);
    let rebuilt = dir.path().join("rebuilt.db").to_string_lossy().into_owned();
    let (manifest, recomputed) = rebuild(&out, &rebuilt).unwrap();

    assert_eq!(
        recomputed, manifest.content_hash,
        "the reconstruction must hash identically to the store it was cut from"
    );
}

#[test]
fn a_value_containing_the_hash_delimiter_survives_the_round_trip() {
    let store = store_with_history_and_a_pipe();
    let (dir, out) = packed("delimiter", &store);
    let rebuilt = dir.path().join("rebuilt.db").to_string_lossy().into_owned();
    rebuild(&out, &rebuilt).unwrap();

    let pipes = |conn: &rusqlite::Connection| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM facts \
             WHERE CAST(v AS TEXT) LIKE '%pipe | inside | value%'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let values = |conn: &rusqlite::Connection| -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT quote(v) FROM facts ORDER BY quote(v)")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect()
    };

    // ANTI-VACUITY: if the fixture does not actually contain the delimiter,
    // this test passes without ever exercising the hazard it is named for.
    assert_eq!(
        pipes(&store.conn),
        1,
        "fixture must contain a value with the hash delimiter in it"
    );

    let conn = rusqlite::Connection::open(&rebuilt).unwrap();
    assert_eq!(
        pipes(&conn),
        1,
        "a value containing '|' must reconstruct intact; splitting on the \
         hash's delimiter would corrupt it"
    );
    assert_eq!(
        values(&store.conn),
        values(&conn),
        "every fact value must reconstruct byte-for-byte"
    );
}

#[test]
fn a_dropped_row_makes_the_reconstruction_refuse() {
    let store = store_with_history_and_a_pipe();
    let (dir, out) = packed("dropped-row", &store);

    let facts = std::path::Path::new(&out).join(DATA_DIR).join("facts.sql");
    let text = std::fs::read_to_string(&facts).unwrap();
    let mut inserts: Vec<&str> = text.lines().filter(|l| l.starts_with("INSERT")).collect();
    let before = inserts.len();
    inserts.pop();
    assert!(
        before > inserts.len(),
        "sabotage must actually remove a row"
    );
    std::fs::write(&facts, inserts.join("\n") + "\n").unwrap();

    let rebuilt = dir.path().join("rebuilt.db").to_string_lossy().into_owned();
    let (manifest, recomputed) = rebuild(&out, &rebuilt).unwrap();
    assert_ne!(
        recomputed, manifest.content_hash,
        "a pack missing a row must NOT hash as the store it claims to be"
    );
}

#[test]
fn a_dropped_table_file_makes_the_reconstruction_refuse() {
    // The "someone forgot a table" case `pack_full` warns about: an
    // enumerate-and-serialise format is lossy the moment a table goes missing,
    // and the whole reason this one is built on the pruned COPY is so that the
    // omission changes the hash rather than passing quietly.
    let store = store_with_history_and_a_pipe();
    let (dir, out) = packed("dropped-table", &store);

    let facts = std::path::Path::new(&out).join(DATA_DIR).join("facts.sql");
    std::fs::remove_file(&facts).unwrap();
    assert!(!facts.exists(), "sabotage must actually remove the file");

    let rebuilt = dir.path().join("rebuilt.db").to_string_lossy().into_owned();
    let (manifest, recomputed) = rebuild(&out, &rebuilt).unwrap();
    assert_ne!(
        recomputed, manifest.content_hash,
        "a pack missing a whole table must NOT hash as the store it claims to be"
    );
}

#[test]
fn a_dangling_reference_is_refused_by_name() {
    // The integrity gate the foreign-key deferral owes. Without it, replay
    // order alone would have to imply referential integrity, and a dump missing
    // a PARENT row would install a store that is self-consistently hashed and
    // referentially broken.
    let store = store_with_history_and_a_pipe();
    let (dir, out) = packed("dangling", &store);

    let txs = std::path::Path::new(&out)
        .join(DATA_DIR)
        .join("transactions.sql");
    let text = std::fs::read_to_string(&txs).unwrap();
    let mut inserts: Vec<&str> = text.lines().filter(|l| l.starts_with("INSERT")).collect();
    assert!(
        inserts.len() > 1,
        "fixture must have more than one transaction for this sabotage to bite"
    );
    inserts.pop();
    std::fs::write(&txs, inserts.join("\n") + "\n").unwrap();

    let rebuilt = dir.path().join("rebuilt.db").to_string_lossy().into_owned();
    let err = rebuild(&out, &rebuilt).unwrap_err().to_string();
    assert!(
        err.contains("referential integrity") && err.contains("facts"),
        "a dangling reference must be refused BY NAME, got: {err}"
    );
}

#[test]
fn an_outward_destination_is_refused() {
    // Same one-way door as `pack --full`: this artifact carries `events` and
    // every operational table, so publishing one is the operator's decision and
    // must not be acquired by convenience.
    let store = store_with_history_and_a_pipe();
    let dir = tmpdir("outward");
    let out = dir.path().join("pack.d").to_string_lossy().into_owned();
    let opts = PackOptions {
        destination: ShareDestination::Outward,
        ..Default::default()
    };
    let err = pack_full_text(&store, &out, &opts, TS)
        .unwrap_err()
        .to_string();
    assert!(err.contains("INTERNAL ONLY"), "got: {err}");
    assert!(
        !std::path::Path::new(&out).exists(),
        "a refused pack must expose no output directory"
    );
}

#[test]
fn the_manifest_declares_the_text_format_so_the_gate_can_route_it() {
    let store = store_with_history_and_a_pipe();
    let (_dir, out) = packed("format", &store);
    let manifest = read_manifest_dir(&out).unwrap();
    assert_eq!(manifest.pack_format, FORMAT_FULL_TEXT);
    assert!(is_text_pack(&out), "a text pack must be recognised as one");
}

/// A fixture that actually HAS vectors.
///
/// Every other fixture here has `vectors: 0`, and that is exactly why the
/// regenerated-set decision went unnoticed for a whole implementation: a pack
/// that drops nothing looks identical to one that drops the right thing.
fn store_with_vectors() -> Store {
    let store = store_with_history_and_a_pipe();
    store
        .conn
        .execute(
            "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
             VALUES (1, 'alice', X'0102030405060708', ?1)",
            [TS],
        )
        .unwrap();
    store
        .conn
        .execute(
            "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
             VALUES (2, 'bob', X'0807060504030201', ?1)",
            [TS],
        )
        .unwrap();
    store
}

#[test]
fn vectors_are_not_transported_and_the_recipe_is() {
    // sattler's ruling, 2026-09-14, and `DECLARED`'s own note at the entry:
    // "~2.2 GB of floats at homelab scale, which rules out text." Inlining them
    // would hex-double a 2.2 GB table into a 4-5 GB "git-friendly" artifact,
    // which fails the half of the directive that says git-friendly.
    let store = store_with_vectors();

    // ANTI-VACUITY: a pack that drops nothing passes this test on a fixture
    // with nothing to drop, which is how the original implementation shipped
    // carrying vectors without anyone noticing.
    let before: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, 2, "fixture must actually carry vectors");

    let (dir, out) = packed_with_options(
        "vectors",
        &store,
        &PackOptions {
            allow_missing_embedding_recipe: true,
            ..internal()
        },
    );

    let data = std::path::Path::new(&out)
        .join(DATA_DIR)
        .join("vectors.sql");
    if data.exists() {
        let text = std::fs::read_to_string(&data).unwrap();
        assert!(
            !text.contains("INSERT"),
            "the text pack must not inline vector rows"
        );
    }

    let manifest = read_manifest_dir(&out).unwrap();
    let counts: serde_json::Value = serde_json::from_str(&manifest.counts).unwrap();
    assert_eq!(
        counts["regenerated"],
        serde_json::json!(["vectors"]),
        "the pack must DECLARE what it did not carry"
    );
    assert_eq!(
        counts["regenerated_source_counts"]["vectors"],
        serde_json::json!(2),
        "the source count must survive the prune that zeroes the table"
    );

    // And the reconstruction is still exact over what the pack does carry.
    let rebuilt = dir.path().join("rebuilt.db").to_string_lossy().into_owned();
    let (m, recomputed) = rebuild(&out, &rebuilt).unwrap();
    assert_eq!(recomputed, m.content_hash);

    let conn = rusqlite::Connection::open(&rebuilt).unwrap();
    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, 0, "vectors are rebuilt by the consumer, not shipped");
}

#[test]
fn a_restore_that_must_rebuild_says_so() {
    // The failure this prevents is a store that looks fully restored and
    // silently answers vector search with nothing.
    let store = store_with_vectors();
    let (_dir, out) = packed_with_options(
        "notice",
        &store,
        &PackOptions {
            allow_missing_embedding_recipe: true,
            ..internal()
        },
    );
    let manifest = read_manifest_dir(&out).unwrap();

    let notice = regeneration_notice(&manifest.counts)
        .expect("a pack that dropped vectors must report that they need rebuilding");
    assert!(notice.contains("vectors"), "got: {notice}");
    assert!(
        notice.contains('2'),
        "the row count belongs in the notice: {notice}"
    );
}

#[test]
fn a_pack_with_nothing_to_rebuild_stays_silent() {
    // The other half, and the reason the notice is an Option: a reassurance
    // printed unconditionally is one nobody checks.
    let counts = serde_json::json!({"regenerated": []}).to_string();
    assert!(regeneration_notice(&counts).is_none());
}
