//! `quipu graph <import|freeze|thaw|list>` — the graph-registry CLI.
//!
//! `import` predates this module (quipu #85, moved here from `cli_commands`
//! for the file-size ratchet); `freeze`/`thaw`/`list` are the deep-freeze
//! surface (`docs/design/graph-kinds-and-deep-freeze.md`).

use crate::cli::{chrono_now, flag_value};

pub fn cmd_graph(args: &[String], db_path: &str) {
    match args.get(2).map(String::as_str) {
        Some("import") => cmd_import(args, db_path),
        Some("freeze") => cmd_freeze(args, db_path),
        Some("thaw") => cmd_thaw(args, db_path),
        Some("list") => cmd_list(args, db_path),
        _ => {
            eprintln!(
                "usage: quipu graph import <db> --as <iri> [--db <path>]\n       \
                 quipu graph freeze <iri> [--out <dir>] [--actor <who>] [--db <path>]\n       \
                 quipu graph thaw <iri> [--actor <who>] [--db <path>]\n       \
                 quipu graph list [--kind <token>] [--frozen] [--db <path>]"
            );
            std::process::exit(1);
        }
    }
}

fn cmd_import(args: &[String], db_path: &str) {
    let Some(source) = args.get(3).filter(|s| !s.starts_with("--")) else {
        eprintln!("quipu graph import requires a source database");
        std::process::exit(1);
    };
    let Some(graph) = flag_value(args, "--as") else {
        eprintln!("quipu graph import requires --as <iri>");
        std::process::exit(1);
    };
    match quipu::store::import::import_graph(
        std::path::Path::new(db_path),
        std::path::Path::new(source),
        graph,
    ) {
        Ok(r) => println!(
            "imported {source} as {graph}\n  graph id: {}\n  terms: {}\n  transactions: {}\n  facts: {}\n  vectors: {}",
            r.graph, r.terms, r.transactions, r.facts, r.vectors
        ),
        Err(e) => {
            eprintln!("graph import error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_freeze(args: &[String], db_path: &str) {
    let Some(iri) = args.get(3).filter(|s| !s.starts_with("--")) else {
        eprintln!("usage: quipu graph freeze <iri> [--out <dir>] [--actor <who>] [--db <path>]");
        std::process::exit(1);
    };
    let out_dir = flag_value(args, "--out").map_or_else(
        || {
            std::path::Path::new(db_path)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_string_lossy()
                .to_string()
        },
        std::string::ToString::to_string,
    );
    let mut store = open_store(db_path);
    match store.freeze_graph(iri, &out_dir, &chrono_now(), flag_value(args, "--actor")) {
        Ok(r) => {
            println!(
                "froze {iri}\n  pack: {}\n  alias: {}\n  hash: {}\n  facts: {} (full history)\n  transactions: {}\n  vectors: {}\n  note: as_of_tx time travel is refused while archives are attached; \
                 valid-time queries and thaw remain available",
                r.path, r.alias, r.content_hash, r.facts, r.transactions, r.vectors
            );
            // Printed on stderr, not folded into the counts above: an
            // archive that could not carry its embeddings is incomplete in a
            // way the operator has to see.
            if let Some(why) = &r.vectors_omitted {
                eprintln!("warning: no entity embeddings were carried into the archive: {why}");
            }
        }
        Err(e) => {
            eprintln!("freeze error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_thaw(args: &[String], db_path: &str) {
    let Some(iri) = args.get(3).filter(|s| !s.starts_with("--")) else {
        eprintln!("usage: quipu graph thaw <iri> [--actor <who>] [--db <path>]");
        std::process::exit(1);
    };
    let mut store = open_store(db_path);
    match store.thaw_graph(iri, &chrono_now(), flag_value(args, "--actor")) {
        Ok((facts, vectors)) => println!(
            "thawed {iri}\n  facts restored: {facts}\n  vectors restored: {vectors}\n  pack file kept on disk"
        ),
        Err(e) => {
            eprintln!("thaw error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_list(args: &[String], db_path: &str) {
    let store = open_store(db_path);
    let lifecycle = args.iter().any(|a| a == "--frozen").then_some("frozen");
    match store.list_graphs(flag_value(args, "--kind"), lifecycle) {
        Ok(graphs) => {
            for g in graphs {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    g.iri,
                    g.class,
                    g.kind.as_deref().unwrap_or("-"),
                    g.lifecycle.as_deref().unwrap_or("-"),
                    g.source.as_deref().unwrap_or("local"),
                );
            }
        }
        Err(e) => {
            eprintln!("graph list error: {e}");
            std::process::exit(1);
        }
    }
}

fn open_store(db_path: &str) -> quipu::Store {
    crate::cli_open::open_store(db_path)
}
