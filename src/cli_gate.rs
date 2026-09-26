//! CLI command: `quipu gate shadow` — judge a candidate policy set over
//! recorded history, on a quiescent copy, never writing (aegis-xfuch4.2).

use quipu::governance::backtest::Window;
use quipu::governance::shadow::{self, Candidate, Mode, Options};
use quipu::governance::shadow_io::open_quiescent_copy;

use crate::cli::flag_value;

const USAGE: &str = "usage: quipu gate shadow --rules <candidate.ttl> --db <quiescent-copy.db> \
     [--since 90m|24h|7d | --from-tx A --to-tx B | --last-txs N] [--max-txs N] \
     [--mode add|replace] [--json]";

pub fn cmd_gate(args: &[String], live_db: &str) {
    match args.get(2).map(String::as_str) {
        Some("shadow") => cmd_shadow(args, live_db),
        _ => fail(USAGE),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// `live_db` is the store the CONFIGURATION points at, resolved without the
/// `--db` override: the shadow refuses it, so it must be known independently.
fn cmd_shadow(args: &[String], live_db: &str) {
    let Some(rules) = flag_value(args, "--rules") else {
        fail(USAGE)
    };
    let Some(db) = flag_value(args, "--db") else {
        fail(&format!(
            "--db <copy> is required: the shadow gate never runs on the configured \
             store ({live_db}).\n{USAGE}"
        ))
    };
    let turtle = std::fs::read_to_string(rules)
        .unwrap_or_else(|e| fail(&format!("error reading {rules}: {e}")));
    let candidate =
        Candidate::from_turtle(&turtle).unwrap_or_else(|e| fail(&format!("error: {e}")));
    let store = open_quiescent_copy(
        std::path::Path::new(db),
        Some(std::path::Path::new(live_db)),
    )
    .unwrap_or_else(|e| fail(&format!("refused: {e}")));

    let int = |name: &str| {
        flag_value(args, name).map(|v| {
            v.parse::<i64>()
                .unwrap_or_else(|_| fail(&format!("{name} {v}: not an integer")))
        })
    };
    let window = match (
        flag_value(args, "--since"),
        int("--from-tx"),
        int("--to-tx"),
    ) {
        (Some(since), None, None) => shadow::parse_duration(since)
            .and_then(|secs| shadow::window_since(&store, secs))
            .unwrap_or_else(|e| fail(&format!("error: {e}"))),
        (None, Some(from_tx), Some(to_tx)) => Window { from_tx, to_tx },
        (None, None, None) => Window::last(&store, int("--last-txs").unwrap_or(i64::MAX))
            .unwrap_or_else(|e| fail(&format!("error reading transaction log: {e}"))),
        _ => fail("give --since, or --from-tx with --to-tx, or --last-txs; not a mix"),
    };
    let mode = match flag_value(args, "--mode").unwrap_or("add") {
        "add" => Mode::Add,
        "replace" => Mode::Replace,
        other => fail(&format!("--mode {other}: expected add or replace")),
    };
    let max_txs = int("--max-txs").map(|n| usize::try_from(n.max(0)).unwrap_or(0));
    let opts = Options {
        window,
        max_txs,
        mode,
    };

    let clock = quipu::time::Stopwatch::start();
    let report =
        shadow::run(&store, &candidate, &opts).unwrap_or_else(|e| fail(&format!("error: {e}")));
    let elapsed = clock.elapsed_ms();
    if args.iter().any(|a| a == "--json") {
        let mut j = report.to_json();
        j["mode"] = mode.as_str().into();
        j["candidate_policies"] = candidate.policy_iris().into();
        j["elapsed_ms"] = u64::try_from(elapsed).unwrap_or(u64::MAX).into();
        println!("{}", serde_json::to_string_pretty(&j).unwrap_or_default());
    } else {
        print!("{}", report.render());
        println!(
            "\nmode {}; candidate: {}; {elapsed} ms.",
            mode.as_str(),
            candidate.policy_iris().join(", ")
        );
    }
}
