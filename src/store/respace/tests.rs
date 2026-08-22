//! Tests for `quipu db respace` (quipu #74, acceptance 2/3/5/6).
//!
//! The three that carry the acceptance are
//! [`respace_round_trips_every_query_identically`],
//! [`a_new_column_must_be_classified`] and
//! [`original_is_byte_identical_after_respace`]. The rest exist because a
//! respace fails silently: every one of them asserts something that a plausible
//! wrong implementation would still let pass the round-trip.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{COLUMN_CLASSIFICATION, MAX_SPACE, TermIdKind, id_remap_sql, remap, respace_file};
use crate::schema::SPACE_SIZE;
use crate::store::Store;
use crate::types::Value;
use crate::{Datum, Op};

/// A scratch directory that removes itself. Respace is a file-level operation,
/// so none of this can be done in memory.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "quipu-respace-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const T0: &str = "2026-01-01T00:00:00Z";

fn fact(entity: i64, attribute: i64, value: Value) -> Datum {
    Datum {
        entity,
        attribute,
        value,
        valid_from: T0.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

/// A store with something worth losing: entity references in object position
/// (the `Ref` BLOBs of acceptance 3), a named graph and an overlay bound to it
/// (`graphs.g` and `graphs.parent_branch`), a vector embedding (`vectors
/// .entity_id`, the column the schema enumeration found), and retracted facts.
fn seed_store(path: &Path) -> Vec<(String, String, String)> {
    let mut store = Store::open(&path.to_string_lossy()).unwrap();

    let alice = store.intern("urn:t:alice").unwrap();
    let bob = store.intern("urn:t:bob").unwrap();
    let carol = store.intern("urn:t:carol").unwrap();
    let knows = store.intern("urn:t:knows").unwrap();
    let name = store.intern("urn:t:name").unwrap();

    let datums = vec![
        fact(alice, name, Value::Str("Alice".into())),
        // The pins for acceptance 3: a term id living inside an opaque BLOB.
        fact(alice, knows, Value::Ref(bob)),
        fact(bob, knows, Value::Ref(carol)),
        fact(carol, name, Value::Str("Carol".into())),
    ];
    store.transact(&datums, T0, None, None).unwrap();

    // A named graph and an overlay bound to it, so both `graphs` columns carry
    // a real term id rather than the ROOT sentinel.
    let g = store.intern("urn:t:graph:main").unwrap();
    let ov = store.intern("urn:t:graph:overlay").unwrap();
    store
        .conn
        .execute(
            "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
             VALUES (?1, 'committed', NULL, '2026-01-01T00:00:00Z')",
            rusqlite::params![g],
        )
        .unwrap();
    store
        .conn
        .execute(
            "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
             VALUES (?1, 'overlay', ?2, '2026-01-01T00:00:00Z')",
            rusqlite::params![ov, g],
        )
        .unwrap();
    // A fact in the named graph, so `facts.g` is non-ROOT somewhere.
    store
        .conn
        .execute(
            "UPDATE facts SET g = ?1 WHERE e = ?2 AND a = ?3",
            rusqlite::params![g, carol, name],
        )
        .unwrap();

    // `vectors.entity_id` — the column no comment in the store named.
    store
        .conn
        .execute(
            "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
             VALUES (?1, 'Alice the engineer', ?2, '2026-01-01T00:00:00Z')",
            rusqlite::params![alice, vec![0u8; 16]],
        )
        .unwrap();

    let observed = observe(&store.conn);
    drop(store);
    observed
}

/// The store as a reader sees it: every fact as `(subject, predicate, object)`
/// IRIs and lexical forms. Deliberately resolved to NAMES, because term ids are
/// exactly what respace changes — comparing ids would assert the opposite of
/// what round-tripping means.
fn observe(conn: &Connection) -> Vec<(String, String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT te.iri, ta.iri, f.v, f.g FROM facts f \
             JOIN terms te ON te.id = f.e JOIN terms ta ON ta.id = f.a \
             ORDER BY te.iri, ta.iri",
        )
        .unwrap();
    let mut out: Vec<(String, String, String)> = stmt
        .query_map([], |r| {
            let e: String = r.get(0)?;
            let a: String = r.get(1)?;
            let blob: Vec<u8> = r.get(2)?;
            let g: i64 = r.get(3)?;
            Ok((e, a, (blob, g)))
        })
        .unwrap()
        .map(|row| {
            let (e, a, (blob, g)) = row.unwrap();
            // Resolve a Ref through the term table so the comparison is about
            // what the fact MEANS, not about which integer encodes it.
            let v = match Value::from_bytes(&blob).unwrap() {
                Value::Ref(id) => format!(
                    "ref:{}",
                    conn.query_row(
                        "SELECT iri FROM terms WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap_or_else(|_| format!("DANGLING({id})"))
                ),
                other => format!("{other:?}"),
            };
            let graph = if g == 0 {
                "ROOT".to_string()
            } else {
                conn.query_row(
                    "SELECT iri FROM terms WHERE id = ?1",
                    rusqlite::params![g],
                    |r| r.get::<_, String>(0),
                )
                .unwrap_or_else(|_| format!("DANGLING({g})"))
            };
            (e, a, format!("{v} @{graph}"))
        })
        .collect();
    out.sort();
    out
}

/// A store as it existed before the later migrations: `INIT_SQL` only, terms
/// and facts written straight in, no `migrate_*` ever run.
///
/// This is what respace is actually pointed at — nobody respaces a store they
/// created a moment ago — and it is the only fixture on which "the original was
/// never opened for writing" is a testable claim. On an already-migrated store
/// a read-write open is an idempotent no-op that changes no bytes, so
/// [`original_is_byte_identical_after_respace`] would pass against an
/// implementation that opened the source read-write. Measured by sabotage: it
/// did.
fn seed_legacy_store(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(crate::schema::INIT_SQL).unwrap();
    conn.execute_batch(
        "INSERT INTO terms (iri) VALUES ('urn:l:alice'), ('urn:l:bob'), ('urn:l:knows');
         INSERT INTO transactions (id, timestamp) VALUES (1, '2026-01-01T00:00:00Z');",
    )
    .unwrap();
    // A Ref fact, written with the pre-quad column set (no `g`).
    let bob_blob = Value::Ref(2).to_bytes();
    conn.execute(
        "INSERT INTO facts (e, a, v, tx, valid_from, op) VALUES (1, 3, ?1, 1, ?2, 1)",
        rusqlite::params![bob_blob, T0],
    )
    .unwrap();
    // Close cleanly so the WAL is checkpointed and the file is settled; any
    // later write by respace would therefore be respace's own.
    drop(conn);
}

fn digest(path: &Path) -> Vec<u8> {
    // Content, length and mtime-independent: the bytes are the claim.
    std::fs::read(path).unwrap()
}

// ---------------------------------------------------------------------------
// Acceptance 2 — round-trip
// ---------------------------------------------------------------------------

#[test]
fn respace_round_trips_every_query_identically() {
    let scratch = Scratch::new("roundtrip");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    let before = seed_store(&src);
    assert!(
        before.len() >= 4,
        "the fixture must actually hold facts, or this asserts nothing"
    );

    let report = respace_file(&src, &dst, 7).unwrap();
    assert_eq!(report.from_space, 0);
    assert_eq!(report.to_space, 7);

    let moved = Store::open(&dst.to_string_lossy()).unwrap();
    assert_eq!(moved.local_term_space().unwrap(), 7);
    let after = observe(&moved.conn);

    assert_eq!(
        after, before,
        "every fact must read back identically after a respace"
    );
    assert!(
        !after.iter().any(|(_, _, v)| v.contains("DANGLING")),
        "a reference was left pointing at a term that no longer exists: {after:?}"
    );
}

#[test]
fn every_id_lands_inside_the_destination_space() {
    let scratch = Scratch::new("inspace");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);
    respace_file(&src, &dst, 3).unwrap();

    let conn = Connection::open(&dst).unwrap();
    let lo = 3 * SPACE_SIZE;
    let hi = lo + SPACE_SIZE;
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM terms")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!ids.is_empty());
    for id in ids {
        assert!(id >= lo && id < hi, "term {id} escaped space 3");
    }

    // And the store keeps allocating inside its new space afterwards — a
    // respace that moved the data but not the registry would allocate the next
    // term back in space 0 and collide on the next composition.
    let store = Store::open(&dst.to_string_lossy()).unwrap();
    let fresh = store.intern("urn:t:after-respace").unwrap();
    assert!(
        fresh >= lo && fresh < hi,
        "post-respace intern {fresh} escaped space 3"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 3 — Ref BLOBs
// ---------------------------------------------------------------------------

#[test]
fn ref_blobs_are_rewritten_and_still_resolve() {
    let scratch = Scratch::new("refblob");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);

    let report = respace_file(&src, &dst, 5).unwrap();
    assert_eq!(
        report.ref_blobs, 2,
        "the fixture has exactly two object-position entity references; if this \
         number changes the fixture changed, and the assertion below is no \
         longer pinning what it claims to pin"
    );

    let conn = Connection::open(&dst).unwrap();
    let lo = 5 * SPACE_SIZE;
    let blobs: Vec<Vec<u8>> = conn
        .prepare("SELECT v FROM facts WHERE substr(v, 1, 1) = X'00'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blobs.len(), 2);
    for blob in blobs {
        let Value::Ref(id) = Value::from_bytes(&blob).unwrap() else {
            panic!("tag said Ref, codec disagreed")
        };
        assert!(id >= lo && id < lo + SPACE_SIZE, "Ref {id} escaped space 5");
        // Reachability, not arithmetic: the id must name a term that is there.
        let iri: String = conn
            .query_row(
                "SELECT iri FROM terms WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .expect("a rewritten Ref must still resolve");
        assert!(iri.starts_with("urn:t:"));
    }
}

#[test]
fn non_ref_values_are_left_exactly_as_they_were() {
    // The mirror of the test above, and the one that fails if respace decides
    // every BLOB is a term id. A `Value::Int` whose payload happens to look
    // like a small term id is the case that would corrupt silently.
    let scratch = Scratch::new("nonref");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    {
        let mut store = Store::open(&src.to_string_lossy()).unwrap();
        let e = store.intern("urn:t:e").unwrap();
        let a = store.intern("urn:t:count").unwrap();
        store
            // 2 is a live term id in this store.
            .transact(&[fact(e, a, Value::Int(2))], T0, None, None)
            .unwrap();
    }
    respace_file(&src, &dst, 4).unwrap();

    let conn = Connection::open(&dst).unwrap();
    let blob: Vec<u8> = conn
        .query_row(
            "SELECT f.v FROM facts f JOIN terms t ON t.id = f.a WHERE t.iri = 'urn:t:count'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        Value::from_bytes(&blob).unwrap(),
        Value::Int(2),
        "a non-Ref value must survive a respace untouched"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 5 — enumerated from the schema
// ---------------------------------------------------------------------------

#[test]
fn a_new_column_must_be_classified() {
    // THE acceptance-5 test. It is not "respace handles the columns we know
    // about" — that is unfalsifiable, since the implementation and the
    // assertion would be reading the same list. It is: add a column to the
    // real schema, and respace must REFUSE rather than silently skip it.
    let scratch = Scratch::new("newcolumn");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);
    {
        let conn = Connection::open(&src).unwrap();
        // Exactly what a future Track A issue would do: add a term-id-bearing
        // column and forget this file exists.
        conn.execute_batch("ALTER TABLE graphs ADD COLUMN successor_graph INTEGER;")
            .unwrap();
    }

    let err = respace_file(&src, &dst, 6).unwrap_err().to_string();
    assert!(
        err.contains("graphs.successor_graph"),
        "the refusal must NAME the column: {err}"
    );
    assert!(
        err.contains("COLUMN_CLASSIFICATION"),
        "the refusal must say where to classify it: {err}"
    );
    assert!(
        !dst.exists(),
        "a refused respace must leave no artifact behind"
    );
}

#[test]
fn the_classification_covers_the_live_schema_exactly() {
    // The control for the test above: with nothing added, every column of a
    // real store is classified and nothing in the table is stale. Without this,
    // `a_new_column_must_be_classified` would still pass on a classification
    // table that had rotted into uselessness.
    let scratch = Scratch::new("cover");
    let src = scratch.path("src.db");
    seed_store(&src);
    let conn = Connection::open(&src).unwrap();

    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let mut live: Vec<(String, String)> = Vec::new();
    for t in &tables {
        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info(?1)")
            .unwrap()
            .query_map(rusqlite::params![t], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for c in cols {
            live.push((t.clone(), c));
        }
    }
    assert!(live.len() > 50, "sanity: the schema is not this small");

    for (t, c) in &live {
        assert!(
            COLUMN_CLASSIFICATION
                .iter()
                .any(|(ct, cc, _)| ct == t && cc == c),
            "{t}.{c} exists in the store and is unclassified"
        );
    }
    // And the reverse: an entry naming a column that no longer exists is a
    // stale classification, which is how a list drifts into a lie.
    for (t, c, _) in COLUMN_CLASSIFICATION {
        assert!(
            live.iter().any(|(lt, lc)| lt == t && lc == c),
            "COLUMN_CLASSIFICATION names {t}.{c}, which the schema does not have"
        );
    }
}

#[test]
fn the_vectors_entity_id_column_is_actually_rewritten() {
    // `vectors.entity_id` is the column the schema enumeration found and no
    // hand-written list had. Pinned on its own, because the round-trip test
    // reads facts and would pass with the vector table left behind — which is
    // precisely the silent-miss failure mode.
    let scratch = Scratch::new("vectors");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);
    respace_file(&src, &dst, 2).unwrap();

    let conn = Connection::open(&dst).unwrap();
    let (entity_id, text): (i64, String) = conn
        .query_row("SELECT entity_id, text FROM vectors", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(text, "Alice the engineer");
    let lo = 2 * SPACE_SIZE;
    assert!(
        entity_id >= lo && entity_id < lo + SPACE_SIZE,
        "vectors.entity_id {entity_id} was left in the old space"
    );
    // And it still names the entity it named before.
    let iri: String = conn
        .query_row(
            "SELECT iri FROM terms WHERE id = ?1",
            rusqlite::params![entity_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(iri, "urn:t:alice");
}

#[test]
fn the_graphs_registry_and_its_parent_binding_move_together() {
    // `graphs.g` and `graphs.parent_branch` are the two columns quipu #74's own
    // scope misses. `parent_branch` REFERENCES `graphs(g)`, so this also proves
    // the deferred-FK path commits a consistent registry rather than one that
    // merely passed a per-row check.
    let scratch = Scratch::new("graphs");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);
    respace_file(&src, &dst, 9).unwrap();

    let conn = Connection::open(&dst).unwrap();
    let lo = 9 * SPACE_SIZE;
    let (g, parent): (i64, i64) = conn
        .query_row(
            "SELECT g, parent_branch FROM graphs WHERE class = 'overlay'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        g >= lo && g < lo + SPACE_SIZE,
        "overlay g {g} escaped space 9"
    );
    assert!(
        parent >= lo && parent < lo + SPACE_SIZE,
        "parent_branch {parent} escaped space 9 — the bind-once binding now \
         points at a graph that does not exist"
    );
    let parent_class: String = conn
        .query_row(
            "SELECT class FROM graphs WHERE g = ?1",
            rusqlite::params![parent],
            |r| r.get(0),
        )
        .expect("parent_branch must still resolve to a real graph");
    assert_eq!(parent_class, "committed");

    // ROOT is a sentinel, not a term: it must NOT have moved.
    let root: i64 = conn
        .query_row("SELECT g FROM graphs WHERE g = 0", [], |r| r.get(0))
        .expect("ROOT must still be graph 0 in every space");
    assert_eq!(root, 0);
    assert!(
        conn.query_row("SELECT COUNT(*) FROM facts WHERE g = 0", [], |r| r
            .get::<_, i64>(0))
            .unwrap()
            > 0,
        "ROOT-graph facts must still be in ROOT"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 6 — the original is untouched
// ---------------------------------------------------------------------------

#[test]
fn original_is_byte_identical_after_respace() {
    // On a LEGACY source, deliberately — see `seed_legacy_store`. A migrated
    // source cannot distinguish "never opened for writing" from "opened
    // read-write and the migrations happened to be no-ops".
    let scratch = Scratch::new("byteident");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_legacy_store(&src);

    let before = digest(&src);
    assert!(!before.is_empty());
    // The control that makes the assertion below mean something: this source
    // is genuinely one that a read-write open WOULD rewrite.
    {
        let probe = scratch.path("probe.db");
        std::fs::copy(&src, &probe).unwrap();
        drop(Store::open(&probe.to_string_lossy()).unwrap());
        assert_ne!(
            digest(&probe),
            before,
            "the fixture is not migration-sensitive, so byte-identity below \
             would prove nothing about how the source was opened"
        );
    }

    respace_file(&src, &dst, 11).unwrap();
    let after = digest(&src);

    assert_eq!(
        before.len(),
        after.len(),
        "the source file changed length during a respace"
    );
    assert!(
        before == after,
        "the source file's bytes changed during a respace"
    );
    // No sidecars left behind either: a -wal next to the original is a write.
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = src.as_os_str().to_owned();
        sidecar.push(suffix);
        assert!(
            !Path::new(&sidecar).exists(),
            "respace left {sidecar:?} beside the original"
        );
    }

    // And the copy is a working, current-schema store — byte-identity of the
    // source is necessary, not sufficient.
    let moved = Store::open(&dst.to_string_lossy()).unwrap();
    assert_eq!(moved.local_term_space().unwrap(), 11);
    let iri: String = moved
        .conn
        .query_row(
            "SELECT t.iri FROM facts f JOIN terms t ON t.id = f.e LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(iri, "urn:l:alice");
}

#[test]
fn respace_refuses_an_existing_destination() {
    let scratch = Scratch::new("existing");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);
    std::fs::write(&dst, b"not a database").unwrap();

    let err = respace_file(&src, &dst, 3).unwrap_err().to_string();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        std::fs::read(&dst).unwrap(),
        b"not a database",
        "the refused destination must not be touched either"
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn respace_into_the_same_space_is_refused() {
    let scratch = Scratch::new("samespace");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);

    let err = respace_file(&src, &dst, 0).unwrap_err().to_string();
    assert!(err.contains("already owns space 0"), "{err}");
    assert!(
        !dst.exists(),
        "a refused respace must not leave a half-written copy"
    );
}

#[test]
fn respace_refuses_a_space_out_of_range() {
    let scratch = Scratch::new("range");
    let src = scratch.path("src.db");
    seed_store(&src);
    for bad in [-1, MAX_SPACE + 1] {
        let dst = scratch.path(&format!("dst{bad}.db"));
        let err = respace_file(&src, &dst, bad).unwrap_err().to_string();
        assert!(err.contains("out of range"), "space {bad}: {err}");
        assert!(!dst.exists());
    }
}

#[test]
fn respace_refuses_a_store_whose_ids_have_two_origins() {
    // A blanket shift is only correct if every id came from one place. A store
    // holding a foreign id has been composed some other way, and shifting it
    // would move that id to an address belonging to nobody.
    let scratch = Scratch::new("twoorigins");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);
    {
        let conn = Connection::open(&src).unwrap();
        conn.execute(
            "INSERT INTO terms (id, iri) VALUES (?1, 'urn:t:foreign')",
            rusqlite::params![4 * SPACE_SIZE + 17],
        )
        .unwrap();
    }

    let err = respace_file(&src, &dst, 8).unwrap_err().to_string();
    assert!(err.contains("more than one origin"), "{err}");
    assert!(!dst.exists());
}

#[test]
fn a_ref_blob_and_an_id_column_cannot_be_confused() {
    // Guards the classification itself rather than the rewrite: `facts.v` must
    // be RefBlob and never Id (an arithmetic shift on a BLOB is a no-op that
    // reports success), and `facts.e` must be Id and never RefBlob (decoding an
    // integer as a Value would error on every row).
    let kind = |t: &str, c: &str| {
        COLUMN_CLASSIFICATION
            .iter()
            .find(|(ct, cc, _)| *ct == t && *cc == c)
            .map(|(_, _, k)| *k)
    };
    assert_eq!(kind("facts", "v"), Some(TermIdKind::RefBlob));
    assert_eq!(kind("facts", "e"), Some(TermIdKind::Id));
    assert_eq!(kind("facts", "a"), Some(TermIdKind::Id));
    assert_eq!(kind("facts", "g"), Some(TermIdKind::Id));
    assert_eq!(kind("graphs", "g"), Some(TermIdKind::Id));
    assert_eq!(kind("graphs", "parent_branch"), Some(TermIdKind::Id));
    assert_eq!(kind("terms", "id"), Some(TermIdKind::Id));
    assert_eq!(kind("vectors", "entity_id"), Some(TermIdKind::Id));
    assert_eq!(kind("forks", "g"), Some(TermIdKind::Id));
    assert_eq!(kind("forks", "parent_branch"), Some(TermIdKind::Id));
    // The near-misses: integers that are NOT term ids.
    assert_eq!(kind("facts", "tx"), Some(TermIdKind::None));
    assert_eq!(kind("facts", "retracted_tx"), Some(TermIdKind::None));
    assert_eq!(kind("forks", "fork_tx"), Some(TermIdKind::None));
    assert_eq!(kind("graphs", "labels_tx"), Some(TermIdKind::None));
    assert_eq!(kind("term_spaces", "space"), Some(TermIdKind::None));
    assert_eq!(kind("events", "tx_id"), Some(TermIdKind::None));
}

#[test]
fn sql_and_rust_remap_agree() {
    // The id mapping exists twice — once in Rust for the `Ref` blob path, once
    // in SQL for the bulk column path — so it can drift, and it already has:
    // sabotaging `remap`'s ROOT branch failed to break a single test in this
    // file, because a `Ref` never points at the sentinel and the SQL string was
    // carrying the exemption alone. This runs both against the same ids.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (id INTEGER);").unwrap();
    let ids = [0i64, 1, 2, 41, 1_000_000, SPACE_SIZE - 1];
    for id in ids {
        conn.execute("INSERT INTO t (id) VALUES (?1)", rusqlite::params![id])
            .unwrap();
    }

    for (from_space, to_space) in [(0i64, 7i64), (5, 13), (13, 0), (0, MAX_SPACE)] {
        let (from_lo, to_lo) = (from_space * SPACE_SIZE, to_space * SPACE_SIZE);
        let sql = format!("SELECT id, {} FROM t ORDER BY id", id_remap_sql("id"));
        let pairs: Vec<(i64, i64)> = conn
            .prepare(&sql)
            .unwrap()
            .query_map(rusqlite::params![from_lo, to_lo], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(pairs.len(), ids.len());
        for (id, by_sql) in pairs {
            assert_eq!(
                by_sql,
                remap(id, from_lo, to_lo),
                "SQL and Rust disagree on id {id} moving {from_space} -> {to_space}"
            );
        }
    }
    // And the property both must have: the sentinel is fixed, everything else moves.
    assert_eq!(remap(0, 0, 7 * SPACE_SIZE), 0);
    assert_eq!(remap(1, 0, 7 * SPACE_SIZE), 7 * SPACE_SIZE + 1);
}

#[test]
fn transaction_ids_do_not_move() {
    // The mirror of the classification test, measured on data: `facts.tx` and
    // `transactions.id` are small integers that look exactly like term ids.
    // Shifting them would break the tx join with no error anywhere.
    let scratch = Scratch::new("txids");
    let src = scratch.path("src.db");
    let dst = scratch.path("dst.db");
    seed_store(&src);

    let before: Vec<i64> = {
        let conn = Connection::open(&src).unwrap();
        conn.prepare("SELECT id FROM transactions ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert!(!before.is_empty());
    respace_file(&src, &dst, 12).unwrap();

    let conn = Connection::open(&dst).unwrap();
    let after: Vec<i64> = conn
        .prepare("SELECT id FROM transactions ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(after, before, "transaction ids are not term ids");

    // And every fact still joins to a transaction that exists.
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM facts f LEFT JOIN transactions t ON t.id = f.tx \
             WHERE t.id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0);
}

#[test]
fn a_respaced_store_can_be_respaced_again() {
    // Respace is not a one-way trip from space 0: the arithmetic has to work
    // from an arbitrary origin, and `from_space` has to be read rather than
    // assumed. An implementation that hardcodes "the source is space 0" passes
    // every other test in this file.
    let scratch = Scratch::new("twice");
    let src = scratch.path("src.db");
    let mid = scratch.path("mid.db");
    let dst = scratch.path("dst.db");
    let before = seed_store(&src);

    respace_file(&src, &mid, 5).unwrap();
    let report = respace_file(&mid, &dst, 13).unwrap();
    assert_eq!(
        report.from_space, 5,
        "the source space must be READ, not assumed"
    );
    assert_eq!(report.to_space, 13);

    let moved = Store::open(&dst.to_string_lossy()).unwrap();
    assert_eq!(moved.local_term_space().unwrap(), 13);
    assert_eq!(
        observe(&moved.conn),
        before,
        "two hops must still read back as the original store"
    );
}
