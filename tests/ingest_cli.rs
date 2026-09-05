//! End-to-end contract for `quipu ingest` — the declared, streaming bulk load
//! (aegis-j0yaxj.2).
//!
//! GATED ON `shacl` BECAUSE THE BINARY IS. `[[bin]] quipu` carries
//! `required-features = ["shacl"]`, so on the `--no-default-features` CI leg the
//! binary is never built and `CARGO_BIN_EXE_quipu` points at a path that does not
//! exist. Ungated, all five arms fail identically with `Os { code: 2, NotFound }` —
//! which reads as "the verb is broken" and is really "the binary was not built on
//! this leg". Same gate as `tests/aegis_ontology_shapes.rs` and `tests/share_cli.rs`
//! (malcolm, on quipu #161).
//!
//! These drive the BINARY, not the library. The library functions
//! (`ingest_rdf_chunked`, `ingest_rdf_declared`) already have their own
//! mutation-checked unit tests; what is untested until here is everything the CLI
//! adds on top — the mandatory flags, the argument validation, the exit codes, and
//! whether the refusal actually leaves the store in the state the message claims.
//!
//! The exit CODE is part of the contract and not decoration. A benchmark harness
//! reads it, and a refusal that exits 0 would let a truncated load into a published
//! number — which is the whole thing this verb exists to prevent.
#![cfg(feature = "shacl")]

use std::process::{Command, Output};

const TS: &str = "2026-09-05T00:00:00Z";

fn nt(n: usize) -> String {
    (0..n)
        .map(|i| format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::new().chain_update(bytes).finalize())
}

fn ingest(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .arg("ingest")
        .args(args)
        .output()
        .expect("running quipu ingest")
}

fn read(db: &str, sparql: &str) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .args(["read", sparql, "--db", db])
        .output()
        .expect("running quipu read");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A met declaration loads, and the graph SAYS it is complete.
