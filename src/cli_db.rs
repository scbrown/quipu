//! Database maintenance subcommands kept separate from the general CLI surface.

/// List read-only stores attached alongside the primary database.
pub fn list_attachments(args: &[String], db_path: &str) {
    if !args.iter().any(|a| a == "--list") {
        eprintln!("usage: quipu db attach --list [--db <path>]");
        std::process::exit(1);
    }
    let store = crate::cli_open::open_store(db_path);
    let mounted = quipu::config::describe_attachments(&store);
    if mounted.is_empty() {
        println!("no attachments mounted");
        return;
    }
    println!("alias\tpath\tmode");
    for line in mounted {
        println!("{line}");
    }
}

/// Move existing engine-derived facts into companion inferred graphs.
pub fn migrate_inferred(db_path: &str) {
    let mut store = crate::cli_open::open_store(db_path);
    let now = crate::cli::chrono_now();
    match store.migrate_inferred(&now) {
        Ok((graphs, facts)) => {
            println!(
                "migrate-inferred: moved {facts} derived fact(s) across {graphs} graph(s) \
                 into their companion inferred graphs"
            );
        }
        Err(e) => {
            eprintln!("migrate-inferred error: {e}");
            std::process::exit(1);
        }
    }
}
