//! CLI command: `quipu policy` — the policy-by-example gesture, quipu side.
//!
//! `draft` emits placement-aimed advisory Turtle from an exemplar + intent
//! (docs/design/policy-by-example.md step 1); `backtest` replays a candidate
//! over the store's recorded history BEFORE anything is created (step 3). The
//! ordering is the design: draft, backtest, read the hit list, and only then
//! `quipu knot` the file — at which point the placement check still runs and
//! can still refuse.

use quipu::governance::backtest::{self, Candidate, Window};
use quipu::governance::draft::{DraftIntent, draft_turtle};

use crate::cli::flag_value;

pub fn cmd_policy(args: &[String], db_path: &str) {
    match args.get(2).map(String::as_str) {
        Some("draft") => cmd_draft(args),
        Some("backtest") => cmd_backtest(args, db_path),
        _ => {
            eprintln!(
                "usage: quipu policy draft --exemplar <iri> --name <slug> --label <sentence> \
                 --targets <type-iri> --claim <ask> [--class soft|hard] [--point <point>] \
                 [--layer <layer>] [--authority <who>] [--out <file.ttl>]\n\
                 \x20      quipu policy backtest <candidate.ttl> [--last-txs N] \
                 [--from-tx A --to-tx B] [--db <path>]"
            );
            std::process::exit(1);
        }
    }
}

/// `quipu policy draft` — a filled-in form out, never a store write. The
/// human edits the emitted Turtle, backtests it, and knots it deliberately.
fn cmd_draft(args: &[String]) {
    let required = |name: &str| {
        flag_value(args, name).unwrap_or_else(|| {
            eprintln!(
                "{name} is required: quipu policy draft --exemplar <iri> --name <slug> \
                 --label <sentence> --targets <type-iri> --claim <ask> [--class soft|hard] \
                 [--point <point>] [--layer <layer>] [--authority <who>] [--out <file.ttl>]"
            );
            std::process::exit(1);
        })
    };
    let intent = DraftIntent {
        exemplar: required("--exemplar").to_string(),
        name: required("--name").to_string(),
        label: required("--label").to_string(),
        target_type_iri: required("--targets").to_string(),
        claim: required("--claim").to_string(),
        class: flag_value(args, "--class").map(str::to_string),
        point: flag_value(args, "--point").map(str::to_string),
        layer: flag_value(args, "--layer").map(str::to_string),
        authority: flag_value(args, "--authority").map(str::to_string),
    };
    let turtle = match draft_turtle(&intent) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("draft refused: {e}");
            std::process::exit(1);
        }
    };
    match flag_value(args, "--out") {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &turtle) {
                eprintln!("error writing {path}: {e}");
                std::process::exit(1);
            }
            println!("drafted {} -> {path}", intent.policy_iri());
        }
        None => print!("{turtle}"),
    }
    // The next move is printed rather than assumed: the gesture's safety is
    // backtest-before-birth, and the CLI is where that ordering is taught.
    eprintln!(
        "# born advisory (effect \"warn\"). Before creating it, see what it \
         would have done:\n#   quipu policy backtest <file.ttl> --last-txs 500\n\
         # then ingest deliberately (placement validation still applies):\n\
         #   quipu knot <file.ttl>"
    );
}

/// `quipu policy backtest <candidate.ttl>` — the hit list, printed honestly.
fn cmd_backtest(args: &[String], db_path: &str) {
    let Some(path) = args.get(3).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: quipu policy backtest <candidate.ttl> [--last-txs N] \
             [--from-tx A --to-tx B] [--db <path>]"
        );
        std::process::exit(1);
    };
    let turtle = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            std::process::exit(1);
        }
    };
    let candidate = match Candidate::from_turtle(&turtle) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };
    let int_flag = |name: &str| flag_value(args, name).and_then(|v| v.parse::<i64>().ok());
    let window = match (int_flag("--from-tx"), int_flag("--to-tx")) {
        (Some(from_tx), Some(to_tx)) => Window { from_tx, to_tx },
        (None, None) => {
            // Default: everything the store remembers. An accidental narrow
            // default would under-report the FP surface at the exact moment it
            // is being relied on.
            let n = int_flag("--last-txs").unwrap_or(i64::MAX);
            match Window::last(&store, n) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("error reading transaction log: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("--from-tx and --to-tx must be given together");
            std::process::exit(1);
        }
    };

    let report = match backtest::backtest(&store, &candidate, &window) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error backtesting: {e}");
            std::process::exit(1);
        }
    };
    for hit in &report.hits {
        println!("{}", hit.line());
    }
    if !report.hits.is_empty() {
        println!();
    }
    println!("{}", report.summary());
    // Exit 1 when nothing was measured: a script that knots the draft on
    // success must not read "cannot evaluate" as "clean".
    if report.unevaluable.is_some() {
        std::process::exit(1);
    }
}
