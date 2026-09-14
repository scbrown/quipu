//! Pack-time diagnostics must describe omitted vectors, not a restore surprise.
#![cfg(feature = "shacl")]

use std::{path::Path, process::Command};

fn fixture(dir: &Path, vectors: bool) {
    let db = dir.join("source.db");
    drop(quipu::Store::open(db.to_str().unwrap()).unwrap());
    if vectors {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
             VALUES (1, 'fixture', X'00000000', '2026-09-14T00:00:00Z')",
            [],
        )
        .unwrap();
    }
}

fn pack_command(dir: &Path, text: bool, waive: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_quipu"));
    cmd.current_dir(dir).env("HOME", dir).args([
        "pack",
        "--full",
        "--destination",
        "internal",
        "--db",
        "source.db",
        "--out",
        "backup",
    ]);
    if text {
        cmd.args(["--format", "text"]);
    }
    if waive {
        cmd.arg("--allow-missing-embedding-recipe");
    }
    cmd.output().unwrap()
}

fn pack(dir: &Path, text: bool) -> String {
    let result = pack_command(dir, text, false);
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(result.status.success(), "{stderr}");
    assert!(dir.join("backup").exists());
    stderr
}

fn configure_model(dir: &Path, exists: bool) {
    std::fs::create_dir(dir.join(".bobbin")).unwrap();
    std::fs::write(
        dir.join(".bobbin/config.toml"),
        "[quipu.embedding]\nmodel_path = 'model.onnx'\ndimension = 1\n",
    )
    .unwrap();
    if exists {
        // Packing fingerprints the model; it does not execute it.
        std::fs::write(dir.join("model.onnx"), b"fixture model bytes").unwrap();
    }
}

fn assert_refused(dir: &Path) {
    let result = pack_command(dir, true, false);
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert_eq!(result.status.code(), Some(1), "{stderr}");
    assert!(!dir.join("backup").exists());
    assert!(!dir.join("backup.building.db").exists());
    for message in [
        "text pack omits 1 vector row(s)",
        "model_path",
        "not reproducible",
        "--allow-missing-embedding-recipe",
    ] {
        assert!(stderr.contains(message), "{stderr}");
    }
}

#[test]
fn missing_model_refuses_without_writing_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), true);
    assert_refused(dir.path());
}

#[test]
fn explicit_override_writes_an_honest_null_recipe_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), true);
    let result = pack_command(dir.path(), true, true);
    assert!(result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("WARNING: text pack omits 1 vector row(s)")
    );
    let manifest =
        quipu::pack_full_text::read_manifest_dir(dir.path().join("backup").to_str().unwrap())
            .unwrap();
    let counts: serde_json::Value = serde_json::from_str(&manifest.counts).unwrap();
    assert!(counts["regeneration_recipe"]["embedding_model"].is_null());
    assert!(counts["regeneration_recipe"]["embedding_model_sha256"].is_null());
}

#[test]
fn configured_model_records_a_digest_without_warning() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), true);
    configure_model(dir.path(), true);
    let stderr = pack(dir.path(), true);
    assert!(!stderr.contains("WARNING:"), "{stderr}");
    let manifest =
        quipu::pack_full_text::read_manifest_dir(dir.path().join("backup").to_str().unwrap())
            .unwrap();
    let counts: serde_json::Value = serde_json::from_str(&manifest.counts).unwrap();
    let recipe = &counts["regeneration_recipe"];
    assert_eq!(recipe["embedding_model"], "model.onnx");
    use sha2::{Digest, Sha256};
    assert_eq!(
        recipe["embedding_model_sha256"],
        format!("sha256:{:x}", Sha256::digest(b"fixture model bytes"))
    );
    assert_eq!(recipe["embedding_dimension"], 1);
}

#[test]
fn configured_but_missing_model_file_refuses() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), true);
    configure_model(dir.path(), false);
    assert_refused(dir.path());
}

#[test]
fn no_vectors_needs_no_recipe_warning() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    assert!(!pack(dir.path(), true).contains("WARNING:"));
}

#[test]
fn binary_full_pack_carries_vectors_without_recipe_warning() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), true);
    assert!(!pack(dir.path(), false).contains("WARNING:"));
    let conn = rusqlite::Connection::open(dir.path().join("backup")).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
