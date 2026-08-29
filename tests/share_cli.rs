//! End-to-end contract for the git-native `quipu share` command.
#![cfg(feature = "shacl")]

use std::process::Command;

#[test]
fn cli_writes_byte_identical_shares_for_unchanged_state() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("source.db");
    let mut store = quipu::Store::open(db.to_str().unwrap()).unwrap();
    quipu::ingest_rdf(
        &mut store,
        &b"<urn:z> <urn:p> \"last\" .\n<urn:a> <urn:p> \"first\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-08-29T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
        .load_shapes(
            "fixture-shapes",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
            "2026-08-29",
        )
        .unwrap();
    drop(store);

    let first = root.path().join("first");
    let second = root.path().join("second");
    for out in [&first, &second] {
        let result = Command::new(env!("CARGO_BIN_EXE_quipu"))
            .args([
                "share",
                "--output",
                out.to_str().unwrap(),
                "--turtle",
                "--db",
                db.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "quipu share failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("shared sha256:"));
    }

    for file in ["manifest.json", "export.nt", "shapes.ttl", "export.ttl"] {
        assert_eq!(
            std::fs::read(first.join(file)).unwrap(),
            std::fs::read(second.join(file)).unwrap(),
            "{file} changed although graph state did not"
        );
    }
}

#[test]
fn cli_refuses_empty_shapes_unless_explicitly_requested() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("empty.db");
    drop(quipu::Store::open(db.to_str().unwrap()).unwrap());

    let refused = root.path().join("refused");
    let result = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args([
            "share",
            "--output",
            refused.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no loaded shape sets"));
    assert!(!refused.exists());

    let explicit = root.path().join("explicit");
    let result = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args([
            "share",
            "--output",
            explicit.to_str().unwrap(),
            "--no-shapes",
            "--db",
            db.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(
        std::fs::metadata(explicit.join("shapes.ttl"))
            .unwrap()
            .len(),
        0
    );
}
