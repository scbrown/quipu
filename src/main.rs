//! Quipu CLI -- AI-native knowledge graph.
//!
//! Commands:
//!   quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]  Assert facts
//!   quipu read "<sparql>" [--db <path>]   Run a SPARQL query
//!   quipu cord [--type <IRI>] [--limit N] [--db <path>]  List entities
//!   quipu unravel [--tx N] [--valid-at <date>] [--db <path>]  Time-travel query
//!   quipu validate --shapes <shapes.ttl> --data <data.ttl>  Validate without writing
//!   quipu episode <file.json> [--db <path>]  Ingest a structured episode
//!   quipu repl [--db <path>]             Interactive SPARQL prompt
//!   quipu export [--format ntriples|turtle] [--db <path>]  Export facts
//!   quipu stats [--db <path>]            Show store statistics
//!
//! Aliases: load=knot, query=read

mod cli;
mod cli_commands;

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
        "episode" => cli_commands::cmd_episode(&args, db_path),
        "retract" => cli_commands::cmd_retract(&args, db_path),
        "shapes" => cli_commands::cmd_shapes(&args, db_path),
        "validate" => cli_commands::cmd_validate(&args),
        "repl" => cli_commands::cmd_repl(db_path),
        "export" => cli_commands::cmd_export(&args, db_path),
        "stats" => cli_commands::cmd_stats(db_path),
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
        "quipu -- AI-native knowledge graph

COMMANDS:
    quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]
    quipu read \"<sparql>\" [--db <path>]
    quipu cord [--type <IRI>] [--limit N] [--db <path>]
    quipu unravel [--tx N] [--valid-at <date>] [--db <path>]
    quipu episode <file.json> [--db <path>]
    quipu retract <entity-IRI> [--predicate <IRI>] [--db <path>]
    quipu shapes load|list|remove [--db <path>]
    quipu validate --shapes <shapes.ttl> --data <data.ttl>
    quipu repl [--db <path>]
    quipu export [--format ntriples|turtle] [--db <path>]
    quipu stats [--db <path>]

OPTIONS:
    --db <path>    Store file (default: quipu.db)

ALIASES:
    load = knot, query = read"
    );
}
