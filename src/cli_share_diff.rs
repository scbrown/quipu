//! `quipu share diff` and `quipu diff-textconv` (aegis-fxpbys.1).
//!
//! Both are store-free: they read pack files and never load configuration or
//! open a database, so `git diff` can call the textconv from any checkout.
use std::path::{Path, PathBuf};

use oxrdf::Quad;
use quipu::git_merge::Decisions;
use quipu::share_diff::{
    Snapshot, diff, read_payload, render_markdown, render_text, render_textconv,
};
use quipu::share_review::{ReviewInput, ShaclReview, render_report_markdown, review};

const DIFF_USAGE: &str = "usage: quipu share diff <old> <new> [--format text|markdown|json]\n       quipu share diff <old> <new> --report [--format markdown|json] [--old-shapes <ttl>] [--new-shapes <ttl>] [--decisions <json>] [--fail-on-introduced]";

fn usage() -> ! {
    eprintln!("{DIFF_USAGE}");
    std::process::exit(1);
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("share diff: {msg}");
    std::process::exit(1);
}

fn quads(path: &str) -> Vec<Quad> {
    read_payload(Path::new(path)).unwrap_or_else(|e| fail(e))
}

fn load(path: &str) -> Snapshot {
    Snapshot::new(&quads(path))
}

/// An explicit file, else `<dir>/<name>` when the side is a pack directory.
fn beside(explicit: Option<&String>, side: &str, name: &str) -> Option<String> {
    let path = match explicit {
        Some(p) => PathBuf::from(p),
        None => Path::new(side).join(name),
    };
    if explicit.is_none() && !path.is_file() {
        return None;
    }
    Some(
        std::fs::read_to_string(&path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display()))),
    )
}

#[derive(Default)]
struct Args {
    paths: Vec<String>,
    format: Option<String>,
    report: bool,
    fail_on_introduced: bool,
    old_shapes: Option<String>,
    new_shapes: Option<String>,
    decisions: Option<String>,
}

fn parse(args: &[String]) -> Args {
    let mut a = Args::default();
    let mut rest = args[3..].iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().cloned().unwrap_or_else(|| usage());
        match arg.as_str() {
            "--format" => a.format = Some(value()),
            "--old-shapes" => a.old_shapes = Some(value()),
            "--new-shapes" => a.new_shapes = Some(value()),
            "--decisions" => a.decisions = Some(value()),
            "--report" => a.report = true,
            "--fail-on-introduced" => a.fail_on_introduced = true,
            flag if flag.starts_with("--") => usage(),
            _ => a.paths.push(arg.clone()),
        }
    }
    let report_only = a.fail_on_introduced
        || a.old_shapes.is_some()
        || a.new_shapes.is_some()
        || a.decisions.is_some();
    if a.paths.len() != 2 || (report_only && !a.report) {
        usage();
    }
    a
}

/// `quipu share diff <old> <new> [--format text|markdown|json]`, and with
/// `--report` the PR-review report (aegis-fxpbys.1 M2). Exit 3 means
/// `--fail-on-introduced` saw introduced SHACL violations; the report is still
/// printed in full first.
pub fn cmd_diff(args: &[String]) {
    let a = parse(args);
    let (old, new) = (&a.paths[0], &a.paths[1]);
    if a.report {
        return report(&a, old, new);
    }
    let d = diff(&load(old), &load(new));
    match a.format.as_deref().unwrap_or("text") {
        "text" => print!("{}", render_text(&d)),
        "markdown" => print!("{}", render_markdown(&d)),
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&d).expect("diff serializes")
        ),
        other => fail(format!("unknown --format {other:?} (text|markdown|json)")),
    }
}

fn report(a: &Args, old: &str, new: &str) {
    let (old_quads, new_quads) = (quads(old), quads(new));
    let old_shapes = beside(a.old_shapes.as_ref(), old, "shapes.ttl");
    let new_shapes = beside(a.new_shapes.as_ref(), new, "shapes.ttl");
    let decisions: Option<Decisions> = beside(a.decisions.as_ref(), new, "decisions.json")
        .map(|d| serde_json::from_str(&d).unwrap_or_else(|e| fail(format!("decisions.json: {e}"))));
    let r = review(&ReviewInput {
        old: &old_quads,
        new: &new_quads,
        old_shapes: old_shapes.as_deref(),
        new_shapes: new_shapes.as_deref(),
        decisions: decisions.as_ref(),
    })
    .unwrap_or_else(|e| fail(format!("cannot evaluate the report: {e}")));
    match a.format.as_deref().unwrap_or("markdown") {
        "markdown" => print!("{}", render_report_markdown(&r)),
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&r).expect("review serializes")
        ),
        other => fail(format!(
            "--report supports --format markdown|json, not {other:?}"
        )),
    }
    if !a.fail_on_introduced {
        return;
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
    match (&r.shacl, r.shacl.introduced()) {
        (
            ShaclReview::NotChecked {
                gate_must_fail: true,
                reason,
            },
            _,
        ) => fail(format!(
            "--fail-on-introduced cannot be evaluated: {reason}"
        )),
        (_, Some(n)) if n > 0 => {
            eprintln!("share diff: {n} introduced SHACL violation(s)");
            std::process::exit(3);
        }
        _ => {}
    }
}

/// `quipu diff-textconv <file>`: one pack payload as labelled, entity-grouped
/// text for `git diff`. A file that does not parse (a conflicted working copy,
/// say) is passed through unchanged, so `git diff` still works on it.
pub fn cmd_textconv(args: &[String]) {
    let Some(path) = args.get(2) else {
        eprintln!("usage: quipu diff-textconv <file>");
        std::process::exit(1);
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("diff-textconv: {path}: {e}");
            std::process::exit(1);
        }
    };
    let out = match quipu::share_diff::parse_payload(&bytes, path) {
        Ok(quads) => render_textconv(&Snapshot::new(&quads)).into_bytes(),
        Err(e) => {
            eprintln!("diff-textconv: {e}; showing the file unchanged");
            bytes
        }
    };
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(&out).and_then(|()| stdout.flush())
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("diff-textconv: {e}");
        std::process::exit(1);
    }
}
