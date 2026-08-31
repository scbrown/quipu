//! `quipu changes` — the change-feed CLI (quipu-2ae).
//!
//! Split from `cli_commands.rs` for the file-size ratchet, like `cli_pack.rs`.
//! The contract lives on `store::changes`; this is argument plumbing.

use crate::cli::flag_value;
use quipu::store::changes::Capture;

/// `quipu changes [--from <tx>] [--capture <mode>] [--limit <txs>]
/// [--graph <IRI>] [--db <path>]`.
///
/// Prints one page as JSON. Pass the page's `next_tx` back as `--from` to
/// page forward; an empty `records` with an advancing `watermark_tx` means
/// the store is idle, not broken.
pub fn cmd_changes(args: &[String], db_path: &str) {
    let since = flag_value(args, "--from").map_or(0, |s| {
        s.parse::<i64>().unwrap_or_else(|_| {
            eprintln!("--from must be a transaction id, got {s:?}");
            std::process::exit(1);
        })
    });
    let capture = match flag_value(args, "--capture") {
        None => Capture::NewValues,
        Some(name) => Capture::parse(name).unwrap_or_else(|| {
            eprintln!("--capture must be new_values, old_and_new_values, or new_row; got {name:?}");
            std::process::exit(1);
        }),
    };
    let limit = flag_value(args, "--limit")
        .map_or(100, |s| {
            s.parse::<usize>().unwrap_or_else(|_| {
                eprintln!("--limit must be a transaction count, got {s:?}");
                std::process::exit(1);
            })
        })
        .clamp(1, 10_000);

    let store = crate::cli_open::open_store(db_path);
    let graph = flag_value(args, "--graph").map(|iri| match store.lookup(iri) {
        Ok(Some(id)) => id,
        Ok(None) => {
            eprintln!("unknown graph IRI: {iri}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error resolving graph: {e}");
            std::process::exit(1);
        }
    });

    match store.changes_after(since, limit, capture, graph) {
        Ok(page) => println!(
            "{}",
            serde_json::to_string_pretty(&page.to_json()).expect("page serializes")
        ),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
