//! Real CLI acceptance: shell bypass exit 1, control exit 0, unknown exit 2.
#![cfg(feature = "shacl")]

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn audit(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(dir)
        .args(["audit", "trace.jsonl", "--db", "store.db", "--json"])
        .args(args)
        .output()
        .unwrap()
}
fn seed(dir: &Path) {
    fs::write(dir.join("trace.jsonl"), "").unwrap();
    let mut store = quipu::Store::open(dir.join("store.db").to_str().unwrap()).unwrap();
    quipu::ingest_rdf(
        &mut store,
        &include_bytes!("../shapes/policies/tripwire.ttl")[..],
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        Some("git-audit-fixture"),
    )
    .unwrap();
}
fn result(o: &Output, expected: i32) -> serde_json::Value {
    assert_eq!(
        o.status.code(),
        Some(expected),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
#[test]
fn committed_shell_bypass_changes_exit_zero_to_one_with_controls() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    seed(dir);
    git(dir, &["init", "-q"]);
    git(dir, &["commit", "--allow-empty", "-qm", "base"]);
    let base = git(dir, &["rev-parse", "HEAD"]);
    fs::write(dir.join("README.md"), "ok").unwrap();
    git(dir, &["add", "README.md"]);
    git(dir, &["commit", "-qm", "control"]);
    let flags = ["--repo", ".", "--from", &base, "--to", "HEAD"];
    let clean = result(&audit(dir, &flags), 0);
    assert_eq!(clean["git"]["commits_checked"], 1);
    assert_eq!(clean["git"]["policies_checked"], 2);
    fs::create_dir_all(dir.join("src/auth")).unwrap();
    fs::write(dir.join("src/auth/x"), "shell bypass").unwrap();
    git(dir, &["add", "src/auth/x"]);
    git(dir, &["commit", "-qm", "shell bypass"]);
    let legacy = result(&audit(dir, &[]), 0);
    assert_eq!(legacy["conforms"], true);
    let checked = result(&audit(dir, &flags), 1);
    assert_eq!(checked["conforms"], false);
    assert_eq!(checked["git"]["commits_checked"], 2);
    assert!(checked["findings"].as_array().unwrap().iter().any(|f| {
        f["detail"]
            .as_str()
            .unwrap()
            .contains("bypassed enforcement")
    }));
    fs::write(dir.join("trace.jsonl"), "{broken\n").unwrap();
    let bad = audit(dir, &flags);
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unreadable"));
}
#[test]
fn incomplete_flags_and_missing_refs_never_return_a_clean_scan() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    seed(dir);
    assert_eq!(audit(dir, &["--repo", "."]).status.code(), Some(2));
    git(dir, &["init", "-q"]);
    git(dir, &["commit", "--allow-empty", "-qm", "base"]);
    assert_eq!(
        audit(dir, &["--repo", ".", "--from", "absent", "--to", "HEAD"])
            .status
            .code(),
        Some(2)
    );
}
