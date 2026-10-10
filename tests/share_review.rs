//! `quipu share diff --report`, the PR-review report (aegis-fxpbys.1, M2), on
//! the fixture packs in `tests/fixtures/share-review/`. Every pack there shares
//! one shapes file (Person: `age` is at most one `xsd:integer`, `email` is
//! required) except `tightened`, which also requires `role`. `base` already
//! violates it once: bob has no email.
// The `quipu` binary has required-features = ["shacl"]; the SHACL half of the
// report is the thing under test.
#![cfg(feature = "shacl")]
use std::path::PathBuf;
use std::process::{Command, Output};

use quipu::git_merge::Decisions;
use quipu::shacl::Validator;
use quipu::share_diff::read_payload;
use quipu::share_pack_review::{Review, ReviewInput, ShaclReview, render_report_markdown, review};

fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/share-review")
        .join(name)
}

fn read(name: &str, file: &str) -> String {
    std::fs::read_to_string(dir(name).join(file)).unwrap()
}

fn run(old: &str, new: &str) -> Review {
    let decisions: Option<Decisions> = dir(new)
        .join("decisions.json")
        .is_file()
        .then(|| serde_json::from_str(&read(new, "decisions.json")).unwrap());
    let (o, n) = (
        read_payload(&dir(old)).unwrap(),
        read_payload(&dir(new)).unwrap(),
    );
    let (os, ns) = (read(old, "shapes.ttl"), read(new, "shapes.ttl"));
    review(&ReviewInput {
        old: &o,
        new: &n,
        old_shapes: Some(&os),
        new_shapes: Some(&ns),
        decisions: decisions.as_ref(),
    })
    .unwrap()
}

fn cli(old: &str, new: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args(["share", "diff"])
        .arg(dir(old))
        .arg(dir(new))
        .arg("--report")
        .args(extra)
        .output()
        .unwrap()
}

fn introduced(r: &Review) -> usize {
    r.shacl.introduced().expect("SHACL was checked")
}

/// CONTROL for the "introduced" fixture: validated directly, without the
/// report, alice's datatype violation is in `introduced` and not in `base`, and
/// bob's missing email is in both. Without this the introduced=1 assertion
/// below could pass on a fixture that never violated anything.
#[test]
fn control_introduced_fixture_really_has_the_violation_only_in_new() {
    let v = Validator::from_turtle(&read("base", "shapes.ttl")).unwrap();
    let datatype = |name: &str| {
        let f = v.validate(read(name, "payload.nq").as_bytes()).unwrap();
        let count = |c: &str| f.results.iter().filter(|i| i.component.contains(c)).count();
        (
            count("DatatypeConstraintComponent"),
            count("MinCountConstraintComponent"),
        )
    };
    assert_eq!(datatype("base"), (0, 1));
    assert_eq!(datatype("introduced"), (1, 1));
}

#[test]
fn an_introduced_violation_counts_once_and_names_the_entity() {
    let r = run("base", "introduced");
    assert_eq!(introduced(&r), 1);
    let ShaclReview::Checked {
        introduced: rows,
        preexisting,
        resolved,
        shapes_changed,
        ..
    } = &r.shacl
    else {
        unreachable!()
    };
    assert_eq!((*preexisting, *resolved, *shapes_changed), (1, 0, false));
    assert_eq!(rows[0].focus, "Alice Smith (people/alice)");
    assert!(rows[0].constraint.contains("Datatype"), "{rows:?}");
    assert!(!rows[0].from_shapes_change);
}

#[test]
fn a_preexisting_violation_is_not_introduced() {
    let r = run("base", "preexisting");
    assert_eq!(introduced(&r), 0);
    assert!(matches!(
        r.shacl,
        ShaclReview::Checked { preexisting: 1, .. }
    ));
}

#[test]
fn a_clean_change_introduces_nothing() {
    let r = run("clean-old", "clean-new");
    assert_eq!(introduced(&r), 0);
    assert!(matches!(
        r.shacl,
        ShaclReview::Checked {
            preexisting: 0,
            resolved: 0,
            ..
        }
    ));
    assert_eq!(r.diff.changed, 1);
}

#[test]
fn a_shapes_change_that_breaks_unchanged_data_is_introduced_and_attributed() {
    let r = run("base", "tightened");
    assert!(r.diff.entities.is_empty(), "data is identical");
    assert_eq!(introduced(&r), 2);
    let ShaclReview::Checked {
        introduced: rows, ..
    } = &r.shacl
    else {
        unreachable!()
    };
    assert!(rows.iter().all(|v| v.from_shapes_change), "{rows:?}");
}

