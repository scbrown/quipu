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
    let policy_graph = store.overlay_create("urn:test:catalogue", 0).unwrap();
    quipu::rdf::ingest_rdf_to_graph(
        &mut store,
        include_bytes!("fixtures/share-catalogue.ttl").as_slice(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-09-09T00:00:00Z",
        None,
        Some("pack-cli-policy"),
        policy_graph,
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

/// An unrecognized `--format` must be REFUSED, not defaulted (aegis-jpmgm8).
///
/// The write path used to test `--format` by equality against each value it
/// knew and fall through otherwise, so a typo — or the documented
/// `--format text` typed against a CLI predating it — produced a DIFFERENT
/// ARTIFACT at the requested path and reported success with a valid content
/// hash. Measured on the CLI installed at 27f6d452: a 274 KB binary SQLite
/// whole-store pack where a text DIRECTORY was asked for.
///
/// The substitution is the dangerous direction: a binary full pack carries
/// `events` and `vectors`, which the text pack deliberately does not.
#[test]
fn an_unknown_pack_format_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("source.db");
    let out = dir.path().join("refused");
    let mut store = quipu::Store::open(db.to_str().unwrap()).unwrap();
    quipu::ingest_rdf(
        &mut store,
        &b"<urn:test:s> <urn:test:p> \"content\" ."[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-14T00:00:00Z",
        None,
        Some("pack-format-test"),
    )
    .unwrap();
    drop(store);

    let result = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args([
            "pack",
            "--full",
            "--format",
            "bogus",
            "--destination",
            "internal",
            "--out",
            out.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        result.status.code(),
        Some(2),
        "an unknown format must exit 2, got {:?}: {}",
        result.status.code(),
        String::from_utf8_lossy(&result.stderr)
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("unknown --format") && stderr.contains("bogus"),
        "the refusal must name the offending value: {stderr}"
    );
    assert!(
        stderr.contains("turtle") && stderr.contains("text"),
        "the refusal must name what IS accepted: {stderr}"
    );
    // The point of the bead: a refused pack leaves no artifact behind for
    // someone to pick up believing it is the one they asked for.
    assert!(
        !out.exists(),
        "a refused --format must write nothing at the requested path"
    );
}

/// CONTROL for the test above: the accepted values still reach their dispatch.
///
/// Without this, tightening the refusal until it rejects everything would leave
/// the test above passing.
#[test]
fn the_accepted_pack_formats_still_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("source.db");
    let out = dir.path().join("textpack");
    let mut store = quipu::Store::open(db.to_str().unwrap()).unwrap();
    quipu::ingest_rdf(
        &mut store,
        &b"<urn:test:s> <urn:test:p> \"content\" ."[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-14T00:00:00Z",
        None,
        Some("pack-format-control"),
    )
    .unwrap();
    drop(store);

    let result = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args([
            "pack",
            "--full",
            "--format",
            "text",
            "--destination",
            "internal",
            "--out",
            out.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "`--format text` must still work: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        out.is_dir(),
        "`--format text` must produce a DIRECTORY, not a file"
    );
}
