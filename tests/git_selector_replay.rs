//! End-to-end replay with an explicitly supplied real Yupana binary.
//! Run with `YUPANA_AUDIT_BIN=/path/to/yupana cargo test --features shacl`
//! plus `--test git_selector_replay -- --ignored`. No mocks in this test.
#![cfg(feature = "shacl")]
use std::{fs, path::Path, process::Command};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn audit(dir: &Path, base: &str, to: &str, executable: &str) -> (i32, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(dir)
        .args([
            "audit",
            "trace.jsonl",
            "--db",
            "store.db",
            "--json",
            "--repo",
            ".",
            "--from",
            base,
            "--to",
            to,
            "--yupana",
            executable,
        ])
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"));
    (output.status.code().unwrap(), value)
}
fn trace(dir: &Path, commit: &str) {
    let entry = serde_json::json!({"git_commit":commit,"path":"src/example.rs",
        "constraints":[{"id":"todo-needs-ticket","outcome":"satisfied","response":"no-action"}]});
    fs::write(dir.join("trace.jsonl"), entry.to_string()).unwrap();
}
#[test]
#[ignore = "requires the separately built real Yupana audit-rule executable"]
fn real_parser_replays_the_commit_even_when_the_worktree_and_trace_claim_clean() {
    let executable = std::env::var("YUPANA_AUDIT_BIN").expect("set YUPANA_AUDIT_BIN");
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut store = quipu::Store::open(dir.join("store.db").to_str().unwrap()).unwrap();
    // Keep the canonical TODO policy and its atoms, before the separate inverse policy.
    let catalogue = include_str!("../shapes/policies/treesitter.ttl")
        .split("# no-ticket-in-comment —")
        .next()
        .unwrap();
    quipu::ingest_rdf(
        &mut store,
        catalogue.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        Some("replay-fixture"),
    )
    .unwrap();
    drop(store);
    git(dir, &["init", "-q"]);
    git(dir, &["commit", "--allow-empty", "-qm", "base"]);
    let base = git(dir, &["rev-parse", "HEAD"]);
    fs::create_dir(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/example.rs"),
        "// TODO finish this\nfn f() {}\n",
    )
    .unwrap();
    git(dir, &["add", "src/example.rs"]);
    git(dir, &["commit", "-qm", "shell edit without hook"]);
    let bad = git(dir, &["rev-parse", "HEAD"]);
    fs::write(dir.join("trace.jsonl"), "").unwrap();
    fs::write(
        dir.join("src/example.rs"),
        "// TODO APP-12 finish this\nfn f() {}\n",
    )
    .unwrap();
    let (code, report) = audit(dir, &base, &bad, &executable);
    assert_eq!(code, 1, "{report}");
    assert_eq!(report["git"]["selectors_checked"], 1);
    assert!(report["findings"].as_array().unwrap().iter().any(|f| {
        f["detail"]
            .as_str()
            .unwrap()
            .contains("selector claim unsatisfied")
    }));
    trace(dir, &bad);
    assert_eq!(
        audit(dir, &base, &bad, &executable).0,
        1,
        "a clean trace cannot excuse bad source"
    );
    git(dir, &["add", "src/example.rs"]);
    git(dir, &["commit", "-qm", "ticket control"]);
    let good = git(dir, &["rev-parse", "HEAD"]);
    trace(dir, &good);
    let (code, report) = audit(dir, &bad, &good, &executable);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["git"]["selectors_checked"], 1);
    assert_eq!(report["git"]["unresolved"], 0);
    let (code, report) = audit(dir, &bad, &good, "/absent/yupana");
    assert_eq!(code, 2, "{report}");
    assert_eq!(report["git"]["unresolved"], 1);
}
