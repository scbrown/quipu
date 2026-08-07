//! Tests for term aliases across term spaces (quipu #76).
//!
//! Its own test file, as the issue asks, because this is where the subtle bug
//! lives: everything here is about two ids that mean one thing, or two things
//! that look like one id.
//!
//! The load-bearing one is [`the_adversarial_fixture`] — #76's acceptance 1 —
//! which is deliberately built to punish BOTH failure directions at once:
//! failing to join what is the same, and joining what merely looks the same.

use std::path::PathBuf;

use crate::store::Store;
use crate::store::attach::Attachment;
use crate::types::Value;
use crate::{Datum, Op};

const T0: &str = "2026-01-01T00:00:00Z";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "quipu-alias-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, n: &str) -> PathBuf {
        self.0.join(n)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fact(e: i64, a: i64, v: Value) -> Datum {
    Datum {
        entity: e,
        attribute: a,
        value: v,
        valid_from: T0.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

/// THE adversarial fixture of #76's acceptance 1, built so that the two
/// failure directions cannot both be satisfied by accident.
///
/// Returns `(store, shared_iri, local_only_iri, layer_only_iri)`.
///
/// - **The same IRI at different rowids.** `urn:shared:thing` is interned in
///   BOTH files. Term spaces guarantee the ids differ. A composition that
///   fails to alias them silently returns fewer rows.
/// - **Different IRIs at the same rowid.** Both files are seeded so that some
///   *within-space* rowid `k` denotes a DIFFERENT IRI on each side. Before
///   respace both stores number from 1, so this is the state a user reaches by
///   doing the obvious thing. A composition that joins on raw rowid — or that
///   canonicalises by anything other than the IRI — silently returns rows that
///   are WRONG rather than missing, which is the worse direction.
fn adversarial(scratch: &Scratch) -> (Store, String, String, String) {
    let shared = "urn:shared:thing".to_string();
    let local_only = "urn:local:only".to_string();
    let layer_only = "urn:layer:only".to_string();

    let main = scratch.path("main.db");
    {
        let mut s = Store::open(&main.to_string_lossy()).unwrap();
        // Intern local-only FIRST so the shared IRI lands on a different rowid
        // than it will in the layer — that is the "same IRI, different rowid"
        // half, made deliberate rather than hoped for.
        let lo = s.intern(&local_only).unwrap();
        let sh = s.intern(&shared).unwrap();
        let attr = s.intern("urn:p:local").unwrap();
        s.transact(
            &[
                fact(sh, attr, Value::Str("local says".into())),
                fact(lo, attr, Value::Str("local only".into())),
            ],
            T0,
            None,
            None,
        )
        .unwrap();
    }

    let src = scratch.path("layer-src.db");
    {
        let mut s = Store::open(&src.to_string_lossy()).unwrap();
        // Reverse order: the layer interns the shared IRI at the rowid the
        // local store gave to `urn:local:only`, and vice versa. So equal
        // within-space rowids denote DIFFERENT IRIs across the two files.
        let sh = s.intern(&shared).unwrap();
        let lo = s.intern(&layer_only).unwrap();
        let attr = s.intern("urn:p:layer").unwrap();
        let giri = "urn:layer:graph";
        let g = s.intern(giri).unwrap();
        s.conn
            .execute(
                "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
                 VALUES (?1, 'committed', NULL, ?2)",
                rusqlite::params![g, T0],
            )
            .unwrap();
        s.transact_to_graph(
            &[
                fact(sh, attr, Value::Str("layer says".into())),
                fact(lo, attr, Value::Str("layer only".into())),
                fact(lo, attr, Value::Ref(sh)),
            ],
            T0,
            None,
            None,
            g,
        )
        .unwrap();
    }
    let layer = scratch.path("layer.db");
    crate::store::respace::respace_file(&src, &layer, 31).unwrap();

    let store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("layer", &layer.to_string_lossy())],
    )
    .unwrap();
    (store, shared, local_only, layer_only)
}

