//! Quipu CLI — AI-native knowledge graph.
//!
//! Commands:
//!   quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]  Assert facts (with optional SHACL)
//!   quipu read "<sparql>" [--db <path>]   Run a SPARQL SELECT query
//!   quipu cord [--type <IRI>] [--limit N] [--db <path>]  List entities
//!   quipu unravel [--tx N] [--valid-at <date>] [--db <path>]  Time-travel query
//!   quipu validate --shapes <shapes.ttl> --data <data.ttl>  Validate without writing
//!   quipu episode <file.json> [--db <path>]  Ingest a structured episode (or - for stdin)
//!   quipu repl [--db <path>]             Interactive SPARQL prompt
//!   quipu export [--format ntriples|turtle] [--db <path>]  Export facts
//!   quipu stats [--db <path>]            Show store statistics
//!
//! Aliases: load=knot, query=read

use std::io::{self, BufRead, Read, Write};

use oxrdfio::RdfFormat;
use serde_json;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return;
    }

    // Parse --db flag from anywhere in args (overrides config file).
    let db_flag = args
        .windows(2)
        .find(|w| w[0] == "--db")
        .map(|w| w[1].as_str());

    // Load config from .bobbin/config.toml, then apply CLI overrides.
    let config = quipu::QuipuConfig::load(std::path::Path::new("."))
        .with_db_override(db_flag);
    let db_path_buf = config.store_path.to_string_lossy().to_string();
    let db_path: &str = &db_path_buf;

    let cmd = args[1].as_str();
    match cmd {
        "knot" | "load" => cmd_knot(&args, db_path),
        "read" | "query" => cmd_query(&args, db_path),
        "cord" => cmd_cord(&args, db_path),
        "unravel" => cmd_unravel(&args, db_path),
        "episode" => cmd_episode(&args, db_path),
        "retract" => cmd_retract(&args, db_path),
        "shapes" => cmd_shapes(&args, db_path),
        "validate" => cmd_validate(&args),
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

COMMANDS:
    quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]
        Assert facts from a Turtle/N-Triples file (with optional SHACL validation)

    quipu read \"<sparql>\" [--db <path>]
        Run a SPARQL SELECT query

    quipu cord [--type <IRI>] [--limit N] [--db <path>]
        List entities, optionally filtered by rdf:type

    quipu unravel [--tx N] [--valid-at <date>] [--db <path>]
        Time-travel query: see facts as they were at a given point

    quipu episode <file.json> [--db <path>]
        Ingest a structured episode (nodes + edges) from JSON (use - for stdin)

    quipu retract <entity-IRI> [--predicate <IRI>] [--db <path>]
        Retract all facts for an entity (or just those with a given predicate)

    quipu shapes load <name> <file.ttl> [--db <path>]
        Load SHACL shapes for auto-validation on writes

    quipu shapes list [--db <path>]
        List loaded shape graphs

    quipu shapes remove <name> [--db <path>]
        Remove a loaded shape graph

    quipu validate --shapes <shapes.ttl> --data <data.ttl>
        Validate data against SHACL shapes (dry run, no write)

    quipu repl [--db <path>]
        Interactive SPARQL prompt

    quipu export [--format ntriples|turtle] [--db <path>]
        Export current facts as RDF

    quipu stats [--db <path>]
        Show store statistics

OPTIONS:
    --db <path>    Store file (default: quipu.db)

ALIASES:
    load = knot, query = read"
    );
}

fn cmd_knot(args: &[String], db_path: &str) {
    let file_path = args.get(2);

    let file_path = match file_path {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!("usage: quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]");
            std::process::exit(1);
        }
    };

    // Optional --shapes for SHACL validation before write.
    let shapes_path = args
        .windows(2)
        .find(|w| w[0] == "--shapes")
        .map(|w| w[1].as_str());

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

    // If shapes provided, validate first.
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
                    eprintln!("SHACL validation failed: {} violation(s)", feedback.violations);
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

    let format = if file_path.ends_with(".nt") || file_path.ends_with(".ntriples") {
        RdfFormat::NTriples
    } else {
        RdfFormat::Turtle
    };

    let now = chrono_now();
    match quipu::ingest_rdf(&mut store, data.as_bytes(), format, None, &now, None, Some(file_path))
    {
        Ok((tx_id, count)) => {
            println!("knotted {count} facts from {file_path} (tx {tx_id})");
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
            eprintln!("usage: quipu query \"SELECT ...\" [--valid-at <date>] [--tx N] [--db <path>]");
            std::process::exit(1);
        }
    };

    let valid_at = args
        .windows(2)
        .find(|w| w[0] == "--valid-at")
        .map(|w| w[1].clone());

    let as_of_tx: Option<i64> = args
        .windows(2)
        .find(|w| w[0] == "--tx")
        .and_then(|w| w[1].parse().ok());

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let ctx = quipu::TemporalContext {
        valid_at,
        as_of_tx,
    };

    run_query_temporal(&store, sparql, &ctx);
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