#[test]
fn the_decisions_sidecar_is_rendered_with_constraint_values_and_resolution() {
    let r = run("preexisting", "decisions");
    let d = r.decisions.as_ref().unwrap();
    assert_eq!(
        (d.conflicts.len(), d.aliases.len(), d.unresolved),
        (1, 1, 1)
    );
    let md = render_report_markdown(&r);
    let line = md.lines().find(|l| l.contains("`conflict:0`")).unwrap();
    for part in [
        "Alice Smith (people/alice)",
        "**age**",
        "`sh:maxCount 1`",
        "base `\"30\"^^xsd:integer`",
        "ours `\"31\"^^xsd:integer`",
        "theirs `\"32\"^^xsd:integer`",
        "resolution **ours**",
    ] {
        assert!(line.contains(part), "{part:?} missing from {line}");
    }
    let alias = md.lines().find(|l| l.contains("`alias:0`")).unwrap();
    assert!(alias.contains("**UNRESOLVED**"), "{alias}");
}

#[test]
fn the_alias_caveat_is_always_present_even_with_no_candidates() {
    for (old, new) in [("clean-old", "clean-new"), ("base", "introduced")] {
        let r = run(old, new);
        assert!(r.alias_candidates.is_empty());
        let md = render_report_markdown(&r);
        assert!(md.contains("### Alias caveat"), "{md}");
        assert!(md.contains("cannot see two different IRIs"), "{md}");
        assert!(md.contains("No candidate pairs proposed."), "{md}");
    }
}

#[test]
fn a_near_duplicate_label_on_an_added_entity_is_proposed() {
    let r = run("clean-old", "alias");
    // carol is added too, and must NOT be proposed: her label is not close.
    assert_eq!(r.alias_candidates.len(), 1, "{:?}", r.alias_candidates);
    let c = &r.alias_candidates[0];
    assert_eq!(c.added, "Alice  Smith. (people/asmith)");
    assert_eq!(c.other, "Alice Smith (people/alice)");
    assert!(c.other_in_old && c.similarity >= 0.90);
}

#[test]
fn sections_come_in_order_and_the_last_line_is_the_summary() {
    let r = run("base", "introduced");
    let md = render_report_markdown(&r);
    let at = |h: &str| md.find(h).unwrap_or_else(|| panic!("{h} missing"));
    assert!(at("### Facts") < at("### SHACL violations introduced"));
    assert!(at("### SHACL violations introduced") < at("### Merge decisions"));
    assert!(at("### Merge decisions") < at("### Alias caveat"));
    assert!(at("### Alias caveat") < at("### Summary"));
    assert_eq!(md.lines().last().unwrap(), r.summary);
    assert!(r.summary.contains("SHACL 1 introduced"), "{}", r.summary);
}

#[test]
fn missing_shapes_is_not_checked_never_zero() {
    let o = read_payload(&dir("clean-old")).unwrap();
    let n = read_payload(&dir("clean-new")).unwrap();
    let r = review(&ReviewInput {
        old: &o,
        new: &n,
        old_shapes: None,
        new_shapes: None,
        decisions: None,
    })
    .unwrap();
    assert!(matches!(
        r.shacl,
        ShaclReview::NotChecked {
            gate_must_fail: true,
            ..
        }
    ));
    let md = render_report_markdown(&r);
    assert!(md.contains("**NOT CHECKED**"), "{md}");
    assert!(!md.contains("0 introduced"), "{md}");
    assert!(r.summary.contains("SHACL NOT CHECKED"), "{}", r.summary);
}

#[test]
fn cli_gate_exits_3_only_on_introduced_violations() {
    let code = |old, new| {
        cli(old, new, &["--fail-on-introduced"])
            .status
            .code()
            .unwrap()
    };
    assert_eq!(code("base", "introduced"), 3);
    assert_eq!(code("base", "preexisting"), 0);
    assert_eq!(code("clean-old", "clean-new"), 0);
    assert_eq!(code("clean-old", "alias"), 0);
    assert_eq!(code("preexisting", "decisions"), 0);
    // Without the flag the report is informational: exit 0, full report.
    let o = cli("base", "introduced", &[]);
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.trim_end().ends_with("alias candidates"), "{out}");
}

