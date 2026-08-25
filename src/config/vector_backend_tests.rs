//! Tests for `vector.backend` selection (quipu-lv7).
//!
//! The load-bearing one is
//! [`configured_lancedb_is_installed_and_actually_serves_the_store`]: it
//! asserts an OBSERVABLE EFFECT — an embedding written through the store lands
//! in `LanceDB` and NOT in the `SQLite` table — because "the call succeeded" is
//! exactly what a set-but-unread config key also does.

use super::*;
use crate::config::{QuipuConfig, VectorBackend};
use crate::store::Store;

fn config_with_backend(backend: VectorBackend, path: &str) -> QuipuConfig {
    QuipuConfig {
        vector: crate::config::VectorConfig {
            backend,
            lancedb_path: path.into(),
        },
        ..Default::default()
    }
}

#[test]
fn the_sqlite_backend_installs_nothing() {
    // The default must stay a no-op: a store already uses its own table, and
    // "installing" it would be a second object answering the same question.
    let mut store = Store::open_in_memory().unwrap();
    let installed =
        install_vector_backend(&mut store, &config_with_backend(VectorBackend::Sqlite, ""))
            .unwrap();
    assert!(installed.is_none(), "sqlite must report nothing installed");
    assert!(!store.has_local_vector_backend());
    assert!(store.has_sqlite_vector_backend());
}

#[cfg(not(feature = "lancedb"))]
#[test]
fn configured_lancedb_without_the_feature_is_a_hard_error() {
    // The honest half of the fix. This binary cannot construct the backend, so
    // it says so — rather than warning and then answering every search out of
    // the SQLite table a migrated deployment has stopped writing to.
    let mut store = Store::open_in_memory().unwrap();
    let err = install_vector_backend(
        &mut store,
        &config_with_backend(VectorBackend::Lancedb, "/tmp/quipu-vectors"),
    )
    .expect_err("a backend this build cannot construct must refuse");
    let msg = err.to_string();
    assert!(msg.contains("lancedb"), "names the feature: {msg}");
    assert!(
        msg.contains("migrate-vectors"),
        "names the failure mode it is avoiding: {msg}"
    );
    assert!(
        msg.contains("sqlite"),
        "names the other way out of the error: {msg}"
    );
    assert!(
        !store.has_local_vector_backend(),
        "a refused install must not half-install"
    );
}

#[cfg(feature = "lancedb")]
#[tokio::test(flavor = "multi_thread")]
async fn configured_lancedb_is_installed_and_actually_serves_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let lance = dir.path().join("vectors").to_string_lossy().to_string();
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("urn:test:subject").unwrap();

    let installed = install_vector_backend(
        &mut store,
        &config_with_backend(VectorBackend::Lancedb, &lance),
    )
    .unwrap();
    assert_eq!(installed.as_deref(), Some(lance.as_str()));
    assert!(store.has_local_vector_backend());
    assert!(
        !store.has_sqlite_vector_backend(),
        "the configured backend must REPLACE the built-in table, not sit beside it"
    );

    // The effect, not the call: a write through the store's chosen backend
    // reaches LanceDB and is searchable there.
    let emb = vec![1.0f32; 384];
    store
        .vector_store()
        .embed_entity(e, "the subject", &emb, "2026-01-01T00:00:00Z")
        .unwrap();
    let hits = store.vector_store().vector_search(&emb, 5, None).unwrap();
    assert_eq!(hits.len(), 1, "the installed backend must answer: {hits:?}");
    assert_eq!(hits[0].entity_id, e);

    // And the SQLite table stayed empty — which is what proves the write did
    // not simply go where it always went.
    let sqlite_rows: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        sqlite_rows, 0,
        "the built-in table must be bypassed once a backend is selected"
    );
}
