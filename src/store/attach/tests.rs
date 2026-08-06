//! Tests for `ATTACH`ed read-only layers (quipu #75, and #74's acceptance 4).
//!
//! The load-bearing ones are [`attaching_changes_no_existing_query_result`] —
//! the compatibility guarantee the whole design rests on — and
//! [`two_space_zero_databases_are_refused_naming_respace`], which is quipu
//! #74's acceptance 4 landing where the composition first exists.

use std::path::{Path, PathBuf};

use super::{Attachment, validate_alias};
use crate::store::Store;
use crate::types::Value;
use crate::{Datum, Op};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "quipu-attach-{name}-{}-{:?}",
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

/// `Store` is not `Debug`, so `unwrap_err` cannot be used on an open. This also
/// reads better: it says what is expected rather than what is unwrapped.
fn expect_refused(r: crate::error::Result<Store>) -> String {
    match r {
        Ok(_) => panic!("expected the open to be REFUSED, but it succeeded"),
        Err(e) => e.to_string(),
    }
}

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

/// A store with one named graph holding one fact, plus a ROOT fact.
/// Returns the named graph's IRI.
fn seed_layer(path: &Path, tag: &str) -> String {
    let mut store = Store::open(&path.to_string_lossy()).unwrap();
    let e = store.intern(&format!("urn:{tag}:entity")).unwrap();
    let a = store.intern(&format!("urn:{tag}:name")).unwrap();
    store
        .transact(
            &[fact(e, a, Value::Str(format!("{tag} root")))],
            T0,
            None,
            None,
        )
        .unwrap();

    let giri = format!("urn:{tag}:graph");
    let g = store.intern(&giri).unwrap();
    store
        .conn
        .execute(
            "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
             VALUES (?1, 'committed', NULL, ?2)",
            rusqlite::params![g, T0],
        )
        .unwrap();
    store
        .transact_to_graph(
            &[fact(e, a, Value::Str(format!("{tag} named")))],
            T0,
            None,
            None,
            g,
        )
        .unwrap();
    drop(store);
    giri
}

/// Move a store into a term space so it can be composed with a space-0 store.
fn respaced_layer(scratch: &Scratch, name: &str, tag: &str, space: i64) -> (PathBuf, String) {
    let src = scratch.path(&format!("{name}-src.db"));
    let dst = scratch.path(&format!("{name}.db"));
    let iri = seed_layer(&src, tag);
    crate::store::respace::respace_file(&src, &dst, space).unwrap();
    (dst, iri)
}

// ---------------------------------------------------------------------------
// Alias validation
// ---------------------------------------------------------------------------

#[test]
fn alias_validation_accepts_names_and_refuses_injection() {
    for good in ["shared", "s", "layer_2", "a0"] {
        assert!(validate_alias(good).is_ok(), "{good} should be valid");
    }
    // A schema alias is interpolated because SQLite cannot bind a parameter to
    // a schema name, so this validation is the only thing between the caller
    // and the SQL.
    for bad in [
        "",
        "Shared",
        "2layer",
        "a-b",
        "a b",
        "a;DROP TABLE facts",
        "main",
        "temp",
        "a'b",
    ] {
        assert!(validate_alias(bad).is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn a_duplicate_alias_is_refused() {
    let scratch = Scratch::new("dupalias");
    let (layer, _) = respaced_layer(&scratch, "l1", "l1", 3);
    let main = scratch.path("main.db");
    seed_layer(&main, "m");

    let l = layer.to_string_lossy().to_string();
    let err = expect_refused(Store::open_with_attachments(
        &main.to_string_lossy(),
        &[
            Attachment::read_only("shared", &l),
            Attachment::read_only("shared", &l),
        ],
    ));
    assert!(err.contains("used twice"), "{err}");
}

// ---------------------------------------------------------------------------
// quipu #74 acceptance 4 — the space-collision refusal
// ---------------------------------------------------------------------------

#[test]
fn two_space_zero_databases_are_refused_naming_respace() {
    // #74's acceptance 4, enforced where a composition first exists. Both
    // stores are space 0 — the default — so this is the case a user hits by
    // doing the obvious thing with two ordinary databases.
    let scratch = Scratch::new("twozero");
    let main = scratch.path("main.db");
    let other = scratch.path("other.db");
    seed_layer(&main, "m");
    seed_layer(&other, "o");

    let err = expect_refused(Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &other.to_string_lossy())],
    ));

    assert!(err.contains("term space 0"), "must name the space: {err}");
    assert!(
        err.contains("respace"),
        "acceptance 4 requires the message to NAME `respace`: {err}"
    );
    assert!(
        err.contains("quipu db respace --into"),
        "a remedy the reader cannot run is not a remedy: {err}"
    );
}