#[test]
fn cli_report_json_carries_the_summary_and_counts() {
    let o = cli("base", "introduced", &["--format", "json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["shacl"]["status"], "checked");
    assert_eq!(v["shacl"]["introduced_count"], 1);
    assert!(v["summary"].as_str().unwrap().starts_with("qpack review:"));
}

#[test]
fn cli_report_only_flags_require_report() {
    let o = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args(["share", "diff"])
        .arg(dir("base"))
        .arg(dir("introduced"))
        .arg("--fail-on-introduced")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args([
            "-c",
            "user.name=Review test",
            "-c",
            "user.email=test@example.org",
            "-C",
        ])
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

/// The CI entrypoint on real commits: base = a pack with a pre-existing
/// violation; three PR heads. Only the head that introduces a violation is red,
/// and a fixture-like directory without a manifest is not reviewed at all.
#[test]
fn ci_script_is_red_only_for_an_introduced_violation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    git(repo, &["init", "-q", "-b", "main"]);
    let pack = repo.join("data/qpack");
    std::fs::create_dir_all(&pack).unwrap();
    let put = |fixture: &str| {
        std::fs::write(pack.join("export.nt"), read(fixture, "payload.nq")).unwrap();
        std::fs::write(pack.join("shapes.ttl"), read(fixture, "shapes.ttl")).unwrap();
    };
    put("base");
    std::fs::write(pack.join("manifest.json"), "{}\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "base"]);
    let base = git(repo, &["rev-parse", "HEAD"]);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/ci/qpack-review.py");
    let head = |branch: &str, change: &dyn Fn()| {
        git(repo, &["checkout", "-q", "-b", branch, &base]);
        change();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", branch]);
        let sha = git(repo, &["rev-parse", "HEAD"]);
        let summary = repo.join(format!("{branch}.md"));
        let o = Command::new("python3")
            .arg(&script)
            .args([
                "--base",
                &base,
                "--head",
                &sha,
                "--binary",
                env!("CARGO_BIN_EXE_quipu"),
            ])
            .arg("--repo")
            .arg(repo)
            .arg("--summary")
            .arg(&summary)
            .output()
            .unwrap();
        let md = std::fs::read_to_string(&summary).unwrap();
        std::fs::remove_file(&summary).unwrap();
        (o.status.code().unwrap(), md)
    };
    let (code, md) = head("introduces", &|| put("introduced"));
    assert_eq!(code, 1, "{md}");
    assert!(
        md.contains("## `data/qpack`") && md.contains("SHACL 1 introduced"),
        "{md}"
    );
    let (code, md) = head("preexisting", &|| put("preexisting"));
    assert_eq!(code, 0, "{md}");
    assert!(
        md.contains("SHACL 0 introduced") && md.contains("**Green:** 1 pack"),
        "{md}"
    );
    let (code, md) = head("shapes-deleted", &|| {
        std::fs::remove_file(pack.join("shapes.ttl")).unwrap();
    });
    assert_eq!(code, 1, "{md}");
    assert!(!md.contains("**Green:**"), "{md}");
    let (code, md) = head("unshaped-new-pack", &|| {
        let new_pack = repo.join("data/unshaped");
        std::fs::create_dir_all(&new_pack).unwrap();
        std::fs::write(new_pack.join("export.nt"), read("clean-new", "payload.nq")).unwrap();
        std::fs::write(new_pack.join("manifest.json"), "{}\n").unwrap();
    });
    assert_eq!(code, 1, "{md}");
    assert!(!md.contains("**Green:**"), "{md}");
    let (code, md) = head("shapes-malformed", &|| {
        std::fs::write(pack.join("shapes.ttl"), "not Turtle {\n").unwrap();
    });
    assert_eq!(code, 1, "{md}");
    let (code, md) = head("partial-deletion", &|| {
        std::fs::remove_file(pack.join("export.nt")).unwrap();
    });
    assert_eq!(code, 1, "{md}");
    let (code, md) = head("whole-deletion", &|| {
        std::fs::remove_dir_all(&pack).unwrap();
    });
    assert_eq!(code, 0, "{md}");
    assert!(
        md.contains("Deleted pack") && md.contains("not applicable"),
        "{md}"
    );
    assert!(!md.contains("SHACL 0 introduced"), "{md}");
    let (code, md) = head("fixture-only", &|| {
        let d = repo.join("tests/fixtures/x");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("export.nt"), read("introduced", "payload.nq")).unwrap();
    });
    assert_eq!(code, 0, "{md}");
    assert!(md.contains("No qpack changed"), "{md}");
}

#[test]
fn cli_missing_or_malformed_shapes_refuses_gate_but_report_is_informational() {
    let temp = tempfile::tempdir().unwrap();
    let new = temp.path().join("new");
    std::fs::create_dir(&new).unwrap();
    std::fs::write(new.join("payload.nq"), read("clean-new", "payload.nq")).unwrap();
    std::fs::write(new.join("manifest.json"), "{}\n").unwrap();
    let probe = |gated: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_quipu"));
        command
            .args(["share", "diff"])
            .arg(dir("clean-old"))
            .arg(&new)
            .arg("--report");
        if gated {
            command.arg("--fail-on-introduced");
        }
        command.output().unwrap()
    };
    assert!(probe(false).status.success());
    let missing = probe(true);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stdout).contains("NOT CHECKED"));
    std::fs::write(new.join("shapes.ttl"), "not Turtle {\n").unwrap();
    assert_eq!(probe(true).status.code(), Some(1));
}
