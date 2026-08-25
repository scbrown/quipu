//! Deep-freeze tests: the round-trip that proves no fact is lost, the write
//! guard, and the three cold-composition opt-ins.

use super::*;
use crate::store::Datum;
use crate::types::{Op, Value};

const TS: &str = "2026-08-24T00:00:00Z";
const TS2: &str = "2026-08-24T00:00:01Z";
const WINDOW: &str = "urn:test:shuttle/runs/2026-07";

fn file_store(dir: &tempfile::TempDir) -> Store {
    let path = dir.path().join("main.db").to_string_lossy().to_string();
    Store::open(&path).unwrap()
}

/// A window graph with history: two asserts, one of which is later retracted.
fn seed_window(store: &mut Store) -> i64 {
    let g = store.graph_create(WINDOW).unwrap();
    let e = store.intern("urn:test:run-1").unwrap();
    let a_state = store.intern("urn:test:state").unwrap();
    let a_of = store.intern("urn:test:runOf").unwrap();
    let wf = store.intern("urn:test:wf:triage").unwrap();
    store
        .transact_to_graph(
            &[
                Datum {
                    entity: e,
                    attribute: a_state,
                    value: Value::Str("open".into()),
                    valid_from: TS.into(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: e,
                    attribute: a_of,
                    value: Value::Ref(wf),
                    valid_from: TS.into(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            TS,
            Some("tester"),
            Some("seed"),
            g,
        )
        .unwrap();
    // A state transition: the old state is retracted, the new one asserted —
    // the history a current-facts pack would lose.
    store
        .transact_to_graph(
            &[
                Datum {
                    entity: e,
                    attribute: a_state,
                    value: Value::Str("open".into()),
                    valid_from: TS2.into(),
                    valid_to: None,
                    op: Op::Retract,
                },
                Datum {
                    entity: e,
                    attribute: a_state,
                    value: Value::Str("done".into()),
                    valid_from: TS2.into(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            TS2,
            Some("tester"),
            Some("seed"),
            g,
        )
        .unwrap();
    store
        .set_graph_label(
            WINDOW,
            &super::super::labels::GraphLabel {
                kind: Some(crate::lattice_kind::DataKind::parse("operational").unwrap()),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
    g
}

fn rows_via(store: &Store, request: &serde_json::Value) -> serde_json::Value {
    crate::mcp::tool_query(store, request).unwrap()
}

#[test]
fn freeze_keeps_the_graph_readable_via_all_three_opt_ins() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    let q = format!("SELECT ?s ?o WHERE {{ GRAPH <{WINDOW}> {{ ?s <urn:test:state> ?o }} }}");
    let by_from = "SELECT ?s ?o WHERE { ?s <urn:test:state> ?o }".to_string();

    let before = rows_via(&store, &serde_json::json!({ "query": q }));
    assert_eq!(before["count"], 1, "one current state pre-freeze: {before}");

    let report = store
        .freeze_graph(
            WINDOW,
            dir.path().to_str().unwrap(),
            "2026-08-24T01:00:00Z",
            Some("tester"),
        )
        .unwrap();
    assert_eq!(
        report.facts, 4,
        "full history: asserts + the retract row, not 2 current"
    );
    assert!(std::path::Path::new(&report.path).exists());

    // Local rows are gone…
    let local: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM main.facts WHERE g = ?1",
            rusqlite::params![store.lookup(WINDOW).unwrap().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(local, 0, "freeze must relocate, not copy");

    // …but the graph reads identically through each opt-in.
    // (1) By IRI (GRAPH clause).
    let after = rows_via(&store, &serde_json::json!({ "query": q }));
    assert_eq!(after["count"], 1, "GRAPH <iri> reads the archive: {after}");
    assert_eq!(after["rows"][0]["o"], "done");

    // (2) By the auto-maintained frozen dataset.
    let after = rows_via(
        &store,
        &serde_json::json!({ "query": by_from, "graph": FROZEN_DATASET_IRI }),
    );
    assert_eq!(after["count"], 1, "frozen dataset composes: {after}");

    // (3) By kind.
    let after = rows_via(
        &store,
        &serde_json::json!({ "query": by_from, "include_kinds": ["archive"] }),
    );
    assert_eq!(after["count"], 1, "include_kinds composes: {after}");
    // And silence still never widens.
    let silent = rows_via(&store, &serde_json::json!({ "query": by_from }));
    assert_eq!(silent["count"], 0, "default scope stays ROOT-only");
}

#[test]
fn freeze_survives_reopen_and_refuses_a_missing_pack() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db").to_string_lossy().to_string();
    let report = {
        let mut store = Store::open(&path).unwrap();
        seed_window(&mut store);
        store
            .freeze_graph(WINDOW, dir.path().to_str().unwrap(), TS2, None)
            .unwrap()
    };

    // Reopen: the pack auto-attaches from the registry.
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.attachments().len(), 1);
        let q = format!("SELECT ?o WHERE {{ GRAPH <{WINDOW}> {{ ?s <urn:test:state> ?o }} }}");
        let out = rows_via(&store, &serde_json::json!({ "query": q }));
        assert_eq!(out["count"], 1, "archive composes after reopen: {out}");
        // The registry lists it frozen.
        let listed = store.list_graphs(None, Some("frozen")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].iri, WINDOW);
        assert_eq!(listed[0].kind.as_deref(), Some("archive"));
    }

    // A missing pack file is a hard refusal naming the path.
    std::fs::remove_file(&report.path).unwrap();
    let Err(err) = Store::open(&path) else {
        panic!("open must refuse a missing archive pack")
    };
    assert!(
        err.to_string().contains("archive pack") && err.to_string().contains(WINDOW),
        "must name the graph and remedy: {err}"
    );
}

#[test]
fn a_frozen_graph_refuses_writes_naming_thaw() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    let g = seed_window(&mut store);
    store
        .freeze_graph(WINDOW, dir.path().to_str().unwrap(), TS2, None)
        .unwrap();

    let e = store.intern("urn:test:run-2").unwrap();
    let a = store.intern("urn:test:state").unwrap();
    let err = store
        .transact_to_graph(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("open".into()),
                valid_from: TS2.into(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS2,
            None,
            None,
            g,
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("FROZEN") && err.to_string().contains("thaw"),
        "guard must name the state and the remedy: {err}"
    );
}

#[test]
fn thaw_round_trips_the_full_history_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    let g = seed_window(&mut store);
    let before = super::super::freeze_io::history_canonical(&store.conn, g).unwrap();

    store
        .freeze_graph(WINDOW, dir.path().to_str().unwrap(), TS2, None)
        .unwrap();
    let (thawed, vectors) = store
        .thaw_graph(WINDOW, "2026-08-24T02:00:00Z", None)
        .unwrap();
    assert_eq!(thawed, 4);
    assert_eq!(vectors, 0, "this window carries no embeddings");

    // Identical canonical history — freeze/thaw lost nothing.
    let g2 = store.lookup(WINDOW).unwrap().unwrap();
    let after = super::super::freeze_io::history_canonical(&store.conn, g2).unwrap();
    assert_eq!(before, after, "thaw must restore history byte-for-byte");

    // Writable again, out of the frozen dataset and lifecycle.
    assert!(!store.is_dataset(FROZEN_DATASET_IRI).unwrap());
    let e = store.intern("urn:test:run-2").unwrap();
    let a = store.intern("urn:test:state").unwrap();
    store
        .transact_to_graph(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str("open".into()),
                valid_from: "2026-08-24T02:00:01Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-08-24T02:00:01Z",
            None,
            None,
            g2,
        )
        .expect("a thawed graph accepts writes again");
    // Frozen registry row is closed, never deleted.
    let (count, open): (i64, i64) = store
        .conn
        .query_row(
            "SELECT COUNT(*), SUM(thawed_at IS NULL) FROM frozen_packs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, open), (1, 0));
}

#[test]
fn freeze_refuses_root_meta_overlays_and_double_freeze() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    let out = dir.path().to_str().unwrap().to_string();

    assert!(
        store
            .freeze_graph("urn:quipu:graph:root", &out, TS, None)
            .is_err()
    );
    assert!(
        store
            .freeze_graph(crate::namespace::META_GRAPH_IRI, &out, TS, None)
            .is_err()
    );
    store.overlay_create("urn:test:overlay", 0).unwrap();
    let err = store
        .freeze_graph("urn:test:overlay", &out, TS, None)
        .unwrap_err();
    assert!(err.to_string().contains("overlay"), "{err}");

    store.freeze_graph(WINDOW, &out, TS2, None).unwrap();
    let err = store.freeze_graph(WINDOW, &out, TS2, None).unwrap_err();
    assert!(err.to_string().contains("already frozen"), "{err}");
}

// ── Entity embeddings across a freeze (quipu-0v4) ────────────────────────────
//
// The bead this closes recorded the harm as "a frozen graph loses semantic
// search locally". Measuring it first showed that is NOT what happens: freeze
// deletes `main.facts` rows and never touches `main.vectors`, so the freezing
// store searches exactly as it did. The real loss was the ARCHIVE's — a pack
// nothing could re-key embeddings out of, so a graph handed to another store,
// or thawed into a rebuilt one, arrived with no semantic index. These tests
// pin both halves: the local non-loss (so it cannot regress into the loss the
// bead described) and the archive's new completeness.

/// Embed one entity of the seeded window, returning its local id.
fn embed_seeded_entity(store: &Store) -> (i64, Vec<f32>) {
    use crate::vector::KnowledgeVectorStore;
    let e = store.lookup("urn:test:run-1").unwrap().unwrap();
    let emb = vec![1.0f32, 0.0, 0.0, 0.0];
    store
        .embed_entity(e, "triage run one", &emb, TS)
        .expect("embed the window's subject");
    (e, emb)
}

fn vector_count(path: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn freeze_keeps_local_vectors_and_carries_them_into_the_archive() {
    use crate::vector::KnowledgeVectorStore;
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    let (_, emb) = embed_seeded_entity(&store);
    assert_eq!(store.vector_search(&emb, 5, None).unwrap().len(), 1);

    let out = dir.path().to_string_lossy().to_string();
    let report = store.freeze_graph(WINDOW, &out, TS2, None).unwrap();
    assert_eq!(report.vectors, 1, "the archive must carry the embedding");
    assert!(report.vectors_omitted.is_none());

    // The local half: nothing was dropped, and search still answers.
    let after = store.vector_search(&emb, 5, None).unwrap();
    assert_eq!(
        after.len(),
        1,
        "freeze must not cost the freezing store its semantic search"
    );
    assert_eq!(after[0].text, "triage run one");

    // The archive half: the pack carries the row, re-keyed into its own term
    // space by the respace that follows the export.
    assert_eq!(vector_count(&report.path), 1, "pack carries the embedding");

    // And it survives a reopen, where the pack re-attaches.
    drop(store);
    let reopened = Store::open(&dir.path().join("main.db").to_string_lossy()).unwrap();
    assert_eq!(reopened.vector_search(&emb, 5, None).unwrap().len(), 1);
}

#[test]
fn the_archive_carries_only_the_graphs_own_subjects() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    embed_seeded_entity(&store);
    // An entity the window merely POINTS at (the Ref target) and one with no
    // relation to the window at all. Neither is a subject of the frozen
    // graph's facts, so neither belongs in an archive OF that graph.
    {
        use crate::vector::KnowledgeVectorStore;
        let wf = store.lookup("urn:test:wf:triage").unwrap().unwrap();
        store
            .embed_entity(wf, "the triage workflow", &[0.0, 1.0, 0.0, 0.0], TS)
            .unwrap();
        let stranger = store.intern("urn:test:unrelated").unwrap();
        store
            .embed_entity(stranger, "unrelated", &[0.0, 0.0, 1.0, 0.0], TS)
            .unwrap();
    }
    let out = dir.path().to_string_lossy().to_string();
    let report = store.freeze_graph(WINDOW, &out, TS2, None).unwrap();
    assert_eq!(report.vectors, 1, "only the window's own subject travels");
    assert_eq!(vector_count(&report.path), 1);
}

#[test]
fn thaw_restores_the_archives_vectors_and_is_idempotent_against_the_local_rows() {
    use crate::vector::KnowledgeVectorStore;
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    let (_, emb) = embed_seeded_entity(&store);
    let out = dir.path().to_string_lossy().to_string();
    store.freeze_graph(WINDOW, &out, TS2, None).unwrap();

    let (facts, vectors) = store.thaw_graph(WINDOW, TS2, None).unwrap();
    assert!(facts > 0);
    assert_eq!(vectors, 1, "the archive's embedding is restored");
    // The local row was never gone, so the restore must not have duplicated
    // it: `(entity_id, valid_from)` is the primary key and the copy IGNOREs.
    let matches = store.vector_search(&emb, 5, None).unwrap();
    assert_eq!(matches.len(), 1, "no duplicate row: {matches:?}");
}

#[test]
fn freezing_on_a_non_enumerable_backend_says_so_rather_than_shipping_a_silent_gap() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    embed_seeded_entity(&store);
    store.set_vector_search_delegate(std::sync::Arc::new(NoopFreezeDelegate));
    assert!(!store.has_sqlite_vector_backend());

    let out = dir.path().to_string_lossy().to_string();
    let report = store.freeze_graph(WINDOW, &out, TS2, None).unwrap();
    // The freeze itself still succeeds — relocating history is not a vector
    // operation, and refusing it here would be a non-sequitur.
    assert!(report.facts > 0);
    assert_eq!(report.vectors, 0);
    let why = report
        .vectors_omitted
        .expect("an incomplete archive must say it is incomplete");
    assert!(why.contains("enumerated"), "says WHY: {why}");
    assert!(why.contains("LanceDB"), "names the backend class: {why}");
    // And the manifest carries the same confession, for a consumer that never
    // saw the CLI output.
    let conn = rusqlite::Connection::open(&report.path).unwrap();
    let counts: String = conn
        .query_row("SELECT counts FROM pack_manifest WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let counts: serde_json::Value = serde_json::from_str(&counts).unwrap();
    assert_eq!(counts["vectors"], 0);
    assert!(
        counts["vectors_omitted"].is_string(),
        "the pack itself must not read as 'this graph had no embeddings': {counts}"
    );
}

#[test]
fn thaw_refuses_to_restore_vectors_into_a_store_that_cannot_serve_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = file_store(&dir);
    seed_window(&mut store);
    embed_seeded_entity(&store);
    let out = dir.path().to_string_lossy().to_string();
    let report = store.freeze_graph(WINDOW, &out, TS2, None).unwrap();
    assert_eq!(report.vectors, 1);

    // The backend changes under the store between freeze and thaw — the
    // migrate-vectors story, run in the wrong order.
    store.set_vector_search_delegate(std::sync::Arc::new(NoopFreezeDelegate));
    let err = store.thaw_graph(WINDOW, TS2, None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("embedding"),
        "names what it refused over: {msg}"
    );
    assert!(msg.contains("migrate-vectors"), "names the remedy: {msg}");
    // The refusal rolls the whole thaw back rather than half-thawing: the
    // graph is still frozen and still composed.
    let frozen: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM frozen_packs WHERE graph_iri = ?1 AND thawed_at IS NULL",
            rusqlite::params![WINDOW],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(frozen, 1, "a refused thaw leaves the freeze intact");
}

/// Enough of a delegate to make the backend non-SQLite, which is the only
/// property these tests need.
struct NoopFreezeDelegate;

impl crate::vector_delegate::VectorSearchDelegate for NoopFreezeDelegate {
    fn vector_search(
        &self,
        _query: &[f32],
        _limit: usize,
        _valid_at: Option<&str>,
    ) -> Result<Vec<crate::vector::VectorMatch>> {
        Ok(Vec::new())
    }

    fn vector_search_filtered(
        &self,
        _query: &[f32],
        _limit: usize,
        _filter: Option<&str>,
        _valid_at: Option<&str>,
    ) -> Result<Vec<crate::vector::VectorMatch>> {
        Ok(Vec::new())
    }

    fn vector_count(&self) -> Result<usize> {
        Ok(0)
    }
}