///
/// The marker query is the one a reader would actually run, and it is scoped
/// `GRAPH <iri>` on purpose: my first attempt asked it unscoped, got zero rows, and
/// the markers were there all along. A retrievability check has to ask the way the
/// data is held (aegis-9fgh / the ontology read-back rule).
#[test]
fn a_met_declaration_loads_and_marks_the_graph_complete() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("d.nt");
    let db = root.path().join("a.db");
    let body = nt(1000);
    std::fs::write(&src, &body).unwrap();

    let out = ingest(&[
        src.to_str().unwrap(),
        "--graph",
        "http://ex/g1",
        "--timestamp",
        TS,
        "--declare-count",
        "1000",
        "--declare-sha256",
        &sha256_hex(body.as_bytes()),
        "--chunk",
        "250",
        "--db",
        db.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ingested 1000 triples"), "{stdout}");
    assert!(
        stdout.contains("transactions: 4"),
        "1000 at chunk 250 is 4 txs: {stdout}"
    );
    // The parse-count caveat rides WITH the number, because a harness reading this
    // output is exactly who would otherwise publish it as throughput (quipu #127).
    assert!(stdout.contains("not a write count"), "{stdout}");

    let markers = read(
        db.to_str().unwrap(),
        "SELECT ?p WHERE { GRAPH <http://ex/g1> { <http://ex/g1> ?p ?o } }",
    );
    for m in ["declaredTriples", "sourceSha256", "complete"] {
        assert!(markers.contains(m), "missing marker {m} in:\n{markers}");
    }
}

/// THE ONE THAT MATTERS. A short load is refused, exits NONZERO, and leaves the
/// partial graph present and unmarked — all three, because any one of them alone
/// would let a truncated dataset into a published benchmark.
#[test]
fn a_short_load_exits_nonzero_and_leaves_a_visible_unmarked_graph() {
    let root = tempfile::tempdir().unwrap();
    let full = nt(1000);
    let short: String = nt(1000).lines().take(600).collect::<Vec<_>>().join("\n") + "\n";
    let src = root.path().join("short.nt");
    let db = root.path().join("e.db");
    std::fs::write(&src, &short).unwrap();

    let out = ingest(&[
        src.to_str().unwrap(),
        "--graph",
        "http://ex/g3",
        "--timestamp",
        TS,
        // Declared from the FULL dataset; the file on disk is truncated.
        "--declare-count",
        "1000",
        "--declare-sha256",
        &sha256_hex(full.as_bytes()),
        "--chunk",
        "250",
        "--db",
        db.to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "a short load must not exit 0");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("declaration unmet"), "{err}");
    assert!(err.contains("600"), "the refusal must say how short: {err}");

    let count = read(
        db.to_str().unwrap(),
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH <http://ex/g3> { ?s ?p ?o } }",
    );
    assert!(
        count.contains("600"),
        "the partial graph vanished:\n{count}"
    );
    let markers = read(
        db.to_str().unwrap(),
        "SELECT ?p WHERE { GRAPH <http://ex/g3> { <http://ex/g3> ?p ?o } }",
    );
    assert!(
        markers.contains("0 results"),
        "a refused load marked itself complete:\n{markers}"
    );
}

/// Two runs over one pinned dataset must produce the SAME store, or "re-derivable
/// result bundle" is unreachable. This is what makes `--timestamp` mandatory rather
/// than defaulted to `now()`.
#[test]
fn two_runs_of_one_pinned_dataset_agree() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("d.nt");
    let body = nt(300);
    std::fs::write(&src, &body).unwrap();
    let sha = sha256_hex(body.as_bytes());

    let mut stamps: Vec<String> = Vec::new();
    for name in ["one.db", "two.db"] {
        let db = root.path().join(name);
        let out = ingest(&[
            src.to_str().unwrap(),
            "--graph",
            "http://ex/g",
            "--timestamp",
            TS,
            "--declare-count",
            "300",
            "--declare-sha256",
            &sha,
            "--chunk",
            "97",
            "--db",
            db.to_str().unwrap(),
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Counted through the CLI, which is also how a reader would check it.
        stamps.push(read(
            db.to_str().unwrap(),
            "SELECT (COUNT(*) AS ?n) WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }",
        ));
        // One ingest is one logical event: every chunk carries the SUPPLIED stamp.
        let asserted = read(
            db.to_str().unwrap(),
            "SELECT ?o WHERE { GRAPH <http://ex/g> { <http://ex/g> <urn:quipu:ingest:declaredTriples> ?o } }",
        );
        assert!(
            asserted.contains("300"),
            "the completion marker is wrong:\n{asserted}"
        );
    }
    assert_eq!(
        stamps[0], stamps[1],
        "two runs of one pinned dataset produced different stores"
    );
}

/// Every declaration flag is MANDATORY, and each refusal must SAY WHY rather than
/// print a bare usage line. A default for any of these would be a second source of
/// truth for a number whose entire job is to come from outside this process.
#[test]
fn each_missing_declaration_flag_refuses_with_its_own_reason() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("d.nt");
    std::fs::write(&src, nt(3)).unwrap();
    let db = root.path().join("x.db");
    let s = src.to_str().unwrap();
    let d = db.to_str().unwrap();
    let sha = "a".repeat(64);

    let cases: [(&[&str], &str); 4] = [
        (
            &[
                "--timestamp",
                TS,
                "--declare-count",
                "3",
                "--declare-sha256",
                &sha,
            ],
            "--graph",
        ),
        (
            &[
                "--graph",
                "http://ex/g",
                "--declare-count",
                "3",
                "--declare-sha256",
                &sha,
            ],
            "re-derivable",
        ),
        (
            &[
                "--graph",
                "http://ex/g",
                "--timestamp",
                TS,
                "--declare-sha256",
                &sha,
            ],
            "--declare-count",
        ),
        (
            &[
                "--graph",
                "http://ex/g",
                "--timestamp",
                TS,
                "--declare-count",
                "3",
            ],
            "--declare-sha256",
        ),
    ];
    for (args, expect) in cases {
        let mut full = vec![s];
        full.extend_from_slice(args);
        full.extend_from_slice(&["--db", d]);
        let out = ingest(&full);
        assert!(!out.status.success(), "missing flag exited 0: {args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains(expect),
            "refusal for {args:?} did not mention {expect}:\n{err}"
        );
    }
}

/// `sha256sum` prints the digest AND the filename. Pasting the whole line is the
/// mistake a person actually makes, so the refusal names it rather than reporting a
/// mismatch after doing the entire load.
#[test]
fn a_sha256sum_line_pasted_whole_is_refused_before_the_load() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("d.nt");
    std::fs::write(&src, nt(3)).unwrap();
    let db = root.path().join("y.db");

    let out = ingest(&[
        src.to_str().unwrap(),
        "--graph",
        "http://ex/g",
        "--timestamp",
        TS,
        "--declare-count",
        "3",
        "--declare-sha256",
        &format!("{}  d.nt", "a".repeat(64)),
        "--db",
        db.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("64 hex"), "{err}");
    assert!(
        err.contains("filename"),
        "the refusal must name the actual mistake:\n{err}"
    );
    // Refused BEFORE touching the store: nothing was loaded on the way to finding out.
    assert!(
        !db.exists(),
        "a malformed flag opened the store before refusing"
    );
}
