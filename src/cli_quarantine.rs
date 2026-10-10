//! `quipu audit replay <verdict>` and `quipu audit quarantine` — the CLI half of
//! the denial quarantine (`src/governance/quarantine.rs`,
//! `src/governance/denial_replay.rs`).
//!
//! **Exit codes follow `quipu audit`.** `1` only when a replay CONTRADICTS the
//! record — a different outcome, a different rule set or post-state, a broken
//! seal. An attestation-only replay (content purged, digest-only with no delta
//! presented) exits `0`: it is incomplete evidence, not contrary evidence, and
//! a gate that failed on it would be switched off along with the real findings.

use quipu::governance::{denial_replay, quarantine};

/// `quipu audit replay <verdict> [--delta <file>] [--json]`.
pub fn cmd_replay_verdict(args: &[String], store: &mut quipu::Store, verdict: &str) {
    // The gate reads its configuration, and the CLI's store starts from the
    // built-in defaults; the replay must judge under the deployment's.
    let config = crate::cli_open::config();
    store.governance_config_mut().clone_from(&config.governance);
    store.owl_config_mut().clone_from(&config.owl);
    store.labels_config_mut().clone_from(&config.label_floors);

    let presented = flag(args, "--delta").map(|path| {
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error reading {path}: {e}");
            std::process::exit(1);
        })
    });
    let replays = denial_replay::replay_verdict(store, verdict, presented.as_deref())
        .unwrap_or_else(|e| {
            eprintln!("error replaying verdict: {e}");
            std::process::exit(1);
        });
    let contradicted = replays
        .iter()
        .any(denial_replay::VerdictReplay::contradicts);
    if args.iter().any(|a| a == "--json") {
        let items: Vec<serde_json::Value> = replays
            .iter()
            .map(denial_replay::VerdictReplay::to_json)
            .collect();
        println!(
            "{}",
            serde_json::json!({ "verdict": verdict, "contradicts": contradicted, "replays": items })
        );
    } else {
        for r in &replays {
            println!("{}", r.line());
        }
    }
    if contradicted {
        std::process::exit(1);
    }
}

/// `quipu audit quarantine [list] [--verdict <iri>] [--json]` and
/// `quipu audit quarantine purge (--graph <iri> | --before <ts> | --older-than <days> | --all)`.
pub fn cmd_quarantine(args: &[String], store: &quipu::Store) {
    let action = args
        .get(3)
        .filter(|a| !a.starts_with("--"))
        .map_or("list", String::as_str);
    match action {
        "list" => list(args, store),
        "purge" => purge(args, store),
        other => {
            eprintln!(
                "unknown quarantine action '{other}'\nusage: quipu audit quarantine \
                 [list [--verdict <iri>]] | purge (--graph <iri> | --before <ts> | \
                 --older-than <days> | --all) [--json]"
            );
            std::process::exit(1);
        }
    }
}

fn list(args: &[String], store: &quipu::Store) {
    let entries = quarantine::entries(store, flag(args, "--verdict")).unwrap_or_else(|e| {
        eprintln!("error reading the quarantine: {e}");
        std::process::exit(1);
    });
    if args.iter().any(|a| a == "--json") {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "verdict": e.verdict,
                    "attempt": e.attempt,
                    "graph": e.graph,
                    "base_tx": e.base_tx,
                    "at": e.at,
                    "actor": e.actor,
                    "retention": e.retention,
                    "sealed_delta": e.sealed_delta,
                    "purged_at": e.purged_at,
                })
            })
            .collect();
        println!("{}", serde_json::json!({ "entries": items }));
        return;
    }
    println!("{} quarantine entr(y/ies)", entries.len());
    for e in &entries {
        // "content" names what a replay can rest on without anything presented.
        let content = match (&e.purged_at, e.sealed_delta) {
            (Some(at), _) => format!("content purged {at}"),
            (None, true) => "sealed delta held".to_string(),
            (None, false) => "digest only".to_string(),
        };
        println!(
            "{id} {verdict} graph={graph} at={at} base_tx={base} {content}",
            id = e.id,
            verdict = e.verdict,
            graph = e.graph,
            at = e.at,
            base = e.base_tx,
        );
    }
}

fn purge(args: &[String], store: &quipu::Store) {
    let graph = flag(args, "--graph");
    let before = match (flag(args, "--before"), flag(args, "--older-than")) {
        (Some(ts), _) => Some(ts.to_string()),
        (None, Some(days)) => {
            let days: u64 = days.parse().unwrap_or_else(|_| {
                eprintln!("--older-than takes a number of days, got '{days}'");
                std::process::exit(1);
            });
            Some(quipu::time::iso_days_ago(days))
        }
        (None, None) => None,
    };
    // An unscoped purge must be asked for by name: erasing every sealed delta
    // is a legitimate retention decision, and not one a missing flag makes.
    if graph.is_none() && before.is_none() && !args.iter().any(|a| a == "--all") {
        eprintln!(
            "usage: quipu audit quarantine purge (--graph <iri> | --before <ts> | \
             --older-than <days> | --all)"
        );
        std::process::exit(1);
    }
    let now = quipu::time::now_iso();
    let purged = quarantine::purge(store, graph, before.as_deref(), &now).unwrap_or_else(|e| {
        eprintln!("error purging the quarantine: {e}");
        std::process::exit(1);
    });
    println!(
        "purged the sealed content of {purged} quarantine entr(y/ies); their verdicts and \
         digests are kept"
    );
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].as_str())
}
