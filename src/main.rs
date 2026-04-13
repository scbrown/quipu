//! Quipu CLI -- AI-native knowledge graph.
//!
//! Commands:
//!   quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]  Assert facts
//!   quipu read "<sparql>" [--db <path>]   Run a SPARQL query
//!   quipu cord [--type <IRI>] [--limit N] [--db <path>]  List entities
//!   quipu unravel [--tx N] [--valid-at <date>] [--db <path>]  Time-travel query
//!   quipu impact <entity-IRI> [--remove] [--hops N] [--predicate <IRI>]...  Impact walk
//!   quipu reason [--rules <file.ttl>] [--db <path>]  Run the Datalog reasoner
//!   quipu validate --shapes <shapes.ttl> --data <data.ttl>  Validate without writing
//!   quipu episode <file.json> [--db <path>]  Ingest a structured episode
//!   quipu repl [--db <path>]             Interactive SPARQL prompt
//!   quipu export [--format ntriples|turtle] [--db <path>]  Export facts
//!   quipu stats [--db <path>]            Show store statistics
//!
//! Aliases: load=knot, query=read

mod cli;
mod cli_commands;
mod cli_propose;

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
    let config = quipu::QuipuConfig::load(std::path::Path::new(".")).with_db_override(db_flag);
    let db_path_buf = config.store_path.to_string_lossy().to_string();
    let db_path: &str = &db_path_buf;

    let cmd = args[1].as_str();
    match cmd {
        "knot" | "load" => cli::cmd_knot(&args, db_path),
        "read" | "query" => cli::cmd_query(&args, db_path),
        "cord" => cli::cmd_cord(&args, db_path),
        "unravel" => cli::cmd_unravel(&args, db_path),
        "impact" => cli::cmd_impact(&args, db_path),
        "reason" => cli::cmd_reason(&args, db_path),
        "episode" => cli_commands::cmd_episode(&args, db_path),
        "retract" => cli_commands::cmd_retract(&args, db_path),
        "shapes" => cli_commands::cmd_shapes(&args, db_path),
        "propose" => cli_propose::cmd_propose(&args, db_path),
        "ontology" => cmd_ontology(&args, db_path),
        "validate" => cli_commands::cmd_validate(&args),
        "repl" => cli_commands::cmd_repl(db_path),
        "export" => cli_commands::cmd_export(&args, db_path),
        "stats" => cli_commands::cmd_stats(db_path),
        "migrate-vectors" => cmd_migrate_vectors(&args, &config),
        "--help" | "-h" | "help" => print_usage(),
        _ => {
            eprintln!("unknown command: {cmd}");
            print_usage();
            std::process::exit(1);
        }
    }
}

fn cmd_ontology(args: &[String], db_path: &str) {
    #[cfg(feature = "owl")]
    {
        let sub = args.get(2).map_or("list", String::as_str);
        let mut store = quipu::Store::open(db_path).unwrap_or_else(|e| {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        });
        match sub {
            "load" => {
                let name = args.get(3).unwrap_or_else(|| {
                    eprintln!("usage: quipu ontology load <name> <file.ttl>");
                    std::process::exit(1);
                });
                let file = args.get(4).unwrap_or_else(|| {
                    eprintln!("usage: quipu ontology load <name> <file.ttl>");
                    std::process::exit(1);
                });
                let turtle = std::fs::read_to_string(file).unwrap_or_else(|e| {
                    eprintln!("error reading {file}: {e}");
                    std::process::exit(1);
                });
                let ts = chrono_now();
                let ont = quipu::Ontology::from_turtle(&turtle).unwrap_or_else(|e| {
                    eprintln!("error parsing ontology: {e}");
                    std::process::exit(1);
                });
                store.load_ontology(name, &turtle, &ts).unwrap_or_else(|e| {
                    eprintln!("error storing ontology: {e}");
                    std::process::exit(1);
                });
                let report = ont.materialize(&mut store, &ts).unwrap_or_else(|e| {
                    eprintln!("error materializing: {e}");
                    std::process::exit(1);
                });
                println!("Loaded ontology '{name}'");
                println!(
                    "  Axioms: {}",
                    serde_json::to_string_pretty(&ont.axiom_summary()).unwrap()
                );
                println!(
                    "  Materialized: {} facts ({} subclass, {} inverse, {} symmetric, {} domain/range, {} equiv-class)",
                    report.total,
                    report.subclass_inferences,
                    report.inverse_inferences,
                    report.symmetric_inferences,
                    report.domain_range_inferences,
                    report.equivalent_class_inferences
                );
            }
            "list" => {
                let list = store.list_ontologies().unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                });
                if list.is_empty() {
                    println!("No ontologies loaded.");
                } else {
                    for (name, _, loaded_at) in &list {
                        println!("{name}  (loaded {loaded_at})");
                    }
                }
            }
            "remove" => {
                let name = args.get(3).unwrap_or_else(|| {
                    eprintln!("usage: quipu ontology remove <name>");
                    std::process::exit(1);
                });
                if store.remove_ontology(name).unwrap() {
                    println!("Removed ontology '{name}'");
                } else {
                    println!("Ontology '{name}' not found");
                }
            }
            _ => {
                eprintln!("usage: quipu ontology load|list|remove");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(feature = "owl"))]
    {
        let _ = (args, db_path);
        eprintln!("error: ontology command requires the 'owl' feature");
        eprintln!("  rebuild with: cargo build --features owl");
        std::process::exit(1);
    }
}

#[cfg(feature = "owl")]
fn chrono_now() -> String {
    // Simple ISO-8601 timestamp without chrono dependency.
    use std::time::SystemTime;
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}Z", d.as_secs())
}

fn cmd_migrate_vectors(args: &[String], config: &quipu::QuipuConfig) {
    #[cfg(feature = "lancedb")]
    {
        // LanceDB requires a Tokio runtime for async operations.
        let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| {
            eprintln!("error creating Tokio runtime: {e}");
            std::process::exit(1);
        });
        let _guard = rt.enter();
        cli_commands::cmd_migrate_vectors(args, config);
    }
    #[cfg(not(feature = "lancedb"))]
    {
        let _ = (args, config);
        eprintln!("error: migrate-vectors requires the 'lancedb' feature");
        eprintln!("  rebuild with: cargo build --features lancedb");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!(
        "quipu -- AI-native knowledge graph

COMMANDS:
    quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]
    quipu read \"<sparql>\" [--db <path>]
    quipu cord [--type <IRI>] [--limit N] [--db <path>]
    quipu unravel [--tx N] [--valid-at <date>] [--db <path>]
    quipu impact <entity-IRI> [--remove] [--hops N] [--predicate <IRI>]... [--db <path>]
    quipu reason [--rules <file.ttl>] [--db <path>]
    quipu episode <file.json> [--db <path>]
    quipu retract <entity-IRI> [--predicate <IRI>] [--db <path>]
    quipu shapes load|list|remove [--db <path>]
    quipu propose list|submit|accept|reject [--status pending] [--db <path>]
    quipu ontology load|list|remove [--db <path>]
    quipu validate --shapes <shapes.ttl> --data <data.ttl>
    quipu repl [--db <path>]
    quipu export [--format ntriples|turtle] [--db <path>]
    quipu stats [--db <path>]
    quipu migrate-vectors --from sqlite --to lancedb [--dry-run] [--db <path>]

OPTIONS:
    --db <path>    Store file (default: quipu.db)

ALIASES:
    load = knot, query = read"
    );
}
