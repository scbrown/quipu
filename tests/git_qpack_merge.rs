//! Real Git branches exercise the driver and the driver-absent CI gate.
#![cfg(feature = "shacl")]
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const NS: &str = "http://neuralamplifier.local/ontology/smac/";
fn git(p: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args([
            "-c",
            "user.name=Merge test",
            "-c",
            "user.email=test@example.org",
            "-C",
        ])
        .arg(p)
        .args(args)
        .output()
        .unwrap()
}
fn g(p: &Path, args: &[&str]) -> String {
    let o = git(p, args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn cli(p: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(p)
        .args(args)
        .output()
        .unwrap()
}
fn hash(b: &[u8]) -> String {
    format!(
        "sha256:{}",
        hex::encode(ring::digest::digest(&ring::digest::SHA256, b).as_ref())
    )
}
fn rehash(p: &Path) {
    let graph =
        quipu::share::canonicalize_ntriples(&std::fs::read(p.join("qpack/export.nt")).unwrap())
            .unwrap();
    std::fs::write(p.join("qpack/export.nt"), &graph).unwrap();
    let file = p.join("qpack/manifest.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    m["graph_hash"] = hash(&graph).into();
    m["shapes_hash"] = hash(&std::fs::read(p.join("qpack/shapes.ttl")).unwrap()).into();
    m.as_object_mut().unwrap().remove("share_id");
    m.as_object_mut().unwrap().remove("attestation");
    m["share_id"] = hash(&serde_json::to_vec(&m).unwrap()).into();
    std::fs::write(file, serde_json::to_vec(&m).unwrap()).unwrap();
}
struct Fixture {
    temp: TempDir,
    base: String,
    ours: String,
    theirs: String,
}
impl Fixture {
    fn path(&self) -> &Path {
        self.temp.path()
    }
    fn new(kind: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let p = temp.path();
        g(p, &["init", "-b", "base"]);
        g(p, &["config", "user.name", "Merge test"]);
        g(p, &["config", "user.email", "test@example.org"]);
        std::fs::create_dir(p.join("qpack")).unwrap();
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/book/src/datalinks/qpack");
        for f in ["manifest.json", "shapes.ttl", "export.nt"] {
            std::fs::copy(source.join(f), p.join("qpack").join(f)).unwrap();
        }
        let mut shapes = std::fs::read_to_string(p.join("qpack/shapes.ttl")).unwrap();
        shapes.push_str(&format!("\n<urn:test:effect> a sh:NodeShape ; sh:targetClass <{NS}Ability> ; sh:property [ sh:path <{NS}effectText> ; sh:maxCount 1 ] .\n"));
        std::fs::write(p.join("qpack/shapes.ttl"), shapes).unwrap();
        std::fs::write(p.join(".gitattributes"),"qpack/export.nt merge=quipu\nqpack/shapes.ttl merge=quipu\nqpack/manifest.json merge=quipu\n").unwrap();
        rehash(p);
        g(p, &["add", "."]);
        g(p, &["commit", "-m", "base"]);
        let base = g(p, &["rev-parse", "HEAD"]);
        let mut commits = Vec::new();
        for side in ["ours", "theirs"] {
            g(p, &["checkout", "-b", side, &base]);
            let path = p.join("qpack/export.nt");
            let graph = std::fs::read_to_string(&path).unwrap();
            let changed = if kind == "functional" {
                graph
                    .lines()
                    .map(|l| {
                        if l.starts_with(&format!("<{NS}ability/aaa-tracking> <{NS}effectText>")) {
                            format!("<{NS}ability/aaa-tracking> <{NS}effectText> \"{side}\" .\n")
                        } else {
                            format!("{l}\n")
                        }
                    })
                    .collect::<String>()
            } else {
                let iri = if side == "ours" { "aaa-new" } else { "zzz-new" };
                let label = if kind == "alias" {
                    "Shared ability"
                } else if side == "ours" {
                    "Quantum kite"
                } else {
                    "Velvet moon"
                };
                format!(
                    "{graph}<{NS}ability/{iri}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{NS}Ability> .\n<{NS}ability/{iri}> <http://www.w3.org/2000/01/rdf-schema#label> \"{label}\" .\n"
                )
            };
            std::fs::write(path, changed).unwrap();
            rehash(p);
            g(p, &["add", "."]);
            g(p, &["commit", "-m", side]);
            commits.push(g(p, &["rev-parse", "HEAD"]));
        }
        g(p, &["checkout", "ours"]);
        Self {
            temp,
            base,
            ours: commits[0].clone(),
            theirs: commits[1].clone(),
        }
    }
    fn merge(&self) -> Output {
        cli(self.path(), &["git-merge", &self.theirs])
    }
    fn resolve(&self, key: &str, choice: &str) -> Output {
        cli(
            self.path(),
            &[
                "qpack-resolve",
                &self.base,
                &self.ours,
                &self.theirs,
                "qpack",
                key,
                choice,
            ],
        )
    }
    fn commit(&self) {
        g(self.path(), &["add", "."]);
        g(self.path(), &["commit", "-m", "resolved"]);
    }
    fn check(&self) -> Output {
        cli(
            self.path(),
            &["qpack-check", &self.base, &self.ours, &self.theirs, "HEAD"],
        )
    }
    fn graph(&self) -> String {
        std::fs::read_to_string(self.path().join("qpack/export.nt")).unwrap()
    }
    fn decisions(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.path().join("qpack/decisions.json")).unwrap())
            .unwrap()
    }
}

#[test]
fn functional_driver_holds_base_then_resolves_and_ci_verifies() {
    let f = Fixture::new("functional");
    let o = f.merge();
    assert_eq!(
        o.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!f.graph().contains("<<<<<<<"));
    assert!(f.graph().contains("x2 vs. air attacks"));
    assert_eq!(f.decisions()["conflicts"].as_array().unwrap().len(), 1);
    assert!(f.resolve("conflict:0", "ours").status.success());
    f.commit();
    let o = f.check();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let m: Value =
        serde_json::from_slice(&std::fs::read(f.path().join("qpack/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(m["merge_parents"].as_array().unwrap().len(), 2);
    assert_eq!(m["parent_share"], m["merge_parents"][0]);
}
#[test]
fn alias_is_proposed_never_automatically_applied() {
    let f = Fixture::new("alias");
    assert_eq!(f.merge().status.code(), Some(2));
    assert_eq!(f.decisions()["aliases"].as_array().unwrap().len(), 1);
    assert!(!f.graph().contains("owl#sameAs"));
    assert!(f.resolve("alias:0", "accept").status.success());
    assert!(f.graph().contains("owl#sameAs"));
    f.commit();
    assert!(f.check().status.success());
}
#[test]
fn rejected_alias_is_auditable_and_passes() {
    let f = Fixture::new("alias");
    f.merge();
    assert!(f.resolve("alias:0", "reject").status.success());
    f.commit();
    assert!(f.check().status.success());
}
#[test]
fn disjoint_changes_merge_and_manifest_verifies() {
    let f = Fixture::new("disjoint");
    let o = f.merge();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(f.graph().contains("Quantum kite") && f.graph().contains("Velvet moon"));
    f.commit();
    assert!(f.check().status.success());
}
#[test]
fn ci_rejects_driver_absent_alias_even_with_rehashed_manifest() {
    let f = Fixture::new("alias");
    // Stock Git: no driver installed. Manifest text-conflicts; graph merges cleanly.
    git(f.path(), &["merge", "--no-commit", "--no-ff", &f.theirs]);
    assert!(!f.graph().contains("<<<<<<<"));
    assert_eq!(f.graph().matches("Shared ability").count(), 2);
    let m = g(
        f.path(),
        &["show", &format!("{}:qpack/manifest.json", f.ours)],
    );
    std::fs::write(f.path().join("qpack/manifest.json"), m).unwrap();
    rehash(f.path());
    f.commit();
    let o = f.check();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("unresolved conflicts/alias proposals"));
}
#[test]
fn ci_rejects_silent_double_even_with_forged_resolution() {
    let f = Fixture::new("functional");
    f.merge();
    f.resolve("conflict:0", "ours");
    let path = f.path().join("qpack/export.nt");
    std::fs::write(
        &path,
        format!(
            "{}<{NS}ability/aaa-tracking> <{NS}effectText> \"theirs\" .\n",
            f.graph()
        ),
    )
    .unwrap();
    rehash(f.path());
    f.commit();
    assert!(!f.check().status.success());
}
#[test]
fn stale_decision_and_unknown_choice_are_refused() {
    let f = Fixture::new("functional");
    f.merge();
    assert!(!f.resolve("conflict:0", "union").status.success());
    let mut d = f.decisions();
    d["inputs"][0] = json!("forged");
    std::fs::write(
        f.path().join("qpack/decisions.json"),
        serde_json::to_vec(&d).unwrap(),
    )
    .unwrap();
    assert!(!f.resolve("conflict:0", "ours").status.success());
}
#[test]
fn driver_requires_snapshot_context() {
    let f = Fixture::new("functional");
    let o = cli(
        f.path(),
        &["merge-driver", "base", "ours", "theirs", "qpack/export.nt"],
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("snapshot context"));
}
#[test]
fn overlapping_shapes_refuse_before_git_changes_the_tree() {
    let f = Fixture::new("disjoint");
    for (side, n) in [("ours", "2"), ("theirs", "3")] {
        g(f.path(), &["checkout", side]);
        let path = f.path().join("qpack/shapes.ttl");
        let s = std::fs::read_to_string(&path)
            .unwrap()
            .replace("sh:maxCount 1", &format!("sh:maxCount {n}"));
        std::fs::write(path, s).unwrap();
        rehash(f.path());
        g(f.path(), &["add", "."]);
        g(f.path(), &["commit", "-m", "shape"]);
    }
    g(f.path(), &["checkout", "ours"]);
    let before = g(f.path(), &["rev-parse", "HEAD"]);
    let o = cli(f.path(), &["git-merge", "theirs"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("overlapping shapes"));
    assert_eq!(before, g(f.path(), &["rev-parse", "HEAD"]));
    assert!(g(f.path(), &["status", "--porcelain"]).is_empty());
}
#[test]
fn single_branch_invalid_shacl_is_caught_without_driver() {
    let f = Fixture::new("disjoint");
    g(f.path(), &["checkout", "ours"]);
    let path = f.path().join("qpack/export.nt");
    std::fs::write(
        &path,
        format!(
            "{}<{NS}ability/aaa-tracking> <{NS}effectText> \"extra\" .\n",
            f.graph()
        ),
    )
    .unwrap();
    rehash(f.path());
    g(f.path(), &["add", "."]);
    g(f.path(), &["commit", "-m", "invalid"]);
    let o = cli(f.path(), &["qpack-check", &f.base, &f.base, "HEAD", "HEAD"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("SHACL"));
}

#[test]
fn ci_entrypoint_replays_historical_merges_and_has_a_positive_control() {
    for kind in ["alias", "disjoint"] {
        let f = Fixture::new(kind);
        if kind == "alias" {
            git(f.path(), &["merge", "--no-commit", "--no-ff", &f.theirs]);
            let m = g(
                f.path(),
                &["show", &format!("{}:qpack/manifest.json", f.ours)],
            );
            std::fs::write(f.path().join("qpack/manifest.json"), m).unwrap();
            rehash(f.path());
        } else {
            assert!(f.merge().status.success());
        }
        f.commit();
        let event = f.temp.path().join("event.json");
        std::fs::write(
            &event,
            serde_json::to_vec(&json!({"before": f.theirs})).unwrap(),
        )
        .unwrap();
        let output = Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/ci/qpack-merge-check.py"))
            .args(["--binary", env!("CARGO_BIN_EXE_quipu")])
            .env("GITHUB_EVENT_NAME", "push")
            .env("GITHUB_EVENT_PATH", &event)
            .current_dir(f.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            kind == "disjoint",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if kind == "alias" {
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("unresolved conflicts/alias proposals")
            );
        }
    }
}

#[test]
fn driver_binds_temporary_inputs_to_context_before_writing() {
    let f = Fixture::new("functional");
    let base = f.path().join("base.nt");
    let ours = f.path().join("ours.nt");
    let theirs = f.path().join("theirs.nt");
    for path in [&base, &ours, &theirs] {
        std::fs::write(path, "forged input").unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .arg("merge-driver")
        .args([&base, &ours, &theirs])
        .arg("qpack/export.nt")
        .env("QUIPU_QPACK_BASE", &f.base)
        .env("QUIPU_QPACK_OURS", &f.ours)
        .env("QUIPU_QPACK_THEIRS", &f.theirs)
        .current_dir(f.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not match immutable snapshot"));
    assert_eq!(std::fs::read_to_string(ours).unwrap(), "forged input");
}

#[test]
fn derived_manifest_and_lineage_tampering_are_detected() {
    let f = Fixture::new("disjoint");
    assert!(f.merge().status.success());
    f.commit();
    let derived = f.path().join("qpack/manifest.ttl");
    std::fs::write(derived, "# stale but syntactically valid Turtle\n").unwrap();
    f.commit();
    let out = f.check();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("stale derived manifest.ttl"));
}

#[test]
fn repository_qpack_is_a_positive_control() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        quipu::git_merge::check(repo, "HEAD", "HEAD", "HEAD", "HEAD").unwrap(),
        1
    );
}
