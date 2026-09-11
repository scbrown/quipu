//! Archive imports must use the operator's database without trusting pack shapes.
#![cfg(feature = "shacl")]

use std::path::Path;
use std::process::Command;

const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n<urn:WidgetShape> a sh:NodeShape; sh:targetClass <urn:Widget> .";

fn cli(root: &Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(root)
        .env("HOME", root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture(root: &Path) -> (String, String) {
    let mut source = quipu::Store::open_in_memory().unwrap();
    source.load_shapes("fixture", SHAPES, "2026-09-11").unwrap();
    quipu::ingest_rdf(
        &mut source,
        &b"<urn:item> a <urn:Widget> ."[..],
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-09-11",
        None,
        None,
    )
    .unwrap();
    let dir = root.join("share");
    quipu::share::share(&source, dir.to_str().unwrap(), &Default::default()).unwrap();
    let archive = root.join("share.qpack.tar.gz");
    let zip = flate2::write::GzEncoder::new(
        std::fs::File::create(&archive).unwrap(),
        flate2::Compression::default(),
    );
    let mut tar = tar::Builder::new(zip);
    for name in ["manifest.json", "export.nt", "shapes.ttl"] {
        tar.append_path_with_name(dir.join(name), name).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
    (
        dir.to_str().unwrap().into(),
        archive.to_str().unwrap().into(),
    )
}

#[test]
fn directory_and_archive_stage_in_the_selected_database_and_promote_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let (dir, archive) = fixture(root.path());
    for (index, reference) in [dir, archive].iter().enumerate() {
        let db = root.path().join(format!("receiver-{index}.db"));
        let receiver = quipu::Store::open(db.to_str().unwrap()).unwrap();
        receiver
            .load_shapes("fixture", SHAPES, "2026-09-11")
            .unwrap();
        drop(receiver);
        let imported = cli(
            root.path(),
            &["import", reference, "--db", db.to_str().unwrap()],
        );
        assert_eq!(imported["outcome"], "staged", "{imported}");
        assert_eq!(imported["triples"]["accepted"], 1);
        assert_eq!(
            imported["validation"]["off_vocabulary"],
            serde_json::json!([])
        );
        let promoted = cli(
            root.path(),
            &[
                "import",
                "promote",
                imported["share_id"].as_str().unwrap(),
                "--db",
                db.to_str().unwrap(),
            ],
        );
        assert_eq!(promoted["outcome"], "promoted", "{promoted}");
        assert_eq!(promoted["triples"], 1);
    }
}

#[test]
fn archive_does_not_adopt_carried_shapes_or_create_a_default_database() {
    let root = tempfile::tempdir().unwrap();
    let (_, archive) = fixture(root.path());
    let transient = cli(root.path(), &["import", &archive]);
    assert_eq!(transient["outcome"], "quarantined");
    assert!(!root.path().join(".bobbin").exists());
    let db = root.path().join("empty.db");
    let persisted = cli(
        root.path(),
        &["import", &archive, "--db", db.to_str().unwrap()],
    );
    assert_eq!(persisted["outcome"], "quarantined");
    assert_eq!(
        persisted["validation"]["off_vocabulary"],
        serde_json::json!(["urn:Widget"])
    );
    assert!(db.exists(), "an explicit --db must not be silently ignored");
}
