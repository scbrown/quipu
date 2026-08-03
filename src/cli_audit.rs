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

/// Run the checker over a trace file.
pub fn cmd_audit(args: &[String], db_path: &str) {
    let Some(trace_path) = args.get(2).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: quipu audit <trace.jsonl> [--json] [--db <path>]");
        std::process::exit(1);
    };
    let jsonl = std::fs::read_to_string(trace_path).unwrap_or_else(|e| {
        eprintln!("error reading {trace_path}: {e}");
        std::process::exit(1);
    });
    let store = quipu::Store::open(db_path).unwrap_or_else(|e| {
        eprintln!("error opening store: {e}");
        std::process::exit(1);
    });
    let report = audit::check_jsonl(&store, &jsonl).unwrap_or_else(|e| {
        eprintln!("error checking trace: {e}");
        std::process::exit(1);
    });

    if args.iter().any(|a| a == "--json") {
        println!("{}", as_json(&report));
    } else {
        print_report(&report);
    }
    // Only a contradiction fails the gate. See the module doc.
    if !report.conforms() {
        std::process::exit(1);
    }
}

fn print_report(report: &Report) {
    println!("{}", report.summary());
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
            // The record index leads, because the first thing a reader does with
            // a finding is go back to the line it came from.
            let where_ = finding
                .record
                .map_or_else(|| "window".to_string(), |i| format!("record {i}"));
            let about = finding
                .constraint
                .as_deref()
                .map_or(String::new(), |c| format!(" [{c}]"));
            println!(
                "{label} {pass} {where_}{about}: {detail}",
                pass = finding.pass.as_str(),
                detail = finding.detail
            );
        }
    }
}

fn as_json(report: &Report) -> String {
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
        "summary": report.summary(),
        "findings": findings,
    })
    .to_string()
}