fn cmd_cord(args: &[String], db_path: &str) {
    let type_filter = args
        .windows(2)
        .find(|w| w[0] == "--type")
        .map(|w| w[1].as_str());

    let limit: usize = args
        .windows(2)
        .find(|w| w[0] == "--limit")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(100);

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let input = serde_json::json!({
        "type": type_filter,
        "limit": limit,
    });

    match quipu::tool_cord(&store, &input) {
        Ok(result) => {
            let entities = result["entities"].as_array().unwrap();
            for entity in entities {
                let iri = entity["iri"].as_str().unwrap_or("?");
                let facts = entity["facts"].as_array().unwrap();
                println!("{iri}");
                for fact in facts {
                    let pred = fact["predicate"].as_str().unwrap_or("?");
                    let val = &fact["value"];
                    let val_str = match val {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    println!("  {pred} → {val_str}");
                }
                println!();
            }
            println!("{} entities", result["count"]);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_unravel(args: &[String], db_path: &str) {
    let tx: Option<i64> = args
        .windows(2)
        .find(|w| w[0] == "--tx")
        .and_then(|w| w[1].parse().ok());

    let valid_at = args
        .windows(2)
        .find(|w| w[0] == "--valid-at")
        .map(|w| w[1].as_str());

    if tx.is_none() && valid_at.is_none() {
        eprintln!("usage: quipu unravel [--tx N] [--valid-at <date>]");
        eprintln!("  at least one of --tx or --valid-at is required");
        std::process::exit(1);
    }

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let input = serde_json::json!({
        "tx": tx,
        "valid_at": valid_at,
    });

    match quipu::tool_unravel(&store, &input) {
        Ok(result) => {
            let facts = result["facts"].as_array().unwrap();
            for fact in facts {
                let entity = fact["entity"].as_str().unwrap_or("?");
                let pred = fact["predicate"].as_str().unwrap_or("?");
                let val = &fact["value"];
                let val_str = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let vf = fact["valid_from"].as_str().unwrap_or("?");
                let vt = fact["valid_to"].as_str().unwrap_or("∞");
                println!("{entity}  {pred}  {val_str}  [{vf} → {vt}]");
            }
            println!("\n{} facts", result["count"]);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_episode(args: &[String], db_path: &str) {
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
        io::stdin().lock().read_to_string(&mut buf).unwrap_or_else(|e| {
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
    match quipu::ingest_episode(&mut store, &episode, &now) {
        Ok((tx_id, count)) => {
            println!(
                "ingested episode \"{}\" — {count} facts (tx {tx_id})",
                episode.name
            );
        }
        Err(e) => {
            eprintln!("error ingesting episode: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_retract(args: &[String], db_path: &str) {
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

fn cmd_shapes(args: &[String], db_path: &str) {
    let action = args.get(2).map(|s| s.as_str()).unwrap_or("list");

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
        "list" | _ => {
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

fn cmd_validate(args: &[String]) {
    let shapes_path = args
        .windows(2)
        .find(|w| w[0] == "--shapes")
        .map(|w| w[1].as_str());
    let data_path = args
        .windows(2)
        .find(|w| w[0] == "--data")
        .map(|w| w[1].as_str());

    let (shapes_path, data_path) = match (shapes_path, data_path) {
        (Some(s), Some(d)) => (s, d),
        _ => {
            eprintln!("usage: quipu validate --shapes <shapes.ttl> --data <data.ttl>");
            std::process::exit(1);
        }
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
                println!("✓ valid ({} warnings)", feedback.warnings);
            } else {
                println!("✗ invalid: {} violation(s), {} warning(s)", feedback.violations, feedback.warnings);
                for issue in &feedback.results {
                    println!(
                        "  [{:>9}] {} — {}{}",
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
    run_query_temporal(store, sparql, &quipu::TemporalContext::default());
}

fn run_query_temporal(store: &quipu::Store, sparql: &str, ctx: &quipu::TemporalContext) {
    match quipu::sparql_query_temporal(store, sparql, ctx) {
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
