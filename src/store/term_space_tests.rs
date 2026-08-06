//! Tests for term spaces (quipu #74).
//!
//! The load-bearing one is [`legacy_store_keeps_byte_identical_space_zero_ids`].
//! It deliberately does NOT use a fresh store: a fresh store has the reserved
//! meta-graph at id 1 and its first user term at 2, so every id it allocates is
//! consistent with a per-space counter that happens to start in the right
//! place. The case that actually distinguishes the two implementations is a
//! store that already holds terms at positions this code did not choose —
//! which is every store in production and none of the fixtures.

use rusqlite::Connection;

use super::{Store, intern_in_space, local_term_space};
use crate::schema::{INIT_SQL, SPACE_SIZE};

/// A store shaped like one created before the term-space registry existed:
/// `INIT_SQL` only, terms interned straight onto the rowid, and no migration
/// run yet. Returns the raw connection so the caller controls when migrations
/// happen — which is the whole point of the test below.
fn legacy_connection(term_count: usize) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(INIT_SQL).unwrap();
    for i in 0..term_count {
        conn.execute(
            "INSERT INTO terms (iri) VALUES (?1)",
            rusqlite::params![format!("urn:legacy:term-{i}")],
        )
        .unwrap();
    }
    conn
}

#[test]
fn legacy_store_keeps_byte_identical_space_zero_ids() {
    // quipu #74 acceptance 1, under sattler's binding condition: a SEEDED
    // store, with the meta-graph at a non-1 id.
    let conn = legacy_connection(5);

    // Record the pre-migration truth, so "unchanged" is a comparison and not a
    // belief about what the ids ought to be.
    let before: Vec<(i64, String)> = conn
        .prepare("SELECT id, iri FROM terms ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before.len(), 5);
    assert_eq!(before[0].0, 1, "a legacy store's ids start at 1");

    let store = Store::init(conn).expect("migrations run on a legacy store");

    // 1. It is space 0 by definition, from one seeded row and no rewrite.
    assert_eq!(store.local_term_space().unwrap(), 0);

    // 2. NOT ONE existing id moved.
    let after: Vec<(i64, String)> = store
        .prepare("SELECT id, iri FROM terms WHERE iri LIKE 'urn:legacy:%' ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(after, before, "a legacy store is respaced by nobody");

    // 3. The meta-graph landed AFTER the pre-existing terms — the condition a
    //    fresh-store test cannot express, because there it is always 1.
    let meta = store.meta_graph_id().unwrap();
    assert_eq!(meta, 6, "meta-graph takes the next rowid, not a fixed slot");
    assert_ne!(meta, 1, "the fresh-store position would hide the bug");

    // 4. Allocation CONTINUES the rowid sequence. A per-space counter for
    //    space 0 would compute k = 1 here and collide with a live primary key.
    assert_eq!(store.intern("urn:new:first").unwrap(), 7);
    assert_eq!(store.intern("urn:new:second").unwrap(), 8);

    // 5. Interning is still idempotent, in both directions.
    assert_eq!(store.intern("urn:new:first").unwrap(), 7);
    assert_eq!(store.intern("urn:legacy:term-0").unwrap(), 1);
}

#[test]
fn allocation_never_returns_an_id_already_bound() {
    // THE guard for space-0 safety — and it exists because the obvious guard
    // does not work. "Space 0 must delegate to the rowid" cannot be tested:
    // this allocator computes `MAX(id) + 1` within the space, and for space 0
    // that IS the rowid, so removing the delegation changes no observable
    // behaviour. Measured by sabotage: with the space-0 branch disabled, all of
    // the other tests in this file still passed.
    //
    // What the constraint is really protecting against is an allocator that
    // invents `k` from an independent counter instead of deriving it from the
    // table. On a store whose ids were assigned by someone else — every real
    // one — that hands back an id that is already taken. THAT is falsifiable,
    // so that is what is asserted here.
    let store = Store::init(legacy_connection(5)).unwrap();

    let mut seen: Vec<i64> = store
        .prepare("SELECT id FROM terms")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let pre_existing = seen.clone();

    for i in 0..25 {
        let id = store.intern(&format!("urn:fresh:{i}")).unwrap();
        assert!(
            !seen.contains(&id),
            "intern handed back id {id}, which is already bound"
        );
        assert!(
            !pre_existing.contains(&id),
            "intern collided with a pre-existing id {id} — the naive-counter bug"
        );
        seen.push(id);
    }
}

#[test]
fn a_fresh_store_is_space_zero_and_allocates_exactly_as_before() {
    // The control for the test above: the fixture shape must ALSO be unchanged,
    // or #74 would break every golden file even while the legacy path is safe.
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.local_term_space().unwrap(), 0);
    assert_eq!(store.meta_graph_id().unwrap(), 1);
    assert_eq!(store.intern("urn:a").unwrap(), 2);
    assert_eq!(store.intern("urn:b").unwrap(), 3);
}

#[test]
fn a_non_zero_space_allocates_inside_its_own_range() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(INIT_SQL).unwrap();
    conn.execute(
        "INSERT INTO term_spaces (space, db, local) VALUES (7, 'main', 1)",
        [],
    )
    .unwrap();

    let lo = 7 * SPACE_SIZE;
    // The first term of an EMPTY non-zero space: the rowid would be 1, which is
    // outside the space entirely. This is why allocation cannot just delegate.
    let first = intern_in_space(&conn, "urn:s7:first").unwrap();
    assert_eq!(first, lo + 1);
    let second = intern_in_space(&conn, "urn:s7:second").unwrap();
    assert_eq!(second, lo + 2);

    assert!(
        first >= lo && first < lo + SPACE_SIZE,
        "allocation must land inside the owning space"
    );
    // And idempotence still holds across the explicit-id path.
    assert_eq!(intern_in_space(&conn, "urn:s7:first").unwrap(), lo + 1);
}