#[test]
fn two_attachments_in_the_same_nonzero_space_are_refused() {
    // The collision rule is not "space 0 is special" — any two databases
    // sharing a space are uncomposable. An implementation that only special-
    // cased space 0 passes the test above and fails here.
    let scratch = Scratch::new("samenonzero");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (a, _) = respaced_layer(&scratch, "a", "a", 4);
    let (b, _) = respaced_layer(&scratch, "b", "b", 4);

    let err = expect_refused(Store::open_with_attachments(
        &main.to_string_lossy(),
        &[
            Attachment::read_only("first", &a.to_string_lossy()),
            Attachment::read_only("second", &b.to_string_lossy()),
        ],
    ));
    assert!(err.contains("term space 4"), "{err}");
    assert!(
        err.contains("attachment first"),
        "must name the incumbent: {err}"
    );
}

#[test]
fn distinct_spaces_attach_cleanly() {
    // The control. Without it the two refusals above would pass on an
    // implementation that refuses everything.
    let scratch = Scratch::new("distinct");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (a, _) = respaced_layer(&scratch, "a", "a", 4);
    let (b, _) = respaced_layer(&scratch, "b", "b", 5);

    let store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[
            Attachment::read_only("first", &a.to_string_lossy()),
            Attachment::read_only("second", &b.to_string_lossy()),
        ],
    )
    .expect("three distinct spaces compose");
    assert_eq!(store.attachments().len(), 2);
}

#[test]
fn a_layer_without_named_graphs_is_refused() {
    // A pre-quad database has no `g`, so a composed query has no graph to
    // select on. Built by hand because no current quipu can produce one.
    let scratch = Scratch::new("nog");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let old = scratch.path("old.db");
    {
        let conn = rusqlite::Connection::open(&old).unwrap();
        conn.execute_batch(
            "CREATE TABLE terms (id INTEGER PRIMARY KEY, iri TEXT NOT NULL UNIQUE);
             CREATE TABLE facts (e INTEGER, a INTEGER, v BLOB, tx INTEGER, valid_from TEXT);
             CREATE TABLE term_spaces (space INTEGER PRIMARY KEY, db TEXT, local INTEGER);
             INSERT INTO term_spaces VALUES (6, 'main', 1);",
        )
        .unwrap();
    }
    let err = expect_refused(Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("old", &old.to_string_lossy())],
    ));
    assert!(err.contains("no `g` column"), "{err}");
    assert!(
        err.contains("migrate"),
        "the refusal must name the fix: {err}"
    );
}

// ---------------------------------------------------------------------------
// The compatibility guarantee
// ---------------------------------------------------------------------------

#[test]
fn attaching_changes_no_existing_query_result() {
    // THE guarantee of §3: attaching adds named graphs and never widens the
    // default dataset. Measured as a comparison against the same store opened
    // without the attachment, not against an expectation of what it should say.
    let scratch = Scratch::new("nowiden");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (layer, _) = respaced_layer(&scratch, "shared", "s", 8);

    let read_all = |store: &Store| -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = store
            .conn
            .prepare(
                "SELECT t.iri, f.v FROM facts f JOIN terms t ON t.id = f.e \
                 WHERE f.g = 0 AND f.op = 1 AND f.valid_to IS NULL",
            )
            .unwrap()
            .query_map([], |r| {
                let iri: String = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                Ok((iri, format!("{:?}", Value::from_bytes(&blob).unwrap())))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.sort();
        rows
    };

    let before = {
        let s = Store::open(&main.to_string_lossy()).unwrap();
        read_all(&s)
    };
    assert!(!before.is_empty(), "the fixture must hold ROOT facts");

    let with = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &layer.to_string_lossy())],
    )
    .unwrap();
    assert_eq!(
        read_all(&with),
        before,
        "attaching a database changed what the default dataset returns"
    );
}

