//! Real Git controls for inherited versus newly introduced merge state.
use super::repository;
use super::tests::{Fixture, bypasses, store, trace};
use std::fs;

fn evidence(commit: &str, path: &str) -> super::TraceRecord {
    let mut record = trace(commit, path);
    record.constraints[0].outcome = Some("satisfied".into());
    record.constraints[0].response = Some("no-action".into());
    record
}

#[test]
fn traced_side_change_and_clean_merge_have_no_bypass_but_new_merge_content_does() {
    for effect in ["record", "warn", "escalate"] {
        for evil in [false, true] {
            let f = Fixture::new();
            let main = f.git(&["branch", "--show-current"]);
            f.git(&["checkout", "-qb", "side"]);
            let side = f.commit("src/governed/x", "side content");
            f.git(&["checkout", &main]);
            f.commit("README.md", "control");
            f.git(&["merge", "--no-ff", "--no-commit", "side"]);
            if evil {
                fs::write(f.dir.path().join("src/governed/x"), "merge-only content").unwrap();
                f.git(&["add", "src/governed/x"]);
            }
            f.git(&["commit", "-qm", "merge"]);
            let (report, scope) = f.check(
                &store(effect, &["src/governed/**"], false),
                &[evidence(&side, "src/governed/x")],
            );
            assert_eq!(
                bypasses(&report),
                usize::from(evil),
                "effect={effect} evil={evil}"
            );
            assert_eq!(scope.paths_checked, 2 + usize::from(evil));
            assert_eq!(report.conforms(), !evil);
        }
    }
}

#[test]
fn octopus_merge_carries_each_parent_without_duplicate_coverage() {
    let f = Fixture::new();
    let main = f.git(&["branch", "--show-current"]);
    let mut traces = Vec::new();
    for branch in ["side-a", "side-b"] {
        f.git(&["checkout", "-qb", branch, &f.base]);
        let path = format!("src/governed/{branch}");
        let commit = f.commit(&path, branch);
        traces.push(evidence(&commit, &path));
    }
    f.git(&["checkout", &main]);
    f.commit("README.md", "main control");
    f.git(&["merge", "--no-ff", "-qm", "octopus", "side-a", "side-b"]);
    let (report, scope) = f.check(&store("record", &["src/governed/**"], false), &traces);
    assert_eq!(scope.commits_checked, 4);
    assert_eq!(scope.paths_checked, 3);
    assert_eq!(bypasses(&report), 0);
    assert!(repository::paths(f.dir.path(), "HEAD").unwrap().is_empty());
}

#[test]
fn merge_only_deletion_and_mode_change_are_still_evidence() {
    for mode_change in [false, true] {
        let f = Fixture::new();
        let inherited = f.commit("src/governed/x", "shared");
        let main = f.git(&["branch", "--show-current"]);
        f.git(&["checkout", "-qb", "side"]);
        f.commit("side.txt", "side");
        f.git(&["checkout", &main]);
        f.commit("README.md", "main");
        f.git(&["merge", "--no-ff", "--no-commit", "side"]);
        if mode_change {
            f.git(&["update-index", "--chmod=+x", "src/governed/x"]);
        } else {
            f.git(&["rm", "src/governed/x"]);
        }
        f.git(&["commit", "-qm", "new merge state"]);
        let (report, _) = f.check(
            &store("record", &["src/governed/**"], false),
            &[evidence(&inherited, "src/governed/x")],
        );
        assert_eq!(bypasses(&report), 1, "mode_change={mode_change}");
        assert_eq!(
            repository::paths(f.dir.path(), "HEAD")
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["src/governed/x"]
        );
    }
}
