//! The committed project graph under `.quipu/` (aegis-w3k75d.11), end to end.
//!
//! The done-when, as a test: in a FRESH CLONE, one quipu command loads
//! `.quipu/graph` and a SPARQL count matches the committed export, and
//! `git add .quipu` after quipu has run stages no key and no database.
//!
//! The producer shares OUTWARD (the default), with an identifier catalogue
//! loaded, because a committed graph may sit in a public repository. So the
//! consumer's plain `quipu load`, with no flags, is the path under test.
#![cfg(feature = "shacl")]

use std::path::Path;
use std::process::{Command, Output};

const GRAPH: &str = "urn:quipu:project:demo";

fn quipu(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Seed the producer store: three facts in the project graph, one in ROOT
/// (which must NOT travel), and the identifier catalogue an outward share needs.
fn seed_producer(db: &Path) {
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let mut store = quipu::Store::open(db.to_str().unwrap()).unwrap();
    let g = store.graph_create(GRAPH).unwrap();
    quipu::rdf::ingest_rdf_to_graph(
        &mut store,
        &b"<urn:ex:a> <urn:ex:p> <urn:ex:b> .\n\
           <urn:ex:b> <urn:ex:p> <urn:ex:c> .\n\
           <urn:ex:c> <urn:ex:q> \"lit\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-30T00:00:00Z",
        None,
        Some("fixture"),
        g,
    )
    .unwrap();
    quipu::ingest_rdf(
        &mut store,
        &b"<urn:ex:root> <urn:ex:p> \"not project data\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-30T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let catalogue = store.overlay_create("urn:test:catalogue", 0).unwrap();
    quipu::rdf::ingest_rdf_to_graph(
        &mut store,
        include_bytes!("fixtures/share-catalogue.ttl").as_slice(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-01T00:00:00Z",
        None,
        Some("test-catalogue"),
        catalogue,
    )
    .unwrap();
}

fn count(dir: &Path, query: &str) -> u64 {
    let out = ok(&quipu(dir, &["query", query, "--db", ".quipu/local.db"]));
    out.lines()
        .filter_map(|l| l.trim().parse::<u64>().ok())
        .next()
        .unwrap_or_else(|| panic!("no count in:\n{out}"))
}

#[test]
fn a_fresh_clone_loads_the_committed_project_graph_in_one_command() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    // The producer's store lives where a developer's would, INSIDE .quipu/.
    seed_producer(&repo.join(".quipu/local.db"));

    ok(&quipu(
        &repo,
        &[
            "share",
            "--project",
            "demo",
            "--no-shapes",
            "--db",
            ".quipu/local.db",
        ],
    ));
    // A private signing key beside it, as the server or `share --attest` writes.
    std::fs::write(repo.join(".quipu/verifier.pk8"), b"PRIVATE").unwrap();

    git(&repo, &["add", ".quipu"]);
    let staged = git(&repo, &["diff", "--cached", "--name-only"]);
    let mut staged: Vec<&str> = staged.lines().collect();
    staged.sort_unstable();
    assert_eq!(
        staged,
        [
            ".quipu/.gitignore",
            ".quipu/graph/export.nt",
            ".quipu/graph/manifest.json",
            ".quipu/graph/manifest.ttl",
            ".quipu/graph/shapes.ttl",
            ".quipu/project",
        ],
        "only the project graph may be staged; never local.db* or verifier.pk8"
    );
    assert!(repo.join(".quipu/local.db").exists() && repo.join(".quipu/verifier.pk8").exists());
    git(&repo, &["commit", "-qm", "project graph"]);

    let clone = tmp.path().join("clone");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            repo.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert!(
        !clone.join(".quipu/local.db").exists(),
        "a clone starts with no store"
    );

    // THE one command. No flags: an outward bundle needs no catalogue to load.
    let loaded = ok(&quipu(
        &clone,
        &["load", ".quipu/graph", "--db", ".quipu/local.db"],
    ));
    assert!(loaded.contains("\"added\": 3"), "{loaded}");

    let exported = std::fs::read_to_string(clone.join(".quipu/graph/export.nt"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u64;
    let in_graph = count(
        &clone,
        &format!("SELECT (COUNT(*) AS ?n) WHERE {{ GRAPH <{GRAPH}> {{ ?s ?p ?o }} }}"),
    );
    assert_eq!(
        in_graph, exported,
        "the loaded graph must match the committed export"
    );
    assert_eq!(exported, 3);
    // Nothing reached ROOT: the producer's ROOT fact never travelled, and the
    // load promoted into the project graph, not the default graph.
    assert_eq!(
        count(&clone, "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }"),
        0
    );

    // A re-load after a pull that changed nothing is a no-op.
    let again = ok(&quipu(
        &clone,
        &["load", ".quipu/graph", "--db", ".quipu/local.db"],
    ));
    assert!(
        again.contains("\"added\": 0") && again.contains("\"tx_id\": null"),
        "{again}"
    );

    // And quipu having run in the clone does not dirty it.
    assert_eq!(git(&clone, &["status", "--porcelain"]), "");
}

#[test]
fn a_repository_cannot_be_silently_re_pointed_at_another_project() {
    let tmp = tempfile::tempdir().unwrap();
    seed_producer(&tmp.path().join(".quipu/local.db"));
    ok(&quipu(
        tmp.path(),
        &[
            "share",
            "--project",
            "demo",
            "--no-shapes",
            "--db",
            ".quipu/local.db",
        ],
    ));
    let out = quipu(
        tmp.path(),
        &[
            "share",
            "--project",
            "other",
            "--no-shapes",
            "--db",
            ".quipu/local.db",
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusing to re-point"));
    // The committed id is in force without naming it again, and re-sharing
    // REPLACES the committed bundle (its normal life), instead of refusing an
    // existing destination the way a plain `--output` does.
    ok(&quipu(
        tmp.path(),
        &[
            "share",
            "--project",
            "--no-shapes",
            "--db",
            ".quipu/local.db",
        ],
    ));
    assert!(tmp.path().join(".quipu/graph/manifest.json").is_file());
    assert!(!tmp.path().join(".quipu/graph.next").exists());
    assert!(!tmp.path().join(".quipu/graph.prev").exists());
}

#[test]
fn load_refuses_a_bundle_that_is_not_a_single_graph() {
    let tmp = tempfile::tempdir().unwrap();
    seed_producer(&tmp.path().join(".quipu/local.db"));
    // A ROOT share: `load` must not guess a graph for it.
    ok(&quipu(
        tmp.path(),
        &[
            "share",
            "--output",
            "rootshare",
            "--no-shapes",
            "--db",
            ".quipu/local.db",
        ],
    ));
    let out = quipu(tmp.path(), &["load", "rootshare", "--db", "other.db"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("single-graph"),
        "{out:?}"
    );
}