#[test]
fn attached_named_graphs_register_locally_and_root_does_not() {
    let scratch = Scratch::new("register");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (layer, layer_iri) = respaced_layer(&scratch, "shared", "s", 8);

    let store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &layer.to_string_lossy())],
    )
    .unwrap();

    // The attached graph is in the ONE registry, attributed to its alias.
    let g = store.lookup(&layer_iri).unwrap();
    // Its IRI is interned in the ATTACHED store, so the local term table need
    // not know it; the registry row is keyed by the id either way.
    let sources: Vec<(i64, Option<String>)> = store
        .conn
        .prepare("SELECT g, source FROM main.graphs ORDER BY g")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let attached: Vec<_> = sources
        .iter()
        .filter(|(_, s)| s.as_deref() == Some("shared"))
        .collect();
    assert_eq!(
        attached.len(),
        1,
        "exactly one attached named graph: {sources:?}"
    );
    if let Some(gid) = g {
        assert_eq!(attached[0].0, gid);
    }

    // ROOT is NOT copied: every database has a g=0 row meaning its OWN default
    // graph, and copying it would let an attachment claim the local ROOT.
    let root_source: Option<String> = store
        .conn
        .query_row("SELECT source FROM main.graphs WHERE g = 0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(root_source, None, "ROOT must stay local");

    // Nor is the attachment's META-graph copied. It is reserved per-database
    // exactly as ROOT is, and a second row claiming to be the meta-graph
    // resolves to nothing locally. This is what caught the omission: the test
    // expected one attached graph and found two.
    let meta_local = store.meta_graph_id().unwrap();
    let attached_meta: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM main.graphs WHERE source IS NOT NULL AND g <> ?1",
            rusqlite::params![meta_local],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        attached_meta, 1,
        "only the NAMED graph should be contributed"
    );
}

#[test]
fn reopening_with_the_same_attachment_converges() {
    // Registration is idempotent: re-opening must not accumulate rows or fail
    // on the primary key.
    let scratch = Scratch::new("reopen");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (layer, _) = respaced_layer(&scratch, "shared", "s", 8);
    let att = [Attachment::read_only("shared", &layer.to_string_lossy())];

    let count = |s: &Store| -> i64 {
        s.conn
            .query_row(
                "SELECT COUNT(*) FROM main.graphs WHERE source IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };

    let first = {
        let s = Store::open_with_attachments(&main.to_string_lossy(), &att).unwrap();
        count(&s)
    };
    let second = {
        let s = Store::open_with_attachments(&main.to_string_lossy(), &att).unwrap();
        count(&s)
    };
    assert_eq!(first, 1);
    assert_eq!(second, first, "re-opening accumulated registry rows");
}

// ---------------------------------------------------------------------------
// Writes can never reach an attachment
// ---------------------------------------------------------------------------

#[test]
fn a_write_to_an_attached_graph_is_refused() {
    // #75 acceptance 4 — and the guard is LOAD-BEARING, not a third redundant
    // layer. Measured with the guard removed: the write SUCCEEDS, lands one row
    // in `main.facts` tagged with the attached graph's id, and leaves the
    // attached file untouched (2 rows, unchanged).
    //
    // So neither structural mechanism covers this case. `mode=ro` protects the
    // attached FILE; unqualified table names route the write to `main` — and
    // routing to main is exactly what produces the defect. The result is a
    // LOCAL fact silently claiming membership of an attached graph, which every
    // composed query then reads as if the layer had supplied it. Nothing errors.
    let scratch = Scratch::new("nowrite");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (layer, layer_iri) = respaced_layer(&scratch, "shared", "s", 8);

    let mut store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &layer.to_string_lossy())],
    )
    .unwrap();

    let g: i64 = store
        .conn
        .query_row(
            "SELECT g FROM main.graphs WHERE source = 'shared'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let _ = layer_iri;

    let e = store.intern("urn:m:new").unwrap();
    let a = store.intern("urn:m:name").unwrap();
    let err = store
        .transact_to_graph(&[fact(e, a, Value::Str("nope".into()))], T0, None, None, g)
        .unwrap_err()
        .to_string();
    assert!(err.contains("attached database"), "{err}");
    assert!(err.contains("shared"), "must name the attachment: {err}");

    // And the attachment's file is genuinely unchanged.
    let facts_in_layer: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM shared.facts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(facts_in_layer, 2, "the attached file must be untouched");
}

#[test]
fn a_local_write_still_works_with_an_attachment_mounted() {
    // The control for the refusal above: it must refuse attached graphs, not
    // writes in general. A guard that refuses everything passes that test.
    let scratch = Scratch::new("localwrite");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (layer, _) = respaced_layer(&scratch, "shared", "s", 8);

    let mut store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &layer.to_string_lossy())],
    )
    .unwrap();
    let e = store.intern("urn:m:fresh").unwrap();
    let a = store.intern("urn:m:name").unwrap();
    // ROOT...
    store
        .transact(
            &[fact(e, a, Value::Str("local root".into()))],
            T0,
            None,
            None,
        )
        .expect("a local ROOT write must still succeed");
    // ...and a LOCAL named graph, which is the case the guard must not catch.
    let lg = store.intern("urn:m:localgraph").unwrap();
    store
        .conn
        .execute(
            "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
             VALUES (?1, 'committed', NULL, ?2)",
            rusqlite::params![lg, T0],
        )
        .unwrap();
    store
        .transact_to_graph(
            &[fact(e, a, Value::Str("local named".into()))],
            T0,
            None,
            None,
            lg,
        )
        .expect("a write to a LOCAL named graph must still succeed");
}

