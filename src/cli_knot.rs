//! File-based RDF knot command.

use oxrdfio::RdfFormat;

use crate::cli::{flag_value, resolve_timestamp};

pub fn cmd_knot(args: &[String], db_path: &str) {
    let file_path = match args.get(2) {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!(
                "usage: quipu knot <file.ttl> [--graph <iri>] [--shapes <shapes.ttl>] [--timestamp <ISO-8601>] [--db <path>]"
            );
            std::process::exit(1);
        }
    };

    let shapes_path = flag_value(args, "--shapes");

    let mut store = crate::cli_open::open_store(db_path);

    let data = match std::fs::read_to_string(file_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error reading {file_path}: {e}");
            std::process::exit(1);
        }
    };

    if let Some(sp) = shapes_path {
        let shapes = match std::fs::read_to_string(sp) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error reading shapes {sp}: {e}");
                std::process::exit(1);
            }
        };
        match quipu::validate_shapes(&shapes, &data) {
            Ok(feedback) => {
                if !feedback.conforms {
                    eprintln!(
                        "SHACL validation failed: {} violation(s)",
                        feedback.violations
                    );
                    for issue in &feedback.results {
                        eprintln!(
                            "  {} on {}: {}",
                            issue.severity,
                            issue.focus_node,
                            issue.message.as_deref().unwrap_or("constraint violated")
                        );
                    }
                    std::process::exit(1);
                }
                println!("SHACL validation passed");
            }
            Err(e) => {
                eprintln!("validation error: {e}");
                std::process::exit(1);
            }
        }
    }

    let format = match std::path::Path::new(file_path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("nt" | "ntriples") => RdfFormat::NTriples,
        Some("rdf" | "xml") => RdfFormat::RdfXml,
        _ => RdfFormat::Turtle,
    };

    let now = resolve_timestamp(args);
    let graph = match flag_value(args, "--graph") {
        Some(iri) => match store.graph_create(iri) {
            Ok(graph) => graph,
            Err(e) => {
                eprintln!("error registering graph: {e}");
                std::process::exit(1);
            }
        },
        None => 0,
    };
    let base_iri = std::fs::canonicalize(file_path)
        .ok()
        .map(|path| format!("file://{}", path.display()));
    match quipu::rdf::ingest_rdf_bitemporal_with_scope(
        &mut store,
        data.as_bytes(),
        format,
        base_iri.as_deref(),
        &now,
        &now,
        None,
        Some(file_path),
        graph,
        flag_value(args, "--blank-node-scope"),
    ) {
        Ok((tx_id, count)) => {
            println!("knotted {count} facts from {file_path} (tx {tx_id})");
        }
        Err(e) => {
            eprintln!("error ingesting: {e}");
            std::process::exit(1);
        }
    }
}
