//! Quipu CLI — interactive knowledge graph demo.
//!
//! Commands:
//!   quipu load <file.ttl> [--db <path>]  Load a Turtle file into the store
//!   quipu query <sparql> [--db <path>]   Run a SPARQL SELECT query
//!   quipu repl [--db <path>]             Interactive SPARQL prompt
//!   quipu export [--format ntriples|turtle] [--db <path>]  Export facts
//!   quipu stats [--db <path>]            Show store statistics

use std::io::{self, BufRead, Write};

use oxrdfio::RdfFormat;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return;
    }

    // Parse --db flag from anywhere in args.
    let db_path = args
        .windows(2)
        .find(|w| w[0] == "--db")
        .map(|w| w[1].as_str())
        .unwrap_or("quipu.db");

    let cmd = args[1].as_str();
    match cmd {
        "load" => cmd_load(&args, db_path),
        "query" => cmd_query(&args, db_path),
        "repl" => cmd_repl(db_path),
        "export" => cmd_export(&args, db_path),
        "stats" => cmd_stats(db_path),
        "--help" | "-h" | "help" => print_usage(),
        _ => {
            eprintln!("unknown command: {cmd}");
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!(
        "quipu — AI-native knowledge graph

USAGE:
    quipu load <file.ttl> [--db <path>]     Load Turtle/N-Triples into store
    quipu query \"<sparql>\" [--db <path>]     Run a SPARQL SELECT query
    quipu repl [--db <path>]                 Interactive SPARQL prompt
    quipu export [--format ntriples] [--db <path>]  Export current facts
    quipu stats [--db <path>]                Show store statistics

OPTIONS:
    --db <path>    Store file (default: quipu.db)
    --format <fmt> Export format: ntriples, turtle (default: ntriples)"
    );
}

fn cmd_load(args: &[String], db_path: &str) {
    let file_path = args.get(2);

    let file_path = match file_path {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!("usage: quipu load <file.ttl> [--db <path>]");
            std::process::exit(1);
        }
    };

    let mut store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let data = match std::fs::read_to_string(file_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error reading {file_path}: {e}");
            std::process::exit(1);
        }
    };

    let format = if file_path.ends_with(".nt") || file_path.ends_with(".ntriples") {
        RdfFormat::NTriples
    } else {
        RdfFormat::Turtle
    };

    let now = chrono_now();
    match quipu::ingest_rdf(&mut store, data.as_bytes(), format, None, &now, None, Some(file_path))
    {
        Ok((tx_id, count)) => {
            println!("loaded {count} triples from {file_path} (tx {tx_id})");
        }
        Err(e) => {
            eprintln!("error ingesting: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_query(args: &[String], db_path: &str) {
    let sparql = match args.get(2) {
        Some(q) if !q.starts_with("--") => q,
        _ => {
            eprintln!("usage: quipu query \"SELECT ...\" [--db <path>]");
            std::process::exit(1);
        }
    };

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    run_query(&store, sparql);
}

fn cmd_repl(db_path: &str) {
    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    println!("quipu SPARQL repl (db: {db_path})");
    println!("type a SPARQL query, or :quit to exit\n");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("sparql> ");
        stdout.flush().unwrap();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap() == 0 {
            break; // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == ":quit" || trimmed == ":q" {
            break;
        }

        run_query(&store, trimmed);
        println!();
    }
}

fn cmd_export(args: &[String], db_path: &str) {
    let format = args
        .windows(2)
        .find(|w| w[0] == "--format")
        .map(|w| w[1].as_str())
        .unwrap_or("ntriples");

    let rdf_format = match format {
        "ntriples" | "nt" => RdfFormat::NTriples,
        "turtle" | "ttl" => RdfFormat::Turtle,
        _ => {
            eprintln!("unknown format: {format} (try: ntriples, turtle)");
            std::process::exit(1);
        }
    };

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    match quipu::export_rdf(&store, rdf_format) {
        Ok(bytes) => {
            io::stdout().write_all(&bytes).unwrap();
        }
        Err(e) => {
            eprintln!("error exporting: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_stats(db_path: &str) {
    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    match quipu::sparql_query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }") {
        Ok(result) => {
            println!("store: {db_path}");
            println!("facts (current): {}", result.rows.len());

            // Count distinct entities and predicates.
            let mut entities = Vec::new();
            let mut predicates = Vec::new();
            for row in &result.rows {
                if let Some(s) = row.get("s")
                    && !entities.contains(s)
                {
                    entities.push(s.clone());
                }
                if let Some(p) = row.get("p")
                    && !predicates.contains(p)
                {
                    predicates.push(p.clone());
                }
            }
            println!("entities: {}", entities.len());
            println!("predicates: {}", predicates.len());
        }
        Err(e) => {
            eprintln!("error querying stats: {e}");
            std::process::exit(1);
        }
    }
}

fn run_query(store: &quipu::Store, sparql: &str) {
    match quipu::sparql_query(store, sparql) {
        Ok(result) => {
            // Print header.
            println!("{}", result.variables.join("\t"));
            println!("{}", "-".repeat(result.variables.len() * 20));

            // Print rows.
            for row in &result.rows {
                let cols: Vec<String> = result
                    .variables
                    .iter()
                    .map(|v| match row.get(v) {
                        Some(val) => format_value(store, val),
                        None => "(unbound)".to_string(),
                    })
                    .collect();
                println!("{}", cols.join("\t"));
            }
            println!("\n{} results", result.rows.len());
        }
        Err(e) => {
            eprintln!("query error: {e}");
        }
    }
}

fn format_value(store: &quipu::Store, val: &quipu::Value) -> String {
    match val {
        quipu::Value::Ref(id) => store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}")),
        quipu::Value::Str(s) => format!("\"{s}\""),
        quipu::Value::Int(n) => n.to_string(),
        quipu::Value::Float(f) => f.to_string(),
        quipu::Value::Bool(b) => b.to_string(),
        quipu::Value::Bytes(b) => format!("<{} bytes>", b.len()),
    }
}

/// Simple ISO-8601 timestamp without pulling in chrono.
fn chrono_now() -> String {
    // Use a fixed format that's good enough for the fact log.
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Convert to rough ISO date (not perfect, but functional for demo).
    let days = epoch / 86400;
    let years = 1970 + days / 365;
    let remaining_days = days % 365;
    let months = remaining_days / 30 + 1;
    let day = remaining_days % 30 + 1;
    let secs_today = epoch % 86400;
    let hours = secs_today / 3600;
    let mins = (secs_today % 3600) / 60;
    let secs = secs_today % 60;
    format!("{years:04}-{months:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
}
