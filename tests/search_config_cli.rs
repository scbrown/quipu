//! `[quipu.search]` must reach a CLI query, not only the server.
//!
//! It was parsed into `QuipuConfig` and then dropped on the floor for the
//! `quipu` binary: `server.rs` does `store.search_config_mut().clone_from(&config.search)`
//! and `cli_open.rs` did not, so every `quipu query` ran on the built-in
//! defaults whatever the config file said.
//!
//! That is worse than an ordinary ignored setting, because the query budget's
//! own refusal names this exact key as the remedy -- "raise `[quipu.search]`
//! `query_timeout_ms`" -- so the one instruction an operator receives when a
//! query is cut short could not be carried out from the CLI. Measured against
//! a real multi-million-fact store (aegis-j0yaxj.2): 30,000 ms regardless of
//! the configured value, from the working directory and from the store's
//! directory alike.
//!
//! The timeout is asserted in the direction that CANNOT pass by accident: a
//! tiny budget must make a query REFUSE. Asserting a large budget lets a query
//! finish would also pass with the setting still ignored, since the default is
//! already larger than any query in a test fixture.

use std::process::Command;

/// Build a store with enough rows that an unconstrained scan is not instant.
fn seeded_store(dir: &std::path::Path) -> String {
    let db = dir.join("store.db");
    let path = db.to_str().unwrap().to_string();
    let mut store = quipu::Store::open(&path).unwrap();
    let mut turtle = String::from("@prefix ex: <http://example.test/> .\n");
    for index in 0..2000 {
        turtle.push_str(&format!("ex:s{index} ex:p \"value {index}\" .\n"));
    }
    quipu::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-09-13T00:00:00Z",
        None,
        Some("search-config-cli-test"),
    )
    .unwrap();
    path
}

fn run_query(cwd: &std::path::Path, db: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(cwd)
        .args(["query", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--db", db])
        .output()
        .unwrap()
}

#[test]
fn a_configured_query_timeout_reaches_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_store(dir.path());

    // CONTROL: with no config file the query completes on the default budget.
    // Without this, a refusal below could be any unrelated failure.
    let control = run_query(dir.path(), &db);
    let control_err = String::from_utf8_lossy(&control.stderr);
    assert!(
        !control_err.contains("query timeout"),
        "control query should not time out on the default budget: {control_err}"
    );

    // A 1 ms budget must make the same query on the same store refuse. If the
    // setting is ignored -- the bug -- this query succeeds exactly like the
    // control and the test fails.
    std::fs::create_dir_all(dir.path().join(".bobbin")).unwrap();
    std::fs::write(
        dir.path().join(".bobbin/config.toml"),
        "[quipu.search]\nquery_timeout_ms = 1\n",
    )
    .unwrap();

    let limited = run_query(dir.path(), &db);
    let limited_err = String::from_utf8_lossy(&limited.stderr);
    assert!(
        limited_err.contains("query timeout"),
        "configured query_timeout_ms was ignored by the CLI; stderr was: {limited_err}"
    );
}
