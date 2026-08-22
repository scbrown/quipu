//! `quipu fork` — persistent named forks (quipu-gp5).
//!
//! Fork ROOT at any transaction into an independent named lineage, read it
//! exactly like the main line (`--fork <n>` on `quipu read`), diff it
//! structurally, then drop it or promote it. Promotion re-enters through the
//! SHACL + policy write gates — see `docs/design/fork-at-any-event.md`.

use quipu::store::Store;

use crate::cli::{flag_value, format_value, resolve_timestamp};

const USAGE: &str = "usage: quipu fork <tx> [--name <n>] [--timestamp <ISO-8601>] [--db <path>]
       quipu fork list [--db <path>]
       quipu fork diff <a> <b> [--db <path>]      each side: a fork name, or 'main'
       quipu fork drop <name> [--db <path>]
       quipu fork promote <name> [--db <path>]
read a fork with:  quipu read \"<sparql>\" --fork <name>";

fn open(db_path: &str) -> Store {
    Store::open(db_path).unwrap_or_else(|e| {
        eprintln!("error opening store: {e}");
        std::process::exit(1);
    })
}

fn die() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(1);
}

fn or_die<T>(result: quipu::Result<T>) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    })
}

/// Dispatch `quipu fork <sub>`.
pub fn cmd_fork(args: &[String], db_path: &str) {
    match args.get(2).map(String::as_str) {
        Some("list") => cmd_list(db_path),
        Some("diff") => match (args.get(3), args.get(4)) {
            (Some(a), Some(b)) if !a.starts_with("--") && !b.starts_with("--") => {
                cmd_diff(db_path, a, b);
            }
            _ => die(),
        },
        Some("drop") => match args.get(3) {
            Some(name) if !name.starts_with("--") => cmd_drop(args, db_path, name),
            _ => die(),
        },
        Some("promote") => match args.get(3) {
            Some(name) if !name.starts_with("--") => cmd_promote(args, db_path, name),
            _ => die(),
        },
        Some(tx) => match tx.parse::<i64>() {
            Ok(tx) => cmd_create(args, db_path, tx),
            Err(_) => die(),
        },
        None => die(),
    }
}

/// Resolve `--fork <name>` on a read command to a graph scope; the ROOT
/// default when the flag is absent. Unknown and dropped forks are refused
/// loudly — never a silent fall-through to ROOT.
pub fn fork_scope(store: &Store, args: &[String]) -> quipu::GraphScope {
    match flag_value(args, "--fork") {
        None => quipu::GraphScope::default(),
        Some(name) => match store.fork_graph_for_read(name) {
            Ok(g) => quipu::GraphScope::Default(vec![g]),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
    }
}

fn cmd_create(args: &[String], db_path: &str, tx: i64) {
    let default_name = format!("fork-{tx}");
    let name = flag_value(args, "--name").unwrap_or(&default_name);
    let ts = resolve_timestamp(args);
    let mut store = open(db_path);
    let info = or_die(store.fork_create(name, tx, &ts, None));
    println!(
        "forked ROOT as of tx {} into '{}' ({})",
        info.fork_tx,
        info.name,
        Store::fork_iri(&info.name)
    );
    println!(
        "read it with:  quipu read \"<sparql>\" --fork {}",
        info.name
    );
}

fn cmd_list(db_path: &str) {
    let store = open(db_path);
    let forks = or_die(store.fork_list());
    if forks.is_empty() {
        println!("no forks");
        return;
    }
    println!(
        "{:<24}  {:>8}  {:<10}  created",
        "name", "fork-tx", "status"
    );
    for f in forks {
        println!(
            "{:<24}  {:>8}  {:<10}  {}",
            f.name, f.fork_tx, f.status, f.created_at
        );
    }
}

fn cmd_diff(db_path: &str, a: &str, b: &str) {
    let store = open(db_path);
    let diff = or_die(store.fork_diff(a, b));
    // Sorted, so the output is stable rather than term-id-ordered.
    let mut lines: Vec<String> = Vec::new();
    for f in &diff.added {
        lines.push(format!("+ {}", triple_line(&store, f)));
    }
    for f in &diff.removed {
        lines.push(format!("- {}", triple_line(&store, f)));
    }
    lines.sort_by(|x, y| x[2..].cmp(&y[2..]).then(x.cmp(y)));
    for line in &lines {
        println!("{line}");
    }
    println!(
        "\n{a} -> {b}: +{} -{} (present-state triples only; \
         valid-time intervals and per-tx attribution are unravel's job)",
        diff.added.len(),
        diff.removed.len()
    );
}

fn triple_line(store: &Store, f: &quipu::Fact) -> String {
    let s = store
        .resolve(f.entity)
        .unwrap_or_else(|_| format!("ref:{}", f.entity));
    let p = store
        .resolve(f.attribute)
        .unwrap_or_else(|_| format!("ref:{}", f.attribute));
    format!("<{s}> <{p}> {}", format_value(store, &f.value))
}

fn cmd_drop(args: &[String], db_path: &str, name: &str) {
    let ts = resolve_timestamp(args);
    let mut store = open(db_path);
    or_die(store.fork_drop(name, &ts));
    println!("fork '{name}' dropped (its facts remain as history; the name is not reusable)");
}

fn cmd_promote(args: &[String], db_path: &str, name: &str) {
    let ts = resolve_timestamp(args);
    let mut store = open(db_path);
    match or_die(store.fork_promote(name, &ts, None)) {
        quipu::ForkPromotion::Promoted {
            tx,
            asserted,
            retracted,
        } => {
            println!(
                "promoted fork '{name}' to ROOT: asserted {asserted}, retracted {retracted} (tx {tx})"
            );
        }
        #[cfg(feature = "shacl")]
        quipu::ForkPromotion::Refused(feedback) => {
            eprintln!(
                "promotion REFUSED by SHACL: {} violation(s); nothing was written",
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
    }
}
