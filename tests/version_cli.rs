//! Version inspection must preserve legacy parsers and never open a store.
#![cfg(feature = "shacl")]

use std::process::Command;

#[test]
fn version_aliases_report_build_identity_without_reading_config_or_creating_store() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".bobbin")).unwrap();
    std::fs::write(dir.path().join(".bobbin/config.toml"), "not valid TOML [").unwrap();
    let db = dir.path().join("must-not-exist.db");
    for flag in ["--version", "-V", "version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_quipu"))
            .current_dir(dir.path())
            .args([flag, "--db", db.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success(), "{flag}: {output:?}");
        assert!(output.stderr.is_empty(), "{flag}: {output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = stdout.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], format!("quipu {}", env!("CARGO_PKG_VERSION")));
        // This is the existing collector's `head -1 | awk '{print $NF}'` contract.
        assert_eq!(
            lines[0].split_whitespace().last(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(lines[1], format!("git_sha: {}", env!("QUIPU_GIT_SHA")));
        assert!(!db.exists(), "version inspection created a database");
        assert!(!dir.path().join(".bobbin/quipu").exists());
    }
}
