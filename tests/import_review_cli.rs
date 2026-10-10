//! Receiver selection and review policy through the actual native CLI.
use quipu::{
    Store,
    share::{ShareDestination, ShareOptions},
};
use std::process::Command;

#[test]
fn review_cli_selects_receiver_and_preserves_quarantine_until_named_decision() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = Store::open_in_memory().unwrap();
    quipu::rdf::ingest_rdf(&mut source,&b"<https://example.org/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://foreign.example/Unknown> .\n"[..],oxrdfio::RdfFormat::NTriples,None,"2026-10-09T00:00:00Z",None,None).unwrap();
    let pack = dir.path().join("share");
    let manifest = quipu::share::share(
        &source,
        pack.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            destination: ShareDestination::Internal,
            ..Default::default()
        },
    )
    .unwrap();
    let db = dir.path().join("receiver.db");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_quipu"))
            .args(args)
            .args(["--db", db.to_str().unwrap()])
            .env("HOME", dir.path())
            .output()
            .unwrap()
    };
    let imported = run(&[
        "import",
        pack.to_str().unwrap(),
        "--destination",
        "internal",
    ]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let pending = run(&["import", "review", "pending", "--limit", "1"]);
    assert!(pending.status.success());
    let page: serde_json::Value = serde_json::from_slice(&pending.stdout).unwrap();
    assert_eq!(page["reviews"][0]["share_id"], manifest.share_id);
    assert_eq!(page["has_more"], false);
    assert!(
        !run(&[
            "import",
            "review",
            "notify",
            "--age-seconds",
            "0",
            "--route",
            "reviewer"
        ])
        .status
        .success()
    );
    assert!(
        !run(&[
            "import",
            "review",
            "rejected",
            &manifest.share_id,
            "--reason",
            "untrusted"
        ])
        .status
        .success()
    );
    assert!(
        run(&[
            "import",
            "review",
            "rejected",
            &manifest.share_id,
            "--actor",
            "operator",
            "--reason",
            "untrusted"
        ])
        .status
        .success()
    );
    assert!(
        !run(&[
            "import",
            pack.to_str().unwrap(),
            "--destination",
            "internal"
        ])
        .status
        .success()
    );
    let store = Store::open(db.to_str().unwrap()).unwrap();
    let events = store
        .events_after(
            0,
            10,
            Some(&["import.quarantined".into(), "import.rejected".into()]),
            None,
        )
        .unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0].offset < events[1].offset);
    assert_eq!(events[1].event_type, "import.rejected");
    assert!(
        run(&[
            "import",
            "review",
            "reopen",
            &manifest.share_id,
            "--actor",
            "operator",
            "--reason",
            "reconsider"
        ])
        .status
        .success()
    );
}
