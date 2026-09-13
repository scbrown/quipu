//! A query that did not answer must not exit 0.
//!
//! `quipu query` printed `query error: query timeout: exceeded 30000ms` and
//! exited **0**, so a timed-out query was indistinguishable from a successful
//! empty result to every caller that branches on `$?`. An empty result is the
//! input to every "is this absent?" question we ask, so a benchmark, a detector
//! or a patrol sweep would record a confident zero (aegis-41rc28).
//!
//! Two non-zero codes, because the caller's remedy differs:
//!
//! * **2** -- the query did not COMPLETE (timeout, join-complexity refusal).
//!   Narrow it or widen the budget; retrying the same text can legitimately
//!   succeed.
//! * **1** -- the query was REFUSED or FAILED (malformed SPARQL, store error).
//!   Retrying the same text changes nothing.
//!
//! The CONTROL matters as much as the arms: without it, a binary that exited
//! non-zero on *every* query would satisfy both failure assertions and look
//! fixed.

// `[[bin]] quipu` is `required-features = ["shacl"]`, so under
// `--no-default-features` the binary is NOT BUILT and `CARGO_BIN_EXE_quipu`
// names a path that does not exist -- the tests would panic with `NotFound`
// rather than failing an assertion. Same gate as `tests/pack_cli.rs`, and the
// same defect that turned main red from #236 (aegis-z29mwm).
#![cfg(feature = "shacl")]

use std::process::Command;

/// Enough rows that an unbounded scan is not instant, so a 1 ms budget bites.
fn seeded_store(dir: &std::path::Path) -> String {
    let db = dir.join("store.db");
    let path = db.to_str().unwrap().to_string();
    let mut store = quipu::Store::open(&path).unwrap();
    let mut turtle = String::from("@prefix ex: <http://example.test/> .\n");
    for index in 0..3000 {
        turtle.push_str(&format!("ex:s{index} ex:p \"value {index}\" .\n"));
    }
    quipu::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-09-13T00:00:00Z",
        None,
        Some("query-exit-codes-test"),
    )
    .unwrap();
    path
}

fn write_config(dir: &std::path::Path, body: &str) {
    std::fs::create_dir_all(dir.join(".bobbin")).unwrap();
    std::fs::write(dir.join(".bobbin/config.toml"), body).unwrap();
}

/// Returns the exit code, plus what the process said, so a failure in an
/// environment the author cannot reach reports the evidence rather than just
/// the number. A bare `left: 0 right: 2` is not enough to diagnose from a CI
/// log.
fn query_run(cwd: &std::path::Path, db: &str, sparql: &str) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(cwd)
        .args(["query", sparql, "--db", db])
        .output()
        .unwrap();
    let code = out
        .status
        .code()
        .expect("quipu was killed by a signal rather than exiting");
    let detail = format!(
        "exit={code}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (code, detail)
}

#[test]
fn a_successful_query_still_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_store(dir.path());
    let (code, detail) = query_run(dir.path(), &db, "SELECT ?s WHERE { ?s ?p ?o } LIMIT 2");
    assert_eq!(
        code, 0,
        "the control failed: a working query must still exit 0, or the \
         assertions below prove nothing.\n{detail}"
    );
}

#[test]
fn a_malformed_query_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_store(dir.path());
    let (code, detail) = query_run(dir.path(), &db, "SELECT ?s WHERE { this is not sparql");
    assert_eq!(
        code, 1,
        "a query that cannot be parsed must exit 1.\n{detail}"
    );
}

#[test]
fn a_timed_out_query_exits_two_not_zero() {
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_store(dir.path());
    write_config(dir.path(), "[quipu.search]\nquery_timeout_ms = 1\n");
    let (code, detail) = query_run(dir.path(), &db, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }");
    assert_eq!(
        code, 2,
        "a timed-out query exited 0, so it is indistinguishable from a \
         successful empty result (aegis-41rc28).\n{detail}"
    );
}

#[test]
fn a_join_complexity_refusal_exits_two_not_zero() {
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_store(dir.path());
    write_config(dir.path(), "[quipu.search]\nmax_join_rows = 10\n");
    let (code, detail) = query_run(
        dir.path(),
        &db,
        "SELECT ?a ?b WHERE { ?a ?p ?o . ?b ?q ?r }",
    );
    assert_eq!(
        code, 2,
        "a join-complexity refusal is also a query that did not complete.\n{detail}"
    );
}