#[test]
fn the_meta_graph_of_a_non_zero_space_store_is_in_that_space() {
    // The latent bug this ordering exists to prevent: `migrate_graph_labels`
    // interns the meta-graph, and if that intern is not space-aware the
    // reserved graph is allocated OUTSIDE the space that owns it — on a store
    // whose every other id is in-space.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(INIT_SQL).unwrap();
    conn.execute(
        "INSERT INTO term_spaces (space, db, local) VALUES (3, 'main', 1)",
        [],
    )
    .unwrap();

    let store = Store::init(conn).unwrap();
    let meta = store.meta_graph_id().unwrap();
    let lo = 3 * SPACE_SIZE;
    assert!(
        meta >= lo && meta < lo + SPACE_SIZE,
        "meta-graph {meta} escaped space 3 [{lo}, {})",
        lo + SPACE_SIZE
    );
}

#[test]
fn reopening_a_respaced_store_does_not_re_seed_space_zero() {
    // Why the seed is a guarded migration rather than an `INSERT OR IGNORE` in
    // INIT_SQL: an unconditional seed of (0, …) would silently give a respaced
    // store a SECOND row claiming to be local, and then `local_term_space`
    // answers whichever one SQLite hands back first.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(INIT_SQL).unwrap();
    conn.execute(
        "INSERT INTO term_spaces (space, db, local) VALUES (9, 'main', 1)",
        [],
    )
    .unwrap();

    Store::migrate_term_spaces(&conn).unwrap();
    Store::migrate_term_spaces(&conn).unwrap(); // idempotent under repetition

    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM term_spaces WHERE local = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "exactly one local space, still");
    assert_eq!(local_term_space(&conn).unwrap(), 9, "and it is still 9");
}

#[test]
fn a_store_with_no_registry_row_reads_as_space_zero() {
    // Absence must mean "legacy, space 0", not an error and not a fabricated
    // space — the same rule the label work follows for undeclared axes.
    let conn = legacy_connection(2);
    assert_eq!(local_term_space(&conn).unwrap(), 0);
    // ...and interning against it stays on the rowid path.
    assert_eq!(intern_in_space(&conn, "urn:x").unwrap(), 3);
}

#[test]
fn two_local_spaces_are_refused_by_the_schema() {
    // The partial unique index is what makes `local_term_space` a question with
    // one answer. Without it the ambiguity above is representable.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(INIT_SQL).unwrap();
    conn.execute(
        "INSERT INTO term_spaces (space, db, local) VALUES (1, 'main', 1)",
        [],
    )
    .unwrap();
    let second = conn.execute(
        "INSERT INTO term_spaces (space, db, local) VALUES (2, 'other', 1)",
        [],
    );
    assert!(second.is_err(), "a second local space must be refused");
}
