//! `quipu audit <trace.jsonl>` — the `T ⊨ Σ` checker, invocable.
//!
//! A checker with no way to run it is a library nobody calls, and "we have an
//! audit checker" would then be a claim about the repository rather than about
//! the deployment. This is the surface that makes it a control.
//!
//! **The exit code is the point.** `0` when the trace conforms, `1` when it does
//! not — so a CI job can gate on it without parsing anything. Incompleteness
//! never changes the exit code: it is reported, and it is not a contradiction.
//! A checker that failed the build over a missing `planner` would be switched
//! off within a week, and then the violations would stop being caught too.

use quipu::governance::audit::{self, Report, Severity};
use quipu::governance::inventory;
use quipu::governance::replay;

/// Run a checker: `audit <trace.jsonl>` against a trace, `audit inventory`
/// against the dispatch graph, `audit replay <trace.jsonl>` for promotion
/// readiness.
pub fn cmd_audit(args: &[String], db_path: &str) {
    let Some(subject) = args.get(2).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: quipu audit <trace.jsonl>|inventory|replay <trace.jsonl> \
             [--json] [--db <path>]"
        );
        std::process::exit(1);
    };
    let store = quipu::Store::open(db_path).unwrap_or_else(|e| {
        eprintln!("error opening store: {e}");
        std::process::exit(1);
    });

    if subject == "replay" {
        cmd_replay(args, &store);
        return;
    }

    let report = if subject == "inventory" {
        inventory::check(&store).unwrap_or_else(|e| {
            eprintln!("error checking inventory: {e}");
            std::process::exit(1);
        })
    } else {
        let jsonl = std::fs::read_to_string(subject).unwrap_or_else(|e| {
            eprintln!("error reading {subject}: {e}");
            std::process::exit(1);
        });
        audit::check_jsonl(&store, &jsonl).unwrap_or_else(|e| {
            eprintln!("error checking trace: {e}");
            std::process::exit(1);
        })
    };

    let headline = if subject == "inventory" {
        inventory::summary(&report)
    } else {
        report.summary()
    };
    if args.iter().any(|a| a == "--json") {
        println!("{}", as_json(&report, &headline));
    } else {
        print_report(&report, &headline);
    }
    // Only a contradiction fails the gate. See the module doc.
    if !report.conforms() {
        std::process::exit(1);
    }
}

fn print_report(report: &Report, headline: &str) {
    println!("{headline}");
    for severity in [Severity::Violation, Severity::Incompleteness] {
        let findings = report.of(severity);
        if findings.is_empty() {
            continue;
        }
        let label = match severity {
            Severity::Violation => "VIOLATION",
            Severity::Incompleteness => "incomplete",
        };
        println!();
        for finding in findings {
            // The record index leads when there is one, because the first thing
            // a reader does with a trace finding is go back to the line it came
            // from. A whole-window or inventory finding has no line to go back
            // to, and printing a placeholder there would send them looking.
            let where_ = finding
                .record
                .map_or(String::new(), |i| format!(" record {i}"));
            let about = finding
                .constraint
                .as_deref()
                .map_or(String::new(), |c| format!(" [{c}]"));
            println!(
                "{label} {pass}{where_}{about}: {detail}",
                pass = finding.pass.as_str(),
                detail = finding.detail
            );
        }
    }
}

fn as_json(report: &Report, headline: &str) -> String {
    let findings: Vec<serde_json::Value> = report
        .discrepancies
        .iter()
        .map(|d| {
            serde_json::json!({
                "pass": d.pass.as_str(),
                "severity": match d.severity {
                    Severity::Violation => "violation",
                    Severity::Incompleteness => "incompleteness",
                },
                "record": d.record,
                "constraint": d.constraint,
                "detail": d.detail,
            })
        })
        .collect();
    serde_json::json!({
        "conforms": report.conforms(),
        "complete": report.is_complete(),
        "records_checked": report.records_checked,
        "records_unreadable": report.records_unreadable,
        "constraints_in_scope": report.constraints_in_scope,
        "summary": headline,
        "findings": findings,
    })
    .to_string()
}

/// `quipu audit replay <trace.jsonl>` — what a recorded window says about
/// promoting each rule from advise to enforce.
///
/// Exits 0 whatever it finds. Replay reports readiness, and readiness is a
/// judgement an operator makes: failing a build because a rule has not yet
/// fired would turn "we have not measured this" into "this is broken", which
/// are different states and need different responses.
fn cmd_replay(args: &[String], store: &quipu::Store) {
    let Some(path) = args.get(3).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: quipu audit replay <trace.jsonl> [--json] [--db <path>]");
        std::process::exit(1);
    };
    let jsonl = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error reading {path}: {e}");
        std::process::exit(1);
    });
    let report = replay::replay_jsonl(store, &jsonl).unwrap_or_else(|e| {
        eprintln!("error replaying trace: {e}");
        std::process::exit(1);
    });

    if args.iter().any(|a| a == "--json") {
        let rules: Vec<serde_json::Value> = report
            .rules
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "in_spec": r.in_spec,
                    "evaluated": r.evaluated,
                    "satisfied": r.satisfied,
                    "unsatisfied": r.unsatisfied,
                    "blocked": r.blocked,
                    "advisory": r.advisory,
                    "would_block": r.would_block,
                    "new_blocks": r.new_blocks(),
                    "targets": r.targets,
                    "recovered": r.recovered,
                    "blocker": r.blocker(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "summary": report.summary(),
                "records": report.records,
                "unreadable": report.unreadable,
                "rules": rules,
            })
        );
        return;
    }
    println!("{}", report.summary());
    if report.rules.is_empty() {
        return;
    }
    println!();
    for rule in &report.rules {
        println!("{}", rule.line());
    }
}
