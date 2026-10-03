//! `quipu share diff` and `quipu diff-textconv` (aegis-fxpbys.1).
//!
//! Both are store-free: they read pack files and never load configuration or
//! open a database, so `git diff` can call the textconv from any checkout.
use std::path::Path;

use quipu::share_diff::{
    Snapshot, diff, read_payload, render_markdown, render_text, render_textconv,
};

const DIFF_USAGE: &str = "usage: quipu share diff <old> <new> [--format text|markdown|json]";

fn load(path: &str) -> Snapshot {
    match read_payload(Path::new(path)) {
        Ok(quads) => Snapshot::new(&quads),
        Err(e) => {
            eprintln!("share diff: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu share diff <old> <new> [--format text|markdown|json]`.
pub fn cmd_diff(args: &[String]) {
    let mut paths = Vec::new();
    let mut format = "text".to_string();
    let mut rest = args[3..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--format" => match rest.next() {
                Some(f) => format = f.clone(),
                None => {
                    eprintln!("{DIFF_USAGE}");
                    std::process::exit(1);
                }
            },
            _ => paths.push(arg.clone()),
        }
    }
    let [old, new] = paths.as_slice() else {
        eprintln!("{DIFF_USAGE}");
        std::process::exit(1);
    };
    let d = diff(&load(old), &load(new));
    match format.as_str() {
        "text" => print!("{}", render_text(&d)),
        "markdown" => print!("{}", render_markdown(&d)),
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&d).expect("diff serializes")
        ),
        other => {
            eprintln!("share diff: unknown --format {other:?} (text|markdown|json)");
            std::process::exit(1);
        }
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
