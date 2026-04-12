//! CLI command handlers for the `quipu` binary.

use oxrdfio::RdfFormat;

/// Simple ISO-8601 timestamp without pulling in chrono.
pub fn chrono_now() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
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

pub fn cmd_knot(args: &[String], db_path: &str) {
    let file_path = match args.get(2) {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!("usage: quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]");
            std::process::exit(1);
        }
    };

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

    let format = if file_path.ends_with(".nt") || file_path.ends_with(".ntriples") {
        RdfFormat::NTriples
    } else {
        RdfFormat::Turtle
    };

    let now = chrono_now();
    match quipu::ingest_rdf(
        &mut store,
        data.as_bytes(),
        format,
        None,
        &now,
        None,
        Some(file_path),
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

pub fn cmd_query(args: &[String], db_path: &str) {
    let sparql = match args.get(2) {
        Some(q) if !q.starts_with("--") => q,
        _ => {
            eprintln!(
                "usage: quipu query \"SELECT ...\" [--valid-at <date>] [--tx N] [--db <path>]"
            );
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

    let ctx = quipu::TemporalContext { valid_at, as_of_tx };

    run_query_temporal(&store, sparql, &ctx);
}

pub fn cmd_cord(args: &[String], db_path: &str) {
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
                    println!("  {pred} -> {val_str}");
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

pub fn cmd_unravel(args: &[String], db_path: &str) {
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
                let vt = fact["valid_to"].as_str().unwrap_or("inf");
                println!("{entity}  {pred}  {val_str}  [{vf} -> {vt}]");
            }
            println!("\n{} facts", result["count"]);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu impact` — walk the store outward from an entity and list what it reaches.
///
/// Phase 1 of the reasoner rollout: no rule engine, just a bounded BFS over
/// entity→entity edges. Answers "what is downstream of this entity?" on
/// current data and surfaces ontology gaps where an expected edge is missing.
pub fn cmd_impact(args: &[String], db_path: &str) {
    let entity_iri = match args.get(2) {
        Some(iri) if !iri.starts_with("--") => iri.as_str(),
        _ => {
            eprintln!(
                "usage: quipu impact <entity-IRI> [--hops N] [--predicate <IRI>]... [--db <path>]"
            );
            std::process::exit(1);
        }
    };

    let hops: usize = args
        .windows(2)
        .find(|w| w[0] == "--hops")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(quipu::DEFAULT_HOPS);

    // Collect all --predicate values (flag is repeatable).
    let predicates: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--predicate")
        .map(|w| w[1].clone())
        .collect();

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let opts = quipu::ImpactOptions { hops, predicates };
    let report = match quipu::impact(&store, entity_iri, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    // Text output. Root, then one line per reached entity grouped by depth.
    println!("root: {}", report.root);
    println!(
        "hops: {}  reached: {}  edges: {}",
        report.hops,
        report.reached.len().saturating_sub(1),
        report.edges_traversed
    );
    println!();

    if report.reached.len() == 1 {
        println!("(no reachable entities within {} hops)", report.hops);
        return;
    }

    // Header + one row per non-root node.
    println!("depth  via                                    entity");
    println!("{}", "-".repeat(80));
    for node in report.reached.iter().skip(1) {
        let via = node.via_predicate.as_deref().unwrap_or("?");
        // Truncate predicate to keep the table readable.
        let via_trunc = if via.len() > 38 {
            format!("…{}", &via[via.len() - 37..])
        } else {
            via.to_string()
        };
        println!("{:>5}  {:<38}  {}", node.depth, via_trunc, node.iri);
    }
}

/// `quipu reason` — load a Turtle ruleset and run the reasoner to a fixed point.
///
/// Phases 2–3 of the reasoner rollout: loads rules from a Turtle file
/// (default `shapes/aegis-rules.ttl`), stratifies them, runs every stratum
/// against the current EAVT snapshot, and writes derived facts back through
/// the store with `source = reasoner:<rule-id>`. Each call is a full
/// re-derivation — tuples that were derived last run but no longer hold are
/// retracted.
///
/// With `--reactive` (requires the `reactive-reasoner` feature), registers
/// a [`TransactObserver`] so derived facts stay fresh automatically on
/// subsequent `transact()` calls within this session.
pub fn cmd_reason(args: &[String], db_path: &str) {
    let rules_path = args
        .windows(2)
        .find(|w| w[0] == "--rules")
        .map_or("shapes/aegis-rules.ttl", |w| w[1].as_str());

    #[cfg(feature = "reactive-reasoner")]
    let reactive = args.iter().any(|a| a == "--reactive");

    let ttl = match std::fs::read_to_string(rules_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading rules file {rules_path}: {e}");
            std::process::exit(1);
        }
    };

    let ruleset = match quipu::reasoner::parse_rules(&ttl, None) {
        Ok(rs) => rs,
        Err(e) => {
            eprintln!("error parsing rules: {e}");
            std::process::exit(1);
        }
    };

    if ruleset.is_empty() {
        println!("no rules found in {rules_path}");
        return;
    }

    let mut store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    let now = chrono_now();
    let report = match quipu::reasoner::evaluate(&mut store, &ruleset, &now) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("reasoner error: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "reasoner: {} rules across {} strata — asserted {}, retracted {}",
        ruleset.len(),
        report.strata_run,
        report.asserted,
        report.retracted
    );
    if !report.per_rule.is_empty() {
        println!();
        println!("per-rule contributions:");
        for (rule_id, count) in &report.per_rule {
            println!("  {rule_id:<20}  {count}");
        }
    }

    #[cfg(feature = "reactive-reasoner")]
    if reactive {
        let observer = std::sync::Arc::new(quipu::ReactiveReasoner::new(ruleset));
        store.add_observer(observer);
        println!("\nreactive observer registered — derived facts will auto-update on transact");
    }
}

pub fn format_value(store: &quipu::Store, val: &quipu::Value) -> String {
    match val {
        quipu::Value::Ref(id) => store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}")),
        quipu::Value::Str(s) => format!("\"{s}\""),
        quipu::Value::Int(n) => n.to_string(),
        quipu::Value::Float(f) => f.to_string(),
        quipu::Value::Bool(b) => b.to_string(),
        quipu::Value::Bytes(b) => format!("<{} bytes>", b.len()),
    }
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
