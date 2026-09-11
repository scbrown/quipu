//! ROOT selection must be expressible through the actual command line.
#![cfg(feature = "shacl")]

use std::process::Command;

#[test]
fn omitted_graph_packs_root_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("source.db");
    let out = dir.path().join("root.qpack.db");
    let mut store = quipu::Store::open(db.to_str().unwrap()).unwrap();
    quipu::ingest_rdf(
        &mut store,
        &b"<urn:test:s> <urn:test:p> \"root content\" ."[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-09T00:00:00Z",
        None,
        Some("pack-cli-test"),
    )
    .unwrap();
    drop(store);
    let result = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args([
            "pack",
            "--out",
            out.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = quipu::pack::read_manifest(out.to_str().unwrap()).unwrap();
    assert_eq!(manifest.source_graph, quipu::schema::ROOT_GRAPH_IRI);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&manifest.counts).unwrap()["facts"],
        1
    );
    let verify = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args(["pack", "--verify", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
}
