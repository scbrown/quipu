//! Shadow gate I/O tests: every refusal arm, and the accepted open. Size-exempt.

use super::*;

fn fixture_store(dir: &Path) -> PathBuf {
    let path = dir.join("copy.db");
    {
        let mut store = Store::open(path.to_str().unwrap()).unwrap();
        let d = vec![crate::store::Datum {
            entity: store.intern("http://ex/a").unwrap(),
            attribute: store.intern("http://ex/p").unwrap(),
            value: crate::types::Value::Str("o".into()),
            valid_from: "2026-01-01T00:00:00Z".into(),
            valid_to: None,
            op: crate::types::Op::Assert,
        }];
        store
            .transact(&d, "2026-01-01T00:00:00Z", None, None)
            .unwrap();
    }
    // A clean close checkpoints the WAL; clear any leftover sidecars so the
    // fixture is the quiescent copy an operator would hand over.
    for s in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

#[test]
fn refuses_the_configured_live_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture_store(dir.path());
    let r = refusal(&path, Some(&path));
    assert!(matches!(r, Some(Refusal::LiveStore(_))), "{r:?}");
    let err = open_quiescent_copy(&path, Some(&path)).err().unwrap();
    assert!(err.to_string().contains("quiescent COPY"), "{err}");
}

#[test]
fn refuses_each_active_sidecar() {
    for suffix in ["-wal", "-shm", "-journal"] {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_store(dir.path());
        std::fs::write(format!("{}{suffix}", path.display()), b"").unwrap();
        let r = refusal(&path, None);
        assert!(
            matches!(&r, Some(Refusal::ActiveSidecar(p)) if p.to_string_lossy().ends_with(suffix)),
            "{suffix}: {r:?}"
        );
    }
}

#[test]
fn refuses_a_store_held_open_by_a_live_connection() {
    // The real case the sidecar rule stands for: a writer has it open.
    let dir = tempfile::tempdir().unwrap();
    let path = fixture_store(dir.path());
    let _writer = Store::open(path.to_str().unwrap()).unwrap();
    assert!(matches!(
        refusal(&path, None),
        Some(Refusal::ActiveSidecar(_))
    ));
}

#[test]
fn refuses_a_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let r = refusal(&dir.path().join("nope.db"), None);
    assert!(matches!(r, Some(Refusal::Missing(_))));
}

#[test]
fn opens_a_quiescent_copy_immutably_and_leaves_it_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture_store(dir.path());
    let before = std::fs::read(&path).unwrap();
    {
        let store = open_quiescent_copy(&path, None).unwrap();
        assert_eq!(store.latest_tx_id().unwrap(), 1);
        // The handle cannot write, structurally.
        assert!(
            store
                .prepare("DELETE FROM facts")
                .unwrap()
                .execute([])
                .is_err()
        );
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the copy must be untouched"
    );
    assert!(
        refusal(&path, None).is_none(),
        "an immutable read leaves no sidecar"
    );
}
