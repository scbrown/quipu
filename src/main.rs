//! Quipu CLI -- AI-native knowledge graph.
//!
//! Commands:
//!   quipu knot <file.ttl> [--shapes <shapes.ttl>] [--db <path>]  Assert facts
//!   quipu read "<sparql>" [--db <path>]   Run a SPARQL query
//!   quipu cord [--type <IRI>] [--limit N] [--db <path>]  List entities
//!   quipu unravel [--tx N] [--valid-at <date>] [--db <path>]  Time-travel query
//!   quipu impact <entity-IRI> [--remove] [--hops N] [--predicate <IRI>]...  Impact walk
//!   quipu explain <s> <p> <o> [--depth N]  Walk a derived fact back to its premises
//!   quipu reason [--rules <file.ttl>] [--db <path>]  Run the Datalog reasoner
//!   quipu validate --shapes <shapes.ttl> --data <data.ttl>  Validate without writing
//!   quipu episode <file.json> [--db <path>]  Ingest a structured episode
//!   quipu repl [--db <path>]             Interactive SPARQL prompt
//!   quipu export [--format ntriples|turtle] [--db <path>]  Export facts
//!   quipu stats [--db <path>]            Show store statistics
//!   quipu policy draft|backtest ...       Draft an advisory policy from an exemplar; backtest it pre-creation
//!   quipu audit <trace.jsonl>|inventory|replay <trace.jsonl>  Check a trace against Σ
//!   quipu audit namespace                                   Report base-namespace drift
//!   quipu db respace --into <space> --out <file>  Move a store into a term space
//!   quipu db attach --list                List the databases mounted alongside this store
//!   quipu pack <graph-iri> --out <file> [--space N]  Export a graph as an attachable pack
//!   quipu unpack <file> [--into <graph-iri>]  Materialize a pack into a local graph
//!   quipu fork <tx>|list|diff|drop|promote  Persistent named forks of ROOT
//!
//! Aliases: load=knot, query=read

mod cli;
mod cli_audit;
mod cli_commands;
mod cli_db;
mod cli_explain;
mod cli_fork;
mod cli_graph;
mod cli_open;
mod cli_pack;
mod cli_path;
mod cli_policy;
mod cli_propose;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return;
    }

    // Asking the binary who it is is a pure read of a compiled-in constant --
    // it must not load config or open a store (aegis-j0nq).
    if args[1] == "--version" || args[1] == "-V" || args[1] == "version" {
        println!("quipu {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Parse --db flag from anywhere in args (overrides config file).
    let db_flag = args
        .windows(2)
        .find(|w| w[0] == "--db")
        .map(|w| w[1].as_str());

    // Load config from .bobbin/config.toml, then apply CLI overrides.
    let config = quipu::QuipuConfig::load(std::path::Path::new(".")).with_db_override(db_flag);
    // a documented, settable knob that this binary does not act on must
    // be LOUD, not silently inert. Only fires when the user actually set one.
    for warning in config.unwired_warnings() {
        eprintln!("warning: {warning}");
    }
    let db_path_buf = config.store_path.to_string_lossy().to_string();
    let db_path: &str = &db_path_buf;

    // quipu-lv7: a configured LanceDB backend is opened and queried through
    // async calls, so every command that touches a store needs a runtime in
    // context. Entered here, once, and held for the whole dispatch — the
    // per-command opens in `cli_open` install the backend inside it. A
    // sqlite-backed store (the default) enters nothing.
    let _vector_rt = vector_runtime(&config);

    let cmd = args[1].as_str();
    match cmd {
        "knot" | "load" => cli::cmd_knot(&args, db_path),
        "read" | "query" => cli::cmd_query(&args, db_path),
        "cord" => cli::cmd_cord(&args, db_path),
        "unravel" => cli::cmd_unravel(&args, db_path),
        "impact" => cli::cmd_impact(&args, db_path),
        "explain" => cli_explain::cmd_explain(&args, db_path),
        "project" => cli::cmd_project(&args, db_path),
        "report" => cli::cmd_report(&args, db_path),
        "reason" => cli::cmd_reason(&args, db_path),
        "episode" => cli_commands::cmd_episode(&args, db_path, &config.base_ns),
        "retract" => cli_commands::cmd_retract(&args, db_path),
        "shapes" => cli_commands::cmd_shapes(&args, db_path),
        "policy" => cli_policy::cmd_policy(&args, db_path),
        "path" => cli_path::cmd_path(&args, db_path, &config.base_ns),
        "propose" => cli_propose::cmd_propose(&args, db_path),
        "audit" => cli_audit::cmd_audit(&args, db_path, &config.base_ns),
        "ontology" => cmd_ontology(&args, db_path),
        "validate" => cli_commands::cmd_validate(&args),
        "repl" => cli_commands::cmd_repl(db_path),
        "export" => cli_commands::cmd_export(&args, db_path),
        "stats" => cli_commands::cmd_stats(db_path),
        "doctor" => cli_commands::cmd_doctor(&args, db_path),
        "pack" => cli_pack::cmd_pack(&args, db_path),
        "share" => cli_pack::cmd_share(&args, db_path),
        "db" => cli_commands::cmd_db(&args, db_path),
        "events" => cli_commands::cmd_events(&args, db_path),
        "graph" => cli_graph::cmd_graph(&args, db_path),
        "fork" => cli_fork::cmd_fork(&args, db_path),
        "unpack" => cli_pack::cmd_unpack(&args, db_path),
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
        let mut store = cli_open::open_store(db_path);
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
    quipu::time::now_iso()
}

/// A Tokio runtime for the configured vector backend, or `None` when the
/// default `SQLite` backend needs no async at all.
///
/// The returned guard must outlive every store open — see the call site.
#[cfg(feature = "lancedb")]
fn vector_runtime(config: &quipu::QuipuConfig) -> Option<tokio::runtime::EnterGuard<'static>> {
    if config.vector.backend != quipu::VectorBackend::Lancedb {
        return None;
    }
    let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        eprintln!("error creating Tokio runtime for the configured vector backend: {e}");
        std::process::exit(1);
    });
    // Leaked deliberately: the guard borrows the runtime, and the runtime must
    // outlive every store open in this dispatch — which is the whole process.
    // A one-per-process leak released at exit beats a self-referential struct.
    Some(Box::leak(Box::new(rt)).enter())
}

