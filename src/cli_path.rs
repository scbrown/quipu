//! `quipu path` — golden-path analysis: cone, backtest, draft.
//!
//! All three are reads; `draft` prints Turtle for a human to review and load.
//! See `docs/design/golden-paths-blessing.md`.

use quipu::path::{ConeOptions, ConeVerdict, DraftOptions, PathVocab, backtest, cone, draft};
use quipu::store::Store;

fn open(db_path: &str) -> Store {
    crate::cli_open::open_store(db_path)
}

fn die(usage: &str) -> ! {
    eprintln!("{usage}");
    std::process::exit(1);
}

const USAGE: &str = "usage: quipu path <cone|backtest|draft> <trajectory-IRI> [options]
  cone     --via <predicate-IRI>...  --hops N  [--json]
  backtest --omit <step-IRI>...  [--json]
  draft    --name <local-name> --label <text>
           --via <predicate-IRI>...  [--omit <step-IRI> --by <decision-IRI>]...
           [--dead-end <step-IRI>]...";

/// Dispatch `quipu path <sub>`.
pub fn cmd_path(args: &[String], db_path: &str, base_ns: &str) {
    let sub = args.get(2).map_or("", String::as_str);
    let Some(trajectory) = args.get(3).filter(|a| !a.starts_with("--")) else {
        die(USAGE);
    };
    let vocab = PathVocab::new(base_ns);
    let rest = &args[4..];
    let json = rest.iter().any(|a| a == "--json");

    match sub {
        "cone" => {
            let opts = ConeOptions {
                via: flag_values(rest, "--via"),
                hops: flag_value(rest, "--hops")
                    .map_or(quipu::path::cone::DEFAULT_CONE_HOPS, |h| {
                        h.parse().unwrap_or_else(|_| die(USAGE))
                    }),
            };
            let store = open(db_path);
            match cone(&store, trajectory, &vocab, &opts) {
                Ok(report) if json => print_json(&report),
                Ok(report) => {
                    println!("cone of {} (hops: {})", report.trajectory, report.hops);
                    println!("verified against: {}", report.verifications.join(", "));
                    for s in &report.steps {
                        let v = match s.verdict {
                            ConeVerdict::InCone => "IN-CONE       ",
                            ConeVerdict::OutOfCone => "OUT-OF-CONE   ",
                            ConeVerdict::CannotEvaluate => "CANNOT-EVALUATE",
                        };
                        let order = s.order.map_or(String::from("-"), |o| o.to_string());
                        println!("  [{order}] {v} {} — {}", s.iri, s.reason);
                    }
                }
                Err(e) => fail(&e),
            }
        }
        "backtest" => {
            let omit = flag_values(rest, "--omit");
            let store = open(db_path);
            match backtest(&store, trajectory, &omit, &vocab) {
                Ok(report) if json => print_json(&report),
                Ok(report) => {
                    println!(
                        "backtest of {} under {} (topics: {})",
                        report.exemplar,
                        report.grammar,
                        report.topics.join(", ")
                    );
                    for r in &report.rows {
                        println!(
                            "  {} -> {:?} (outcome: {})",
                            r.trajectory,
                            r.result,
                            r.outcome.as_deref().unwrap_or("open")
                        );
                    }
                    println!(
                        "conformers {}/{} done; deviators {}/{} done; cannot evaluate: {}",
                        report.conformers_done,
                        report.conformers_total,
                        report.deviators_done,
                        report.deviators_total,
                        report.cannot_evaluate
                    );
                }
                Err(e) => fail(&e),
            }
        }
        "draft" => {
            let name = flag_value(rest, "--name").unwrap_or_else(|| die(USAGE));
            let label = flag_value(rest, "--label").unwrap_or_else(|| die(USAGE));
            let omissions = flag_values(rest, "--omit");
            let rulings = flag_values(rest, "--by");
            if omissions.len() != rulings.len() {
                die(
                    "every --omit needs its --by <decision-IRI>: a human cut without its Decision is a silent edit of history",
                );
            }
            let opts = ConeOptions {
                via: flag_values(rest, "--via"),
                hops: quipu::path::cone::DEFAULT_CONE_HOPS,
            };
            let store = open(db_path);
            let report = match cone(&store, trajectory, &vocab, &opts) {
                Ok(r) => r,
                Err(e) => fail(&e),
            };
            let draft_opts = DraftOptions {
                name,
                label,
                human_omissions: omissions.into_iter().zip(rulings).collect(),
                dead_ends: flag_values(rest, "--dead-end"),
                base_ns: base_ns.to_string(),
            };
            match draft(&report, &draft_opts) {
                Ok(ttl) => print!("{ttl}"),
                Err(e) => fail(&e),
            }
        }
        _ => die(USAGE),
    }
}

fn fail(e: &quipu::error::Error) -> ! {
    eprintln!("error: {e}");
    std::process::exit(1);
}

fn print_json<T: serde::Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serializable report")
    );
}

/// All values following occurrences of `flag`.
fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == flag
            && let Some(v) = it.next()
        {
            out.push(v.clone());
        }
    }
    out
}

/// The value following the first occurrence of `flag`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}
