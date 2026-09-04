use std::fs;
use std::path::PathBuf;

use crate::store::Store;
use crate::{Datum, Value};

use super::import_graph;

/// The `TempDir` is returned so the caller HOLDS it: dropping it removes the
/// two stores, including when the test panics. Returning bare paths leaked a
/// directory per process, per name (aegis-t4oyjy).
fn paths(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("quipu-import-{name}-"))
        .tempdir()
        .unwrap();
    let local = dir.path().join("local.db");
    let foreign = dir.path().join("foreign.db");
    (dir, local, foreign)
}

#[test]
fn imports_with_eager_alias_remap_and_ref_reachability() {
    let (_tmp, local, foreign) = paths("aliases");
    let dst = Store::open(local.to_str().unwrap()).unwrap();
    let shared_local = dst.intern("urn:shared").unwrap();
    let collision_local = dst.intern("urn:local-only").unwrap();

    let mut src = Store::open(foreign.to_str().unwrap()).unwrap();
    let collision_foreign = src.intern("urn:foreign-only").unwrap();
    let shared_foreign = src.intern("urn:shared").unwrap();
    assert_eq!(
        shared_local, collision_foreign,
        "fixture must contain different IRIs at the same raw id"
    );
    assert_ne!(
        shared_local, shared_foreign,
        "fixture must contain the same IRI at different raw ids"
    );
    let p = src.intern("urn:p").unwrap();
    src.transact(
        &[Datum {
            entity: shared_foreign,
            attribute: p,
            value: Value::Ref(collision_foreign),
            valid_from: "2026-01-01T00:00:00Z".into(),
            valid_to: None,
            op: crate::Op::Assert,
        }],
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    drop(src);

    let before = fs::read(&foreign).unwrap();
    let report = import_graph(&local, &foreign, "urn:imported").unwrap();
    assert_eq!(
        fs::read(&foreign).unwrap(),
        before,
        "source must remain byte-identical"
    );
    drop(dst);
    let store = Store::open(local.to_str().unwrap()).unwrap();
    assert_eq!(store.lookup("urn:shared").unwrap(), Some(shared_local));
    assert_eq!(
        store.lookup("urn:local-only").unwrap(),
        Some(collision_local)
    );
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(
        &store,
        "SELECT ?o WHERE { GRAPH <urn:imported> { <urn:shared> <urn:p> ?o } }",
    )
    .unwrap() else {
        panic!("expected SELECT")
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("o"),
        Some(&Value::Ref(
            store.lookup("urn:foreign-only").unwrap().unwrap()
        ))
    );
    assert_eq!(report.facts, 1);
}

#[test]
fn unclassified_source_column_refuses_before_destination_write() {
    let (_tmp, local, foreign) = paths("unknown-column");
    drop(Store::open(local.to_str().unwrap()).unwrap());
    drop(Store::open(foreign.to_str().unwrap()).unwrap());
    rusqlite::Connection::open(&foreign)
        .unwrap()
        .execute_batch("ALTER TABLE facts ADD COLUMN mystery INTEGER")
        .unwrap();
    let before = fs::read(&local).unwrap();
    let err = import_graph(&local, &foreign, "urn:nope")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("facts.mystery") && err.contains("COLUMN_CLASSIFICATION"),
        "{err}"
    );
    assert_eq!(fs::read(&local).unwrap(), before);
}

#[test]
fn import_carries_embeddings_re_keyed_by_iri() {
    // The recipient half of quipu-0v4: an archive (or any `--with-vectors`
    // pack) handed to another store must arrive WITH its semantic index. This
    // path silently dropped every `vectors` row before, which made
    // `pack --with-vectors` produce a file nothing could restore from.
    use crate::vector::KnowledgeVectorStore;
    let (_tmp, local, foreign) = paths("vectors");
    // A destination that already interns an unrelated IRI, so the id spaces
    // differ and the join has to be by IRI rather than by raw id.
    let dst = Store::open(local.to_str().unwrap()).unwrap();
    dst.intern("urn:local-decoy").unwrap();
    drop(dst);

    let mut src = Store::open(foreign.to_str().unwrap()).unwrap();
    let e = src.intern("urn:subject").unwrap();
    let p = src.intern("urn:p").unwrap();
    src.transact(
        &[Datum {
            entity: e,
            attribute: p,
            value: Value::Str("hello".into()),
            valid_from: "2026-01-01T00:00:00Z".into(),
            valid_to: None,
            op: crate::Op::Assert,
        }],
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let emb = vec![0.0f32, 1.0, 0.0, 0.0];
    src.embed_entity(e, "the subject", &emb, "2026-01-01T00:00:00Z")
        .unwrap();
    drop(src);

    let report = import_graph(&local, &foreign, "urn:imported").unwrap();
    assert_eq!(report.vectors, 1, "the embedding must travel");

    let store = Store::open(local.to_str().unwrap()).unwrap();
    let matches = store.vector_search(&emb, 5, None).unwrap();
    assert_eq!(matches.len(), 1, "and be searchable here: {matches:?}");
    assert_eq!(
        store.resolve(matches[0].entity_id).unwrap(),
        "urn:subject",
        "re-keyed onto THIS store's id for that IRI"
    );
}
