//! Git fixtures exercise history rather than mocking its path enumeration.
use super::*;
use crate::{
    namespace::{DEFAULT_BASE_NS, RDF_TYPE},
    store::Datum,
    types::{Op, Value},
};
use std::{fs, process::Command};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    base: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut f = Self {
            dir,
            base: String::new(),
        };
        f.git(&["init", "-q"]);
        f.git(&["commit", "--allow-empty", "-qm", "base"]);
        f.base = f.git(&["rev-parse", "HEAD"]);
        f
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.dir.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Agent")
            .env("GIT_AUTHOR_EMAIL", "agent@example.org")
            .env("GIT_COMMITTER_NAME", "Fixture Agent")
            .env("GIT_COMMITTER_EMAIL", "agent@example.org")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn commit(&self, path: &str, content: &str) -> String {
        let p = self.dir.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        self.git(&["add", "--", path]);
        self.git(&["commit", "-qm", "shell edit", "-m", "SHANTY_AGENT: fixture"]);
        self.git(&["rev-parse", "HEAD"])
    }
    fn check(&self, store: &Store, trace: &[TraceRecord]) -> (Report, Scope) {
        let mut report = super::super::audit::check(store, trace, 0).unwrap();
        let scope = reconcile(
            store,
            trace,
            self.dir.path(),
            &self.base,
            "HEAD",
            &mut report,
        )
        .unwrap();
        (report, scope)
    }
}
fn store(effect: &str, paths: &[&str], selector: bool) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let entity = store
        .intern(&format!("{DEFAULT_BASE_NS}path-policy"))
        .unwrap();
    let mut data = vec![Datum {
        entity,
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Policy")).unwrap()),
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op: Op::Assert,
    }];
    let mut fields = vec![("boundary", "action"), ("effect", effect)];
    fields.extend(paths.iter().map(|p| ("appliesTo", *p)));
    if selector {
        fields.push(("selector", "a-selector"));
    }
    for (key, value) in fields {
        data.push(Datum {
            entity,
            attribute: store.intern(&format!("{DEFAULT_BASE_NS}{key}")).unwrap(),
            value: Value::Str(value.into()),
            valid_from: "2026-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    store
        .transact(&data, "2026-01-01T00:00:00Z", None, None)
        .unwrap();
    store
}
fn trace(commit: &str, path: &str) -> TraceRecord {
    TraceRecord {
        git_commit: Some(commit.into()),
        path: Some(path.into()),
        constraints: vec![super::super::audit::Evaluation {
            id: "path-policy".into(),
            outcome: Some("unsatisfied".into()),
            response: Some("logged".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}
fn bypasses(report: &Report) -> usize {
    report
        .discrepancies
        .iter()
        .filter(|d| d.detail.starts_with("bypassed enforcement:"))
        .count()
}
#[test]
fn shell_commit_without_hook_fails_with_attribution_and_positive_control() {
    let f = Fixture::new();
    let head = f.commit("src/auth/x", "shell");
    let (report, scope) = f.check(&store("deny", &["src/auth/**"], false), &[]);
    assert!(!report.conforms());
    assert_eq!(bypasses(&report), 1);
    assert_eq!(
        (
            scope.commits_checked,
            scope.paths_checked,
            scope.policies_checked
        ),
        (1, 1, 1)
    );
    let detail = &report
        .discrepancies
        .iter()
        .find(|d| d.detail.starts_with("bypassed enforcement:"))
        .unwrap()
        .detail;
    assert!(
        detail.contains(&head)
            && detail.contains("SHANTY_AGENT: fixture")
            && detail.contains("agent@example.org")
    );
}
#[test]
fn unrelated_path_passes_but_window_is_still_measured() {
    let f = Fixture::new();
    f.commit("README.md", "ok");
    let (r, s) = f.check(&store("deny", &["src/auth/**"], false), &[]);
    assert!(r.conforms());
    assert_eq!(s.paths_checked, 1);
    assert_eq!(s.unresolved, 0);
}
#[test]
fn exact_non_deny_evaluation_covers_only_its_commit_and_path() {
    let f = Fixture::new();
    let first = f.commit("src/generated/x", "one");
    let policy = store("record", &["src/generated/**"], false);
    let (r, _) = f.check(&policy, &[trace(&first, "src/generated/x")]);
    assert!(r.conforms());
    f.commit("src/generated/x", "two");
    let (r, _) = f.check(&policy, &[trace(&first, "src/generated/x")]);
    assert_eq!(bypasses(&r), 1);
    let (r, _) = f.check(&policy, &[trace(&first, "/src/generated/x")]);
    assert_eq!(bypasses(&r), 2);
}
#[test]
fn blocked_or_allowed_trace_cannot_excuse_a_deny_crossing() {
    let f = Fixture::new();
    let head = f.commit("src/auth/x", "shell");
    let (r, _) = f.check(
        &store("deny", &["src/auth/**"], false),
        &[trace(&head, "src/auth/x")],
    );
    assert_eq!(bypasses(&r), 0);
    assert!(!r.conforms());
    assert!(
        r.discrepancies
            .iter()
            .any(|d| d.detail.starts_with("denied path committed:"))
    );
}
#[test]
fn legacy_or_unknown_evidence_does_not_cover_a_change() {
    let f = Fixture::new();
    let head = f.commit("src/generated/x", "one");
    let policy = store("record", &["src/generated/**"], false);
    let mut t = trace(&head, "src/generated/x");
    t.git_commit = None;
    assert_eq!(bypasses(&f.check(&policy, &[t]).0), 1);
    let mut t = trace(&head, "src/generated/x");
    t.constraints[0].outcome = Some("unknown".into());
    assert_eq!(bypasses(&f.check(&policy, &[t]).0), 1);
}
#[test]
fn deletion_rename_and_revert_are_not_lost_in_a_net_diff() {
    let f = Fixture::new();
    f.commit("src/auth/x", "shell");
    f.git(&["mv", "src/auth/x", "moved"]);
    f.git(&["commit", "-qm", "rename"]);
    f.git(&["rm", "moved"]);
    f.git(&["commit", "-qm", "delete"]);
    let (r, s) = f.check(&store("deny", &["src/auth/**", "moved"], false), &[]);
    assert_eq!(s.commits_checked, 3);
    assert_eq!(s.paths_checked, 4);
    assert_eq!(bypasses(&r), 4);
}
#[test]
fn newline_paths_are_one_change_and_untracked_files_are_out_of_scope() {
    let f = Fixture::new();
    f.commit("src/auth/with\nnewline", "shell");
    fs::write(f.dir.path().join("src/auth/untracked"), "not committed").unwrap();
    let (r, s) = f.check(&store("deny", &["src/auth/**"], false), &[]);
    assert_eq!(s.paths_checked, 1);
    assert_eq!(bypasses(&r), 1);
}
#[test]
fn selector_and_empty_catalogue_are_explicitly_unresolved() {
    let f = Fixture::new();
    let h = f.commit("src/auth/x", "shell");
    let (r, s) = f.check(
        &store("deny", &["src/auth/**"], true),
        &[trace(&h, "src/auth/x")],
    );
    assert_eq!(s.unresolved, 1);
    assert!(
        r.discrepancies
            .iter()
            .any(|d| d.detail.contains("selector/predicate replay"))
    );
    assert_eq!(
        f.check(&Store::open_in_memory().unwrap(), &[]).1.unresolved,
        1
    );
}
#[test]
fn malformed_glob_and_invalid_refs_are_errors_not_clean_results() {
    let f = Fixture::new();
    f.commit("README.md", "ok");
    assert!(
        reconcile(
            &store("deny", &["src/["], false),
            &[],
            f.dir.path(),
            &f.base,
            "HEAD",
            &mut Report::default()
        )
        .is_err()
    );
    assert!(
        reconcile(
            &store("deny", &["src/**"], false),
            &[],
            f.dir.path(),
            "missing-ref",
            "HEAD",
            &mut Report::default()
        )
        .is_err()
    );
}
#[test]
fn side_branch_and_merge_paths_are_both_covered() {
    let f = Fixture::new();
    let main = f.git(&["branch", "--show-current"]);
    f.git(&["checkout", "-qb", "side"]);
    f.commit("src/auth/side", "shell");
    f.git(&["checkout", &main]);
    f.commit("README.md", "ok");
    f.git(&["merge", "--no-ff", "-qm", "merge", "side"]);
    let (r, s) = f.check(&store("deny", &["src/auth/**"], false), &[]);
    assert_eq!(s.commits_checked, 3);
    assert_eq!(bypasses(&r), 2);
}

#[test]
fn shallow_history_is_refused_even_when_the_selected_refs_resolve() {
    let f = Fixture::new();
    f.commit("src/auth/x", "shell");
    let shallow = tempfile::tempdir().unwrap();
    let url = format!("file://{}", f.dir.path().display());
    let o = Command::new("git")
        .args(["clone", "--depth", "1", &url])
        .arg(shallow.path())
        .output()
        .unwrap();
    assert!(o.status.success());
    let error = reconcile(
        &store("deny", &["src/auth/**"], false),
        &[],
        shallow.path(),
        "HEAD",
        "HEAD",
        &mut Report::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("shallow repository"));
}

#[test]
fn grafted_ancestry_cannot_hide_a_shell_crossing() {
    let f = Fixture::new();
    f.commit("src/auth/x", "shell");
    let head = f.commit("README.md", "ok");
    fs::write(
        f.dir.path().join(".git/info/grafts"),
        format!("{head} {}\n", f.base),
    )
    .unwrap();
    let error = reconcile(
        &store("deny", &["src/auth/**"], false),
        &[],
        f.dir.path(),
        &f.base,
        "HEAD",
        &mut Report::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("grafts"));
}

#[test]
fn blob_reads_history_not_dirty_worktree_and_refuses_deleted_paths() {
    let f = Fixture::new();
    let commit = f.commit("src/example.rs", "// TODO missing ticket\nfn f() {}");
    fs::write(
        f.dir.path().join("src/example.rs"),
        "// TODO APP-12\nfn f() {}",
    )
    .unwrap();
    assert!(
        repository::blob(f.dir.path(), &commit, "src/example.rs")
            .unwrap()
            .contains("missing ticket")
    );
    f.git(&["rm", "-f", "src/example.rs"]);
    f.git(&["commit", "-qm", "delete"]);
    assert!(repository::blob(f.dir.path(), "HEAD", "src/example.rs").is_err());
}

#[cfg(unix)]
#[test]
fn blob_does_not_follow_a_committed_symlink() {
    let f = Fixture::new();
    std::os::unix::fs::symlink("/outside/source.rs", f.dir.path().join("link.rs")).unwrap();
    f.git(&["add", "link.rs"]);
    f.git(&["commit", "-qm", "symlink"]);
    assert!(repository::blob(f.dir.path(), "HEAD", "link.rs").is_err());
}
