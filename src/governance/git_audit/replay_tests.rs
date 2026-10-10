//! Protocol tests are always available; the real-engine acceptance is separate.
use super::*;

fn executable() -> std::path::PathBuf {
    // A tracked script: generating an executable during parallel fork/exec
    // tests can leave its writable fd briefly inherited, causing ETXTBSY.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audit-replay.sh")
}

#[test]
fn protocol_requires_consistent_status_and_identity() {
    let rule = serde_json::json!({"name":"test-policy"});
    for verdict in ["satisfied", "unsatisfied", "unknown", "not_applicable"] {
        assert_eq!(
            run(&executable(), &rule, "src/x.rs", verdict)
                .unwrap()
                .verdict,
            verdict
        );
        assert!(run(&executable(), &rule, "another.rs", verdict).is_err());
    }
    for source in ["wrong-status", "inconsistent", "invalid-json"] {
        assert!(run(&executable(), &rule, "src/x.rs", source).is_err());
    }
}

#[test]
fn runaway_evaluator_is_killed_and_never_reported_clean() {
    let start = std::time::Instant::now();
    assert!(
        run(
            &executable(),
            &serde_json::json!({"name":"test-policy"}),
            "src/x.rs",
            "timeout"
        )
        .is_err()
    );
    assert!(start.elapsed().as_secs() < 20);
}
