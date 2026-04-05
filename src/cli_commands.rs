//! Additional CLI commands: shapes, validate, export, episode, retract, repl, stats.

use std::io::{self, BufRead, Read, Write};

use oxrdfio::RdfFormat;

use crate::cli::{chrono_now, format_value};

pub fn cmd_episode(args: &[String], db_path: &str) {
    let file_arg = match args.get(2) {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!("usage: quipu episode <file.json> [--db <path>]");
            eprintln!("  use - to read from stdin");
            std::process::exit(1);
        }
    };

    let json_str = if file_arg == "-" {
        let mut buf = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut buf)
            .unwrap_or_else(|e| {
                eprintln!("error reading stdin: {e}");
                std::process::exit(1);
            });
        buf
    } else {
        match std::fs::read_to_string(file_arg) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error reading {file_arg}: {e}");
                std::process::exit(1);
            }
        }
    };

    let episode: quipu::Episode = match serde_json::from_str(&json_str) {
        Ok(ep) => ep,
        Err(e) => {
            eprintln!("error parsing episode JSON: {e}");
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

    let now = chrono_now();
    match quipu::ingest_episode(
        &mut store,
        &episode,
        &now,
        quipu::namespace::DEFAULT_BASE_NS,
    ) {
        Ok((tx_id, count)) => {
            println!(
                "ingested episode \"{}\" -- {count} facts (tx {tx_id})",
                episode.name
            );
        }
        Err(e) => {
            eprintln!("error ingesting episode: {e}");
            std::process::exit(1);
        }
    }
}

pub fn cmd_retract(args: &[String], db_path: &str) {
    let entity_iri = match args.get(2) {
        Some(iri) if !iri.starts_with("--") => iri.as_str(),
        _ => {
            eprintln!("usage: quipu retract <entity-IRI> [--predicate <IRI>] [--db <path>]");
            std::process::exit(1);
        }
    };

    let predicate_iri = args
        .windows(2)
        .find(|w| w[0] == "--predicate")
        .map(|w| w[1].as_str());

    let mut store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let mut input = serde_json::json!({
        "entity": entity_iri,
        "timestamp": chrono_now(),
    });
    if let Some(pred) = predicate_iri {
        input["predicate"] = serde_json::json!(pred);
    }

    match quipu::tool_retract(&mut store, &input) {
        Ok(result) => {
            let count = result["retracted"].as_u64().unwrap_or(0);
            if count == 0 {
                println!("no facts found for {entity_iri}");
            } else {
                println!(
                    "retracted {count} fact(s) from {entity_iri} (tx {})",
                    result["tx_id"]
                );
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub fn cmd_shapes(args: &[String], db_path: &str) {
    let action = args.get(2).map_or("list", std::string::String::as_str);

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    match action {
        "load" => {
            let name = match args.get(3) {
                Some(n) if !n.starts_with("--") => n.as_str(),
                _ => {
                    eprintln!("usage: quipu shapes load <name> <file.ttl> [--db <path>]");
                    std::process::exit(1);
                }
            };
            let file_path = match args.get(4) {
                Some(p) if !p.starts_with("--") => p.as_str(),
                _ => {
                    eprintln!("usage: quipu shapes load <name> <file.ttl> [--db <path>]");
                    std::process::exit(1);
                }
            };
            let turtle = match std::fs::read_to_string(file_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error reading {file_path}: {e}");
                    std::process::exit(1);
                }
            };
            let input = serde_json::json!({
                "action": "load",
                "name": name,
                "turtle": turtle,
                "timestamp": chrono_now(),
            });
            match quipu::tool_shapes(&store, &input) {
                Ok(_) => println!("loaded shape graph \"{name}\" from {file_path}"),
                Err(e) => {
                    eprintln!("error loading shapes: {e}");
                    std::process::exit(1);
                }
            }
        }
        "remove" => {
            let name = match args.get(3) {
                Some(n) if !n.starts_with("--") => n.as_str(),
                _ => {
                    eprintln!("usage: quipu shapes remove <name> [--db <path>]");
                    std::process::exit(1);
                }
            };
            let input = serde_json::json!({ "action": "remove", "name": name });
            match quipu::tool_shapes(&store, &input) {
                Ok(result) => {
                    if result["found"].as_bool().unwrap_or(false) {
                        println!("removed shape graph \"{name}\"");
                    } else {
                        println!("shape graph \"{name}\" not found");
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            let input = serde_json::json!({ "action": "list" });
            match quipu::tool_shapes(&store, &input) {
                Ok(result) => {
                    let shapes = result["shapes"].as_array().unwrap();
                    if shapes.is_empty() {
                        println!("no shapes loaded");
                    } else {
                        for shape in shapes {
                            let name = shape["name"].as_str().unwrap_or("?");
                            let loaded = shape["loaded_at"].as_str().unwrap_or("?");
                            println!("  {name} (loaded: {loaded})");
                        }
                        println!("\n{} shape graph(s)", shapes.len());
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

pub fn cmd_validate(args: &[String]) {
    let shapes_path = args
        .windows(2)
        .find(|w| w[0] == "--shapes")
        .map(|w| w[1].as_str());
    let data_path = args
        .windows(2)
        .find(|w| w[0] == "--data")
        .map(|w| w[1].as_str());

    let (Some(shapes_path), Some(data_path)) = (shapes_path, data_path) else {
        eprintln!("usage: quipu validate --shapes <shapes.ttl> --data <data.ttl>");
        std::process::exit(1);
    };

    let shapes = match std::fs::read_to_string(shapes_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading shapes: {e}");
            std::process::exit(1);
        }
    };
    let data = match std::fs::read_to_string(data_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading data: {e}");
            std::process::exit(1);
        }
    };

    match quipu::validate_shapes(&shapes, &data) {
        Ok(feedback) => {
            if feedback.conforms {
                println!("valid ({} warnings)", feedback.warnings);
            } else {
                println!(
                    "invalid: {} violation(s), {} warning(s)",
                    feedback.violations, feedback.warnings
                );
                for issue in &feedback.results {
                    println!(
                        "  [{:>9}] {} -- {}{}",
                        issue.severity,
                        issue.focus_node,
                        issue.message.as_deref().unwrap_or("constraint violated"),
                        issue
                            .path
                            .as_ref()
                            .map(|p| format!(" (path: {p})"))
                            .unwrap_or_default()
                    );
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("validation error: {e}");
            std::process::exit(1);
        }
    }
}

pub fn cmd_repl(db_path: &str) {
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
            break;
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

pub fn cmd_export(args: &[String], db_path: &str) {
    let format = args
        .windows(2)
        .find(|w| w[0] == "--format")
        .map_or("ntriples", |w| w[1].as_str());

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

pub fn cmd_stats(db_path: &str) {
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
            println!("facts (current): {}", result.rows().len());

            let mut entities = Vec::new();
            let mut predicates = Vec::new();
            for row in result.rows() {
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

#[cfg(feature = "lancedb")]
pub fn cmd_migrate_vectors(args: &[String], config: &quipu::QuipuConfig) {
    let from = args
        .windows(2)
        .find(|w| w[0] == "--from")
        .map_or("sqlite", |w| w[1].as_str());
    let to = args
        .windows(2)
        .find(|w| w[0] == "--to")
        .map_or("lancedb", |w| w[1].as_str());
    let dry_run = args.iter().any(|a| a == "--dry-run");

    if from != "sqlite" || to != "lancedb" {
        eprintln!(
            "usage: quipu migrate-vectors --from sqlite --to lancedb [--dry-run] [--db <path>]"
        );
        std::process::exit(1);
    }

    let db_path = config.store_path.to_string_lossy();
    let store = match quipu::Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let lance_path = config.vector.lancedb_path.to_string_lossy().to_string();
    match quipu::migrate_sqlite_to_lancedb(&store, &lance_path, dry_run, 1000) {
        Ok(result) => {
            if dry_run {
                println!(
                    "dry run: {} vector(s) would be migrated, {} skipped",
                    result.migrated, result.skipped
                );
            } else {
                println!(
                    "migrated {} vector(s) to LanceDB ({} skipped)",
                    result.migrated, result.skipped
                );
                if result.migrated > 0 || result.skipped == 0 {
                    println!("  LanceDB path: {lance_path}");
                    println!("  Set vector.backend = \"lancedb\" in .bobbin/config.toml to use it");
                }
            }
        }
        Err(e) => {
            eprintln!("migration error: {e}");
            std::process::exit(1);
        }
    }
}

fn run_query(store: &quipu::Store, sparql: &str) {
    run_query_temporal(store, sparql, &quipu::TemporalContext::default());
}

fn run_query_temporal(store: &quipu::Store, sparql: &str, ctx: &quipu::TemporalContext) {
    match quipu::sparql_query_temporal(store, sparql, ctx) {
        Ok(result) => match result {
            quipu::QueryResult::Select { variables, rows } => {
                println!("{}", variables.join("\t"));
                println!("{}", "-".repeat(variables.len() * 20));
                for row in &rows {
                    let cols: Vec<String> = variables
                        .iter()
                        .map(|v| match row.get(v) {
                            Some(val) => format_value(store, val),
                            None => "(unbound)".to_string(),
                        })
                        .collect();
                    println!("{}", cols.join("\t"));
                }
                println!("\n{} results", rows.len());
            }
            quipu::QueryResult::Ask(result) => {
                println!("{result}");
            }
            quipu::QueryResult::Graph(triples) => {
                for t in &triples {
                    let obj_str = format_value(store, &t.object);
                    println!("{}\t{}\t{}", t.subject, t.predicate, obj_str);
                }
                println!("\n{} triples", triples.len());
            }
        },
        Err(e) => {
            eprintln!("query error: {e}");
        }
    }
}
