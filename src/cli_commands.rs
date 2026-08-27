//! Additional CLI commands: shapes, validate, export, episode, retract, repl, stats.

use std::io::{self, BufRead, Read, Write};

use oxrdfio::RdfFormat;

use crate::cli::{chrono_now, flag_value, format_value, resolve_timestamp};

pub fn cmd_episode(args: &[String], db_path: &str, config_base_ns: &str) {
    let file_arg = match args.get(2) {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!(
                "usage: quipu episode <file.json> [--base-ns <IRI>] [--timestamp <ISO-8601>] [--db <path>]"
            );
            eprintln!("  use - to read from stdin");
            std::process::exit(1);
        }
    };

    // Namespace IRIs are minted in (quipu #28). Precedence: --base-ns flag >
    // configured [quipu].base_ns > built-in aegis default. Before aegis-4h3x the
    // fallback was DEFAULT_BASE_NS directly, so the CLI ignored config just like
    // the REST/MCP paths did; the config value (which itself defaults to
    // DEFAULT_BASE_NS) is now the fallback, so a non-aegis deployment's config is
    // honoured while the flag still wins.
    let base_ns = flag_value(args, "--base-ns").unwrap_or(config_base_ns);

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

    let mut store = crate::cli_open::open_store(db_path);

    let now = resolve_timestamp(args);
    match quipu::ingest_episode(&mut store, &episode, &now, base_ns) {
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
            eprintln!(
                "usage: quipu retract <entity-IRI> [--predicate <IRI>] [--timestamp <ISO-8601>] [--db <path>]"
            );
            std::process::exit(1);
        }
    };

    let predicate_iri = flag_value(args, "--predicate");

    let mut store = crate::cli_open::open_store(db_path);

    let mut input = serde_json::json!({
        "entity": entity_iri,
        "timestamp": resolve_timestamp(args),
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

    let store = crate::cli_open::open_store(db_path);

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
    let store = crate::cli_open::open_store(db_path);

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

    // Subset export (quipu #36): `--graph <iri>` pulls one named graph's slice
    // (or the ROOT default when omitted, which matches the pre-subset behaviour
    // ONLY when there are no named graphs; with named graphs present, no --graph
    // still exports every graph flattened, as before).
    let graph = args
        .windows(2)
        .find(|w| w[0] == "--graph")
        .map(|w| w[1].as_str());

    let store = crate::cli_open::open_store(db_path);

    let exported = match graph {
        Some(iri) => {
            quipu::export_rdf_subset(&store, rdf_format, Some(iri)).map(|(bytes, _)| bytes)
        }
        None => quipu::export_rdf(&store, rdf_format),
    };
    match exported {
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
    let store = crate::cli_open::open_store(db_path);

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
    let store = crate::cli_open::open_store(&db_path);

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
                    // This instruction was DELETED for a year and a half of
                    // commits, because printing it would have been the product
                    // directing the user to a no-op: vector.backend was not
                    // read by either binary. quipu-lv7 wired it, so the next
                    // step is real again — and a binary built without the
                    // feature now REFUSES the key rather than ignoring it, so
                    // following this cannot silently do nothing.
                    println!(
                        "  next: set `[quipu.vector] backend = \"lancedb\"` to query it.\n  \
                         The binary must be built with `--features lancedb`; one that is not\n  \
                         refuses the key rather than falling back to the SQLite table."
                    );
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

/// `quipu doctor labels` — recompute every graph's label from the meta-graph
/// facts and report where the `graphs` cache disagrees (quipu #65).
///
/// RDF is the source of truth. A non-empty report means the CACHE is wrong,
/// never the facts — so this reports rather than repairs, and says which side
/// is authoritative in its own output. Exits non-zero on drift so a cron or CI
/// caller can gate on it without parsing the text.
pub fn cmd_doctor(args: &[String], db_path: &str) {
    let sub = args.get(2).map_or("labels", String::as_str);
    if sub != "labels" {
        eprintln!("usage: quipu doctor labels [--db <path>]");
        std::process::exit(1);
    }

    let store = crate::cli_open::open_store(db_path);

    // quipu #80: surface producers' RECOMMENDED floors here. They are advisory
    // — the store never applies them — so they are reported beside the drift
    // report rather than mixed into it, and the banner says so on every line.
    // (#75's attach path is the other intended print point; `RecommendedFloor::line`
    // is what it will call.)
    if let Ok(graphs) = store.all_named_graph_ids() {
        let mut shown = false;
        for g in graphs {
            let Ok(iri) = store.resolve(g) else { continue };
            let Ok(rec) = store.recommended_floor(&iri) else {
                continue;
            };
            if !rec.is_empty() {
                if !shown {
                    println!("recommended floors (advisory — NOT enforced):");
                    shown = true;
                }
                println!("  {}", rec.line(&iri));
            }
        }
        if shown {
            println!();
        }
    }

    match store.graph_label_drift() {
        Ok(drift) if drift.is_empty() => {
            println!("labels: no drift — every cached label agrees with the meta-graph");
        }
        Ok(drift) => {
            println!(
                "labels: {} disagreement(s) between the meta-graph (authoritative) \
                 and the graphs cache\n",
                drift.len()
            );
            for d in &drift {
                println!("  {}", d.graph_iri);
                println!("    axis:   {}", d.axis);
                println!("    rdf:    {}   <- authoritative", d.rdf);
                println!("    cached: {}", d.cached);
            }
            println!(
                "\nThe cache is derived; the facts are the truth. Re-declare these \
                 graphs' labels with set_graph_label to rebuild the cache."
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("doctor error: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu db respace --into <space> --out <file>` (quipu #74).
///
/// Deliberately requires an explicit `--out`: respace is the one operation here
/// that produces a store whose ids differ from every other copy, and defaulting
/// the destination is how one gets written over something.
/// `quipu db attach --list` — what is actually mounted alongside this store.
///
/// The visibility surface `[[quipu.attachments]]` needs to be operable: a
/// declared layer that failed to mount refuses the open, so anything listed
/// here IS composed — and the list also shows deep freeze's archives, which no
/// config declares. `--list` is required rather than defaulted so the
/// subcommand stays open for a future verb without changing what a bare
/// `quipu db attach` means today.
pub fn cmd_db(args: &[String], db_path: &str) {
    let sub = args.get(2).map_or("", String::as_str);
    if sub == "attach" {
        crate::cli_db::list_attachments(args, db_path);
        return;
    }
    // quipu-0b6: one-time move of engine-derived facts (source
    // owl:materialize / reasoner:*) out of their premise graphs into the
    // companion inferred graphs the entailment regime places them in.
    if sub == "migrate-inferred" {
        crate::cli_db::migrate_inferred(db_path);
        return;
    }
    if sub != "respace" {
        eprintln!(
            "usage: quipu db respace --into <space> --out <file> [--db <path>]\n       \
             quipu db attach --list [--db <path>]\n       \
             quipu db migrate-inferred [--db <path>]"
        );
        std::process::exit(1);
    }

    let Some(into) = flag_value(args, "--into") else {
        eprintln!("quipu db respace requires --into <space>");
        std::process::exit(1);
    };
    let Ok(space) = into.parse::<i64>() else {
        eprintln!("--into must be an integer space number, got {into:?}");
        std::process::exit(1);
    };
    let Some(out) = flag_value(args, "--out") else {
        eprintln!("quipu db respace requires --out <file>");
        std::process::exit(1);
    };

    match quipu::store::respace::respace_file(
        std::path::Path::new(db_path),
        std::path::Path::new(out),
        space,
    ) {
        Ok(report) => {
            println!(
                "respaced {db_path} -> {out}\n  space: {} -> {}\n  rows touched: {}",
                report.from_space,
                report.to_space,
                report.rows()
            );
            for (table, column, n) in &report.columns {
                println!("    {table}.{column}: {n}");
            }
            println!("    facts.v (Ref blobs): {}", report.ref_blobs);
            println!(
                "  the original is untouched: {db_path} was opened read-only and \
                 still owns space {}",
                report.from_space
            );
        }
        Err(e) => {
            eprintln!("respace error: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu events refusals` — count refused writes by gate (camayoc-0d3).
///
/// The incident-rate denominator: how many writes were attempted and refused,
/// and by which gate. Reads the `write.refused` events the write path records
/// after each gate refusal; the raw events (with graph/actor/source/reason)
/// are served by `GET /events?types=write.refused`.
pub fn cmd_events(args: &[String], db_path: &str) {
    let sub = args.get(2).map_or("", String::as_str);
    if sub != "refusals" {
        eprintln!("usage: quipu events refusals [--db <path>]");
        std::process::exit(1);
    }

    let store = crate::cli_open::open_store(db_path);

    match store.refusals_by_gate() {
        Ok(counts) => {
            if counts.is_empty() {
                println!("no refused writes recorded");
                return;
            }
            let mut total = 0i64;
            println!("gate        refused");
            println!("{}", "-".repeat(20));
            for (gate, n) in &counts {
                println!("{gate:<11} {n}");
                total += n;
            }
            println!("{}", "-".repeat(20));
            println!("total       {total}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
