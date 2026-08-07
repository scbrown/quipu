use std::fs;
use std::path::PathBuf;

use crate::store::Store;
use crate::{Datum, Value};

use super::import_graph;

fn paths(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("quipu-import-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    (dir.join("local.db"), dir.join("foreign.db"))
}

#[test]
fn imports_with_eager_alias_remap_and_ref_reachability() {
    let (local, foreign) = paths("aliases");
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
    let (local, foreign) = paths("unknown-column");
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
