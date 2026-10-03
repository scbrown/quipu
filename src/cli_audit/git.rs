//! Opt-in Git inputs for the trace checker.
use quipu::governance::{
    audit::{Report, TraceRecord},
    git_audit::{self, Scope},
};

pub(super) struct Window {
    repo: String,
    from: String,
    to: String,
    yupana: Option<String>,
}

pub(super) fn options(args: &[String], subject: &str) -> Result<Option<Window>, String> {
    let flags = ["--repo", "--from", "--to", "--yupana"];
    if !args.iter().any(|a| flags.contains(&a.as_str())) {
        return Ok(None);
    }
    if ["inventory", "namespace", "replay", "tree", "inheritance"].contains(&subject) {
        return Err("Git flags require audit <trace.jsonl>".into());
    }
    let value = |flag: &str| -> Result<String, String> {
        let positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == flag)
            .map(|(i, _)| i)
            .collect();
        if positions.len() != 1 {
            return Err(format!("supply {flag} exactly once"));
        }
        args.get(positions[0] + 1)
            .filter(|v| !v.starts_with("--"))
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    Ok(Some(Window {
        repo: value("--repo")?,
        from: value("--from")?,
        to: value("--to")?,
        yupana: if args.iter().any(|a| a == "--yupana") {
            Some(value("--yupana")?)
        } else {
            None
        },
    }))
}

pub(super) fn check(
    trace: &[TraceRecord],
    unreadable: usize,
    store: &quipu::Store,
    window: &Window,
    report: &mut Report,
) -> Scope {
    let mut run = || -> quipu::error::Result<Scope> {
        if unreadable > 0 {
            return Err(quipu::error::Error::CannotVerify(format!(
                "{unreadable} unreadable trace lines"
            )));
        }
        git_audit::reconcile_with_yupana(
            store,
            trace,
            std::path::Path::new(&window.repo),
            &window.from,
            &window.to,
            window.yupana.as_deref().map(std::path::Path::new),
            report,
        )
    };
    run().unwrap_or_else(|e| {
        eprintln!("cannot verify Git coverage: {e}");
        std::process::exit(2);
    })
}

pub(super) fn emit(args: &[String], report: &Report, headline: &str, scope: Option<&Scope>) {
    let Some(scope) = scope else {
        super::emit(args, report, headline);
        return;
    };
    if args.iter().any(|a| a == "--json") {
        let mut value: serde_json::Value =
            serde_json::from_str(&super::as_json(report, headline)).expect("audit JSON");
        value["git"] = serde_json::to_value(scope).expect("scope JSON");
        println!("{value}");
    } else {
        super::print_report(report, headline);
        println!(
            "Git {}..{}: {} commits, {} changed paths, {} policies, {} selector evaluations; {} unresolved",
            scope.from,
            scope.to,
            scope.commits_checked,
            scope.paths_checked,
            scope.policies_checked,
            scope.selectors_checked,
            scope.unresolved
        );
    }
}