#[test]
fn the_adversarial_fixture() {
    // #76 acceptance 1. This asserts the fixture is ACTUALLY adversarial
    // before anything relies on it — a fixture that fails to set up both
    // hazards makes every test built on it vacuous, which is the failure this
    // whole workstream keeps finding in itself.
    let scratch = Scratch::new("fixture");
    let (store, shared, local_only, layer_only) = adversarial(&scratch);

    // (a) the SAME IRI has two DIFFERENT ids
    let local_id = store.lookup(&shared).unwrap().expect("local interns it");
    let layer_id: i64 = store
        .conn
        .query_row(
            "SELECT id FROM layer.terms WHERE iri = ?1",
            rusqlite::params![shared],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(
        local_id, layer_id,
        "fixture is not adversarial: the shared IRI must have DIFFERENT ids"
    );

    // (b) equal WITHIN-SPACE rowids denote DIFFERENT IRIs across the files.
    // This is what makes a raw-id join wrong rather than merely incomplete.
    let space_size = crate::schema::SPACE_SIZE;
    let k_local = local_id % space_size;
    let layer_at_same_k: Option<String> = store
        .conn
        .query_row(
            "SELECT iri FROM layer.terms WHERE id % ?1 = ?2",
            rusqlite::params![space_size, k_local],
            |r| r.get(0),
        )
        .ok();
    let at_same_k = layer_at_same_k.expect(
        "fixture is not adversarial: the layer holds NO term at the same \
         within-space rowid as the local shared IRI, so the raw-id-collision \
         hazard is not represented at all",
    );
    assert_ne!(
        at_same_k, shared,
        "fixture is not adversarial: the layer holds the SAME IRI at that \
         rowid, so a raw-id join would accidentally be correct"
    );
    assert_eq!(
        at_same_k, layer_only,
        "the layer's term at the local shared IRI's rowid should be the \
         layer-only IRI (got {at_same_k})"
    );

    // (c) the local-only IRI is genuinely local-only
    assert!(store.lookup(&local_only).unwrap().is_some());
    let in_layer: Option<i64> = store
        .conn
        .query_row(
            "SELECT id FROM layer.terms WHERE iri = ?1",
            rusqlite::params![local_only],
            |r| r.get(0),
        )
        .ok();
    assert!(
        in_layer.is_none(),
        "fixture: urn:local:only must not exist in the layer"
    );
}

#[test]
fn lookup_all_returns_every_id_for_a_shared_iri_canonical_first() {
    let scratch = Scratch::new("lookupall");
    let (store, shared, local_only, layer_only) = adversarial(&scratch);

    let ids = store.lookup_all(&shared).unwrap();
    assert_eq!(ids.len(), 2, "a shared IRI has one id per file: {ids:?}");
    assert_eq!(
        ids[0],
        store.lookup(&shared).unwrap().unwrap(),
        "the CANONICAL (local) id must come first — callers take .first()"
    );

    // one-sided IRIs yield exactly one id each
    assert_eq!(store.lookup_all(&local_only).unwrap().len(), 1);
    assert_eq!(
        store.lookup_all(&layer_only).unwrap().len(),
        1,
        "an IRI only the layer knows is still resolvable to its one id"
    );
    // and an unknown IRI yields none, rather than something that matches
    assert!(store.lookup_all("urn:nope:nothing").unwrap().is_empty());
}

#[test]
fn canonical_id_maps_towards_main_and_is_identity_otherwise() {
    let scratch = Scratch::new("canon");
    let (store, shared, local_only, layer_only) = adversarial(&scratch);

    let ids = store.lookup_all(&shared).unwrap();
    let (canon, alias) = (ids[0], ids[1]);
    assert_eq!(
        store.canonical_id(alias).unwrap(),
        canon,
        "alias -> canonical"
    );
    assert_eq!(
        store.canonical_id(canon).unwrap(),
        canon,
        "canonical -> itself"
    );

    // An id with no alias is returned unchanged, in BOTH directions — this is
    // what keeps the layer's own vocabulary addressable instead of being
    // rewritten to something local that does not exist.
    let lo = store.lookup(&local_only).unwrap().unwrap();
    assert_eq!(store.canonical_id(lo).unwrap(), lo);
    let layer_only_id = store.lookup_all(&layer_only).unwrap()[0];
    assert_eq!(store.canonical_id(layer_only_id).unwrap(), layer_only_id);
}

fn select_rows(store: &Store, query: &str) -> Vec<crate::sparql::Bindings> {
    let crate::sparql::QueryResult::Select { rows, .. } =
        crate::sparql::query(store, query).unwrap()
    else {
        panic!("expected SELECT rows");
    };
    rows
}

#[test]
fn cross_layer_join_uses_iri_aliases_not_raw_ids() {
    let scratch = Scratch::new("join");
    let (store, _, _, _) = adversarial(&scratch);

    let rows = select_rows(
        &store,
        "SELECT ?s WHERE { \
             ?s <urn:p:local> \"local says\" . \
             GRAPH <urn:layer:graph> { ?s <urn:p:layer> \"layer says\" } \
         }",
    );
    assert_eq!(rows.len(), 1, "the shared IRI must join across term spaces");

    // The fixture deliberately gives the local-only IRI the same within-space
    // rowid as the layer's shared IRI. Raw-id equality would manufacture this
    // row; IRI-based aliases must not.
    let wrong = select_rows(
        &store,
        "SELECT ?s WHERE { \
             ?s <urn:p:local> \"local only\" . \
             GRAPH <urn:layer:graph> { ?s <urn:p:layer> \"layer says\" } \
         }",
    );
    assert!(
        wrong.is_empty(),
        "equal rowids for different IRIs must not join"
    );
}

#[test]
fn aliases_collapse_to_one_distinct_solution() {
    let scratch = Scratch::new("dedup");
    let (store, _, _, _) = adversarial(&scratch);
    let rows = select_rows(
        &store,
        "SELECT DISTINCT ?s WHERE { \
             { ?s <urn:p:local> \"local says\" } \
             UNION \
             { GRAPH <urn:layer:graph> { ?s <urn:p:layer> \"layer says\" } } \
         }",
    );
    assert_eq!(
        rows.len(),
        1,
        "two raw ids denoting one IRI must become one semantic solution"
    );
}

#[test]
fn attached_only_graph_is_nameable_by_iri() {
    let scratch = Scratch::new("named");
    let (store, _, _, _) = adversarial(&scratch);
    let rows = select_rows(
        &store,
        "SELECT ?s WHERE { \
             GRAPH <urn:layer:graph> { ?s <urn:p:layer> \"layer says\" } \
         }",
    );
    assert_eq!(rows.len(), 1);
}

#[test]
fn constants_resolve_across_subject_predicate_and_ref_object_positions() {
    let scratch = Scratch::new("constants");
    let (store, _, _, _) = adversarial(&scratch);
    let rows = select_rows(
        &store,
        "SELECT ?s WHERE { \
             GRAPH <urn:layer:graph> { \
                 <urn:layer:only> <urn:p:layer> <urn:shared:thing> \
             } \
         }",
    );
    assert_eq!(rows.len(), 1, "all three Ref-bearing positions must widen");
}

#[test]
fn an_unattached_store_is_indistinguishable() {
    // Half of #76 acceptance 4 — the half this increment can actually assert.
    //
    // Acceptance 4 is "single-space stores: `IN (x)` degenerates to today's
    // plan (no regression)". The PLAN half needs the query path, which is
    // increment 2; what is checkable now is that both new lookups are exact
    // identities on an unattached store, so there is nothing for the query
    // path to widen. Deliberately NOT claiming acceptance 4 is discharged.
    let scratch = Scratch::new("unattached");
    let main = scratch.path("main.db");
    let s = Store::open(&main.to_string_lossy()).unwrap();
    let id = s.intern("urn:a:b").unwrap();

    assert_eq!(s.lookup_all("urn:a:b").unwrap(), vec![id]);
    assert_eq!(s.canonical_id(id).unwrap(), id);
}
