//! `quipu ingest` — a streaming, DECLARED bulk load (aegis-j0yaxj.2).
//!
//! ## Why this is not `quipu knot`
//!
//! `knot` reads the whole file into a `String`, parses it into a `Vec<Datum>`, and
//! commits one transaction. That is exactly right at the scale it was written for
//! and impossible at 1B triples BY CONSTRUCTION rather than by slowness: it needs
//! the entire dataset resident and a single transaction of the same size.
//!
//! ## Why a DECLARATION is required rather than optional
//!
//! A chunked load is N transactions, and N transactions are not atomic the way one
//! is. If chunk 57 of 100 fails, 56 are committed and the graph reads as a smaller,
//! COMPLETE dataset. For the benchmark this verb exists to feed, that is worse than
//! an error and it fails in the flattering direction: 700M has better latency than
//! 1B, so a truncated load makes the numbers look BETTER, and a good-looking result
//! is published rather than investigated.
//!
//! So `--declare-count` and `--declare-sha256` are MANDATORY. They are made from the
//! dataset itself (`wc -l`, `sha256sum`) before the load starts, so a short load
//! cannot satisfy them by lowering the bar. On a mismatch the partial graph is left
//! in place, visibly unmarked, and the exit code is nonzero.
//!
//! ## Why `--timestamp` is mandatory too
//!
//! Two runs over one pinned dataset must produce the SAME store, or "re-derivable
//! result bundle" is unreachable. A `now()` resolved once per run still fails that,
//! so the caller supplies it and every chunk carries it.

use crate::cli::flag_value;

/// Run `quipu ingest`.
pub fn cmd_ingest(args: &[String], db_path: &str) {
    let Some(file_path) = args
        .get(2)
        .map(String::as_str)
        .filter(|p| !p.starts_with("--"))
    else {
        usage();
        std::process::exit(1);
    };

    // Every one of these is REQUIRED, and the refusal names what it is for. A
    // default here would be a second source of truth for a number whose whole job
    // is to come from outside this process.
    let Some(graph_iri) = flag_value(args, "--graph") else {
        eprintln!("error: --graph <iri> is required; a declared ingest names the window it fills");
        usage();
        std::process::exit(1);
    };
    let Some(timestamp) = flag_value(args, "--timestamp") else {
        eprintln!(
            "error: --timestamp <ISO-8601> is required and must NOT be now(): two runs over one \
             pinned dataset must produce the same store, or the result is not re-derivable"
        );
        std::process::exit(1);
    };
    let Some(declared_count) = flag_value(args, "--declare-count") else {
        eprintln!(
            "error: --declare-count <n> is required. A chunked load is N transactions and a short \
             one reads as a smaller COMPLETE dataset -- which benchmarks BETTER than the real one"
        );
        std::process::exit(1);
    };
    let Some(declared_sha) = flag_value(args, "--declare-sha256") else {
        eprintln!(
            "error: --declare-sha256 <hex> is required; run `sha256sum <file>` on the source"
        );
        std::process::exit(1);
    };

    let triples: usize = match declared_count.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: --declare-count must be a whole number, got {declared_count:?}");
            std::process::exit(1);
        }
    };
    if declared_sha.len() != 64 || !declared_sha.chars().all(|c| c.is_ascii_hexdigit()) {
        eprintln!(
            "error: --declare-sha256 must be 64 hex characters (a bare SHA-256), got {} character(s). \
             `sha256sum` prints the digest AND the filename -- pass only the digest",
            declared_sha.len()
        );
        std::process::exit(1);
    }

    let format = match flag_value(args, "--format").unwrap_or("nt") {
        "nt" | "ntriples" => oxrdfio::RdfFormat::NTriples,
        "ttl" | "turtle" => oxrdfio::RdfFormat::Turtle,
        "nq" | "nquads" => oxrdfio::RdfFormat::NQuads,
        other => {
            eprintln!("error: unknown --format {other:?}; expected nt, ttl or nq");
            std::process::exit(1);
        }
    };
    let chunk: usize = flag_value(args, "--chunk")
        .and_then(|c| c.parse().ok())
        .unwrap_or(50_000);

    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error opening {file_path}: {e}");
            std::process::exit(1);
        }
    };
    // STREAMED, never read into memory. A `read_to_string` here would reintroduce
    // the exact limit this verb exists to remove.
    let reader = std::io::BufReader::new(file);

    let mut store = crate::cli_open::open_store(db_path);
    let graph = match store.graph_create(graph_iri) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error registering graph {graph_iri}: {e}");
            std::process::exit(1);
        }
    };

    let declaration = quipu::LoadDeclaration {
        triples,
        sha256: declared_sha.to_string(),
    };
    let started = std::time::Instant::now();
    match quipu::ingest_rdf_declared(
        &mut store,
        reader,
        format,
        flag_value(args, "--base-iri"),
        timestamp,
        flag_value(args, "--actor"),
        flag_value(args, "--source"),
        graph,
        chunk,
        &declaration,
    ) {
        Ok(report) => {
            let secs = started.elapsed().as_secs_f64();
            println!("ingested {} triples into <{graph_iri}>", report.parsed);
            println!("  transactions: {}", report.tx_ids.len());
            println!("  seconds:      {secs:.2}");
            // NOT a throughput claim. `parsed` is what the PARSER produced, and quipu
            // #127 established the hard way that it is not the number written -- it
            // reported 4 writes for a re-apply that stored nothing. A rate computed
            // from it would publish the cheap half of the work as the whole.
            println!(
                "  NOTE: `triples` is the PARSE count, not a write count. For throughput, take a \
                 before/after delta of live facts (quipu #127)."
            );
        }
        Err(e) => {
            eprintln!("ingest REFUSED: {e}");
            eprintln!(
                "The partial graph is left in place and carries no completion marker, so a reader \
                 can tell an incomplete load from one that never ran."
            );
            std::process::exit(1);
        }
    }
}

fn usage() {
    eprintln!(
        "usage: quipu ingest <file> --graph <iri> --timestamp <ISO-8601> \\\n\
        \x20         --declare-count <n> --declare-sha256 <hex>\n\
        \x20         [--format nt|ttl|nq] [--chunk <n>] [--base-iri <iri>]\n\
        \x20         [--actor <a>] [--source <s>] [--db <path>]\n\
        \n\
        The declaration is made from the SOURCE before the load:\n\
        \x20   --declare-count   $(wc -l < file.nt)      (n-triples: one triple per line)\n\
        \x20   --declare-sha256  $(sha256sum file.nt | cut -d' ' -f1)\n\
        \n\
        An unmet declaration is REFUSED and the partial graph is left visibly incomplete."
    );
}
