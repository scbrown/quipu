//! `quipu share diff` and the `git diff` textconv on a fixture pack pair
//! (aegis-fxpbys.1). The pair holds exactly four edits: one functional value
//! change (alice's age), one added entity (carol), one removed fact (alice's
//! nickname), and one blank node that is ONLY relabelled (alice's address,
//! `_:b0` -> `_:c14n7`, same content).
// The `quipu` binary has required-features = ["shacl"].
#![cfg(feature = "shacl")]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use quipu::share_diff::{EntityStatus, Snapshot, diff, read_payload, render_text};

fn fixture(side: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/share-diff")
        .join(side)
}

fn snapshot(side: &str) -> Snapshot {
    Snapshot::new(&read_payload(&fixture(side)).unwrap())
}

fn quipu(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// CONTROL: the fixture really exercises the problem. A raw line diff of the
/// same pair shows full IRIs, splits the value change into a -/+ pair, and
/// reports the relabelled-only blank node as six changed lines: 11 lines for
/// what is semantically 4 facts.
#[test]
fn control_raw_line_diff_of_the_fixture_is_ugly() {
    let lines = |side: &str| -> std::collections::BTreeSet<String> {
        std::fs::read_to_string(fixture(side).join("export.nt"))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    };
    let (old, new) = (lines("old"), lines("new"));
    let removed: Vec<_> = old.difference(&new).collect();
    let added: Vec<_> = new.difference(&old).collect();
    assert!(removed.iter().any(|l| l.contains("\"30\"")));
    assert!(added.iter().any(|l| l.contains("\"31\"")));
    let blank = |set: &Vec<&String>| set.iter().filter(|l| l.contains("_:")).count();
    assert_eq!((blank(&removed), blank(&added)), (3, 3));
    assert!(
        removed
            .iter()
            .chain(&added)
            .all(|l| l.starts_with("<http://") || l.starts_with("_:"))
    );
    assert_eq!((removed.len(), added.len()), (5, 6));
}

#[test]
fn value_change_is_one_line_and_relabel_is_zero() {
    let d = diff(&snapshot("old"), &snapshot("new"));
    let text = render_text(&d);
    assert_eq!((d.changed, d.added, d.removed), (1, 2, 1), "{text}");
    let age: Vec<_> = text.lines().filter(|l| l.contains("age")).collect();
    assert_eq!(
        age,
        ["  ~ age: \"30\"^^xsd:integer -> \"31\"^^xsd:integer"],
        "{text}"
    );
    // The relabelled-only address produces no line at all.
    assert!(
        !text.contains("address") && !text.contains("Paris"),
        "{text}"
    );
    assert!(!text.contains("_:"), "{text}");
}

#[test]
fn entities_are_shown_by_label_not_bare_iri() {
    let d = diff(&snapshot("old"), &snapshot("new"));
    let text = render_text(&d);
    assert!(text.contains("~ Alice (people/alice)"), "{text}");
    assert!(text.contains("+ Carol (people/carol)"), "{text}");
    assert!(text.contains("  - nickname: \"Al\""), "{text}");
    assert!(!text.contains("http://"), "{text}");
    let carol = d
        .entities
        .iter()
        .find(|e| e.name.starts_with("Carol"))
        .unwrap();
    assert_eq!(carol.status, EntityStatus::Added);
    assert!(d.entities.iter().all(|e| !e.name.starts_with("Bob")));
}

#[test]
fn cli_share_diff_formats() {
    let (old, new) = (fixture("old"), fixture("new"));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let args = |f: &'static str| {
        vec![
            "share".to_string(),
            "diff".into(),
            old.to_string_lossy().into(),
            new.to_string_lossy().into(),
            "--format".into(),
            f.into(),
        ]
    };
    let run = |f| {
        let a = args(f);
        stdout(&quipu(
            dir,
            &a.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
    };
    let text = run("text");
    assert!(text.contains("  ~ age: \"30\"^^xsd:integer -> \"31\"^^xsd:integer"));
    assert!(
        text.ends_with("2 entities: 1 changed, 2 added, 1 removed facts\n"),
        "{text}"
    );
    assert!(run("markdown").contains("### + Carol (people/carol)"));
    let json: serde_json::Value = serde_json::from_str(&run("json")).unwrap();
    assert_eq!(json["changed"], 1);
    // A single payload file works as well as a pack directory.
    let file = fixture("new").join("export.nt");
    let same = quipu(
        dir,
        &[
            "share",
            "diff",
            file.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
    );
    assert_eq!(stdout(&same), "no semantic changes\n");
}

#[test]
fn textconv_is_identical_across_a_blank_node_relabel() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.nt");
    let b = dir.path().join("b.nt");
    std::fs::write(
        &a,
        "<http://e/s> <http://e/p> _:x .\n_:x <http://e/q> \"v\" .\n",
    )
    .unwrap();
    std::fs::write(
        &b,
        "<http://e/s> <http://e/p> _:c14n0 .\n_:c14n0 <http://e/q> \"v\" .\n",
    )
    .unwrap();
    let conv = |p: &Path| stdout(&quipu(dir.path(), &["diff-textconv", p.to_str().unwrap()]));
    assert_eq!(conv(&a), conv(&b));
    assert_eq!(conv(&a), "e/s\n  p: [ q \"v\" ]\n\n");
    // Unparseable input passes through so `git diff` never fails on it.
    std::fs::write(&a, "<<<<<<< ours\n").unwrap();
    assert_eq!(conv(&a), "<<<<<<< ours\n");
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args([
            "-c",
            "user.name=Diff test",
            "-c",
            "user.email=test@example.org",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    stdout(&o)
}

/// Real git: `.gitattributes` + `diff.quipu.textconv` turn the fixture's diff
/// into grouped, labelled lines; the relabelled blank node contributes none.
#[test]
fn git_diff_uses_the_textconv() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(
        p.join(".gitattributes"),
        "*.nt diff=quipu\n*.nq diff=quipu\n",
    )
    .unwrap();
    let textconv = format!("'{}' diff-textconv", env!("CARGO_BIN_EXE_quipu"));
    git(p, &["config", "diff.quipu.textconv", &textconv]);
    std::fs::copy(fixture("old").join("export.nt"), p.join("export.nt")).unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "old"]);
    std::fs::copy(fixture("new").join("export.nt"), p.join("export.nt")).unwrap();
    let out = git(p, &["diff", "--no-color", "-U0", "--", "export.nt"]);
    let changes: Vec<&str> = out
        .lines()
        .filter(|l| {
            (l.starts_with('+') || l.starts_with('-'))
                && !l.starts_with("+++")
                && !l.starts_with("---")
        })
        .collect();
    assert_eq!(
        changes,
        [
            "-  age: \"30\"^^xsd:integer",
            "+  age: \"31\"^^xsd:integer",
            "-  nickname: \"Al\"",
            "+Carol (people/carol)",
            "+  rdfs:label: \"Carol\"",
            "+  role: \"designer\"",
            "+",
        ],
        "{out}"
    );
    // CONTROL: the same change without the textconv is raw N-Triples.
    let raw = git(
        p,
        &["diff", "--no-color", "--no-textconv", "--", "export.nt"],
    );
    assert!(
        raw.contains("_:b0") && raw.contains("<http://example.org/people/alice>"),
        "{raw}"
    );
}