#[test]
fn the_attached_file_is_mounted_read_only_by_sqlite() {
    // Not our care but SQLite's: `mode=ro` is the mechanism. Asserted directly
    // so that if the URI form is ever dropped, this fails rather than leaving
    // the Rust guard as the only thing standing.
    let scratch = Scratch::new("modero");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");
    let (layer, _) = respaced_layer(&scratch, "shared", "s", 8);

    let store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &layer.to_string_lossy())],
    )
    .unwrap();
    let err = store
        .conn
        .execute("INSERT INTO shared.terms (iri) VALUES ('urn:injected')", [])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("readonly"),
        "SQLite must refuse the write itself: {err}"
    );
}

// ---------------------------------------------------------------------------
// The no-attachment path is untouched
// ---------------------------------------------------------------------------

#[test]
fn a_store_with_no_attachments_is_indistinguishable() {
    // The default path must stay exactly what it was. `open` now routes through
    // `init_with_attachments`, so this pins that the empty case adds nothing:
    // no attachments recorded, no graph attributed to a source, and the write
    // guard short-circuits rather than querying.
    let scratch = Scratch::new("noattach");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");

    let store = Store::open(&main.to_string_lossy()).unwrap();
    assert!(store.attachments().is_empty());
    let sourced: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM main.graphs WHERE source IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sourced, 0);
    // Every graph is writable when nothing is attached.
    for g in [0i64, 1, 99] {
        store.assert_graph_is_writable(g).unwrap();
    }
}

#[test]
fn graph_labels_travel_with_an_attachment_but_labels_tx_does_not() {
    // A deliberate decision, pinned because it is a decision: a layer is
    // attached BECAUSE of its trust rank, so dropping labels would make it
    // arrive silently untrusted — §3's "uniform labels". But `labels_tx` is a
    // transaction id in the ATTACHED store's sequence and means nothing here,
    // so it is left NULL rather than copied as a foreign integer that looks
    // local. That is the same looks-valid-and-is-not failure quipu #74 spent
    // its acceptance on.
    let scratch = Scratch::new("labels");
    let main = scratch.path("main.db");
    seed_layer(&main, "m");

    let src = scratch.path("lbl-src.db");
    let layer = scratch.path("lbl.db");
    let giri = seed_layer(&src, "l");
    {
        let store = Store::open(&src.to_string_lossy()).unwrap();
        let g = store.lookup(&giri).unwrap().unwrap();
        store
            .conn
            .execute(
                "UPDATE graphs SET trust_rank = 40, fresh_rank = 7, \
                 trust_chain = 'urn:l:chain', policy = 'strict', labels_tx = 99 \
                 WHERE g = ?1",
                rusqlite::params![g],
            )
            .unwrap();
    }
    crate::store::respace::respace_file(&src, &layer, 6).unwrap();

    let store = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[Attachment::read_only("shared", &layer.to_string_lossy())],
    )
    .unwrap();

    struct Labels {
        trust: Option<i64>,
        fresh: Option<i64>,
        chain: Option<String>,
        policy: Option<String>,
        labels_tx: Option<i64>,
    }
    let Labels {
        trust,
        fresh,
        chain,
        policy,
        labels_tx: ltx,
    } = store
        .conn
        .query_row(
            "SELECT trust_rank, fresh_rank, trust_chain, policy, labels_tx \
             FROM main.graphs WHERE source = 'shared'",
            [],
            |r| {
                Ok(Labels {
                    trust: r.get(0)?,
                    fresh: r.get(1)?,
                    chain: r.get(2)?,
                    policy: r.get(3)?,
                    labels_tx: r.get(4)?,
                })
            },
        )
        .unwrap();

    assert_eq!(trust, Some(40), "trust rank must travel with the layer");
    assert_eq!(fresh, Some(7));
    assert_eq!(chain.as_deref(), Some("urn:l:chain"));
    assert_eq!(policy.as_deref(), Some("strict"));
    assert_eq!(
        ltx, None,
        "labels_tx is a FOREIGN transaction id — it must not be copied into \
         the local registry, where it would read as a local tx"
    );
}
