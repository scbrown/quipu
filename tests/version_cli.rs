//! Version inspection must preserve legacy parsers and never open a store.
#![cfg(feature = "shacl")]

use std::process::Command;

#[test]
fn version_aliases_report_build_identity_without_reading_config_or_creating_store() {
    check_version(
        env!("CARGO_BIN_EXE_quipu"),
        "quipu",
        &["--version", "-V", "version"],
    );
}

#[test]
#[cfg(all(feature = "onnx", feature = "server"))]
fn server_version_reports_same_build_identity_without_side_effects() {
    check_version(
        env!("CARGO_BIN_EXE_quipu-server"),
        "quipu-server",
        &["--version", "-V"],
    );
}

fn check_version(binary: &str, name: &str, flags: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".bobbin")).unwrap();
    std::fs::write(dir.path().join(".bobbin/config.toml"), "not valid TOML [").unwrap();
    let db = dir.path().join("must-not-exist.db");
    let home = tempfile::tempdir().unwrap();
    for &flag in flags {
        let output = Command::new(binary)
            .env("HOME", home.path())
            .current_dir(dir.path())
            .args([flag, "--db", db.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success(), "{flag}: {output:?}");
        assert!(output.stderr.is_empty(), "{flag}: {output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = stdout.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], format!("{name} {}", env!("CARGO_PKG_VERSION")));
        // This is the existing collector's `head -1 | awk '{print $NF}'` contract.
        assert_eq!(
            lines[0].split_whitespace().last(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(lines[1], format!("git_sha: {}", env!("QUIPU_GIT_SHA")));
        assert_eq!(lines[2], format!("git_dirty: {}", env!("QUIPU_GIT_DIRTY")));
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
        assert!(!dir.path().join(".quipu").exists());
        assert!(!db.exists(), "version inspection created a database");
        assert!(!dir.path().join(".bobbin/quipu").exists());
    }
}