/// Without the `lancedb` feature there is no async backend to host — and a
/// configured one is refused by `install_vector_backend`, not silently
/// downgraded.
#[cfg(not(feature = "lancedb"))]
fn vector_runtime(_config: &quipu::QuipuConfig) -> Option<()> {
    None
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
    quipu knot <file.ttl> [--shapes <shapes.ttl>] [--timestamp <ISO-8601>] [--db <path>]
    quipu read \"<sparql>\" [--valid-at <date>] [--tx N] [--fork <name>] [--db <path>]
    quipu cord [--type <IRI>] [--limit N] [--db <path>]
    quipu unravel [--tx N] [--valid-at <date>] [--db <path>]
    quipu impact <entity-IRI> [--remove] [--hops N] [--predicate <IRI>]... [--db <path>]
    quipu explain <subject-IRI> <predicate-IRI> <object> [--depth N] [--db <path>]
    quipu path <cone|backtest|draft> <trajectory-IRI> [options] [--db <path>]
    quipu project [--algorithm pagerank] [--seed <IRI>]... [--damping 0.85] [--predicate <IRI>] [--graph <IRI>] [--db <path>]
    quipu report [--hubs N] [--surprises N] [--questions N] [--type <IRI>] [--predicate <IRI>] [--db <path>]
    quipu reason [--rules <file.ttl>] [--db <path>]
    quipu episode <file.json> [--base-ns <ns>] [--timestamp <ISO-8601>] [--db <path>]
    quipu retract <entity-IRI> [--predicate <IRI>] [--db <path>]
    quipu shapes load|list|remove [--db <path>]
    quipu policy draft --exemplar <iri> --name <slug> --label <sentence> --targets <type-IRI> --claim <ask> [--out <file.ttl>]
    quipu policy backtest <candidate.ttl> [--last-txs N] [--from-tx A --to-tx B] [--db <path>]
    quipu propose list|submit|accept|reject [--status pending] [--db <path>]
    quipu ontology load|list|remove [--db <path>]
    quipu validate --shapes <shapes.ttl> --data <data.ttl>
    quipu repl [--db <path>]
    quipu export [--graph <iri>] [--format ntriples|turtle] [--db <path>]
    quipu stats [--db <path>]
    quipu doctor labels [--db <path>]
    quipu pack <graph-iri> --out <file.qpack.db> [--name N] [--version V] [--space N] [--shapes S]... [--queries Q]... [--with-vectors] [--format turtle]
    quipu pack --verify <file.qpack.db>
    quipu db respace --into <space> --out <file> [--db <path>]
    quipu db attach --list [--db <path>]
    quipu events refusals [--db <path>]
    quipu graph import <db> --as <iri> [--db <path>]
    quipu fork <tx> [--name <n>] | list | diff <a> <b> | drop <n> | promote <n>  [--db <path>]
    quipu unpack <file.qpack.db> [--into <graph-iri>] [--db <path>]
    quipu share --output <dir> [--graph IRI|--group-id ID|--construct QUERY] [--shapes NAME]... [--parent-share ID] [--turtle]
    quipu audit <trace.jsonl>|inventory|replay|tree|inheritance <trace.jsonl> [--json] [--db <path>]
    quipu audit namespace [--graph <iri>] [--json] [--db <path>]
    quipu migrate-vectors --from sqlite --to lancedb [--dry-run] [--db <path>]

OPTIONS:
    --db <path>       Store file (default: .bobbin/quipu/quipu.db)
    -V, --version     Print version and exit
    -h, --help        Print this help and exit

ALIASES:
    load = knot, query = read"
    );
}
