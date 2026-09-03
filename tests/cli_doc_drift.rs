//! The CLI is documented in three places that can drift apart independently.
//!
//! 1. the dispatch arms in `src/main.rs` — what the binary actually accepts
//! 2. `print_usage()` — what `--help` tells a user
//! 3. `docs/book/src/reference/cli-sharing.md` — the reference pages
//!
//! Checking the pages against `--help` alone is not enough, and that is not a
//! hypothetical: when this test was written, `--help` documented `share`,
//! `status`, `merge` and `unpack` but **not `import`** — the verb that receives
//! a share and the only one that verifies and quarantines. A pages-vs-help test
//! would have called the pages correct while the admission step stayed
//! undiscoverable. So all three are reconciled against the dispatch arms, which
//! are the only surface that cannot lie about what exists.
//!
//! The test reads sources rather than executing the binary on purpose: it must
//! run in the same cheap job as the rest of the suite, and a doc-drift failure
//! should not depend on a release build being available.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Verbs the binary actually dispatches, from the top-level `match` arms.
///
/// Multi-pattern arms (`"knot" | "load" =>`) contribute every alternative, which
/// a naive `"([a-z]+)" =>` scan silently drops — the first version of this
/// extraction missed `knot` for exactly that reason.
fn dispatch_verbs(main_rs: &str) -> BTreeSet<String> {
    let mut verbs = BTreeSet::new();
    for line in main_rs.lines() {
        // EXACTLY eight spaces: that is the top-level dispatch. Nested
        // subcommand matches (`ontology list|remove`) sit deeper and are not
        // top-level verbs; scanning every indentation level pulls them in and
        // makes this test demand `--help` entries for words that are not
        // commands.
        let Some(rest) = line.strip_prefix("        ") else {
            continue;
        };
        if rest.starts_with(' ') || !rest.starts_with('"') || !rest.contains("=>") {
            continue;
        }
        let head = rest.split("=>").next().unwrap_or("");
        for chunk in head.split('|') {
            let token = chunk.trim().trim_matches('"').trim();
            // A command never begins with a dash: the same arm that dispatches
            // `help` also dispatches `--help` and `-h`, and those belong under
            // OPTIONS, not COMMANDS.
            if !token.is_empty()
                && !token.starts_with('-')
                && token.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            {
                verbs.insert(token.to_string());
            }
        }
    }
    verbs
}

/// Verbs named in a block of documentation, as `quipu <verb>`.
fn documented_verbs(text: &str) -> BTreeSet<String> {
    let mut verbs = BTreeSet::new();
    for (index, _) in text.match_indices("quipu ") {
        let rest = &text[index + "quipu ".len()..];
        let token: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '-')
            .collect();
        if !token.is_empty() {
            verbs.insert(token);
        }
    }
    verbs
}

fn print_usage_body(main_rs: &str) -> String {
    let start = main_rs
        .find("fn print_usage")
        .expect("print_usage must exist — it is what --help emits");
    main_rs[start..].to_string()
}

/// `--help` lists these under OPTIONS/ALIASES rather than COMMANDS, so they
/// are dispatchable without being commands in the catalogue sense.
const NOT_COMMANDS: &[&str] = &["load", "query", "help"];

/// Verbs whose reference page lives elsewhere in the book. The sharing pages
/// own the share/import/pack family; the rest of the CLI is documented in
/// `reference/cli.md` and is out of scope for THIS test, which exists to keep
/// the sharing primitive discoverable.
const SHARING_VERBS: &[&str] = &[
    "share", "import", "status", "merge", "pack", "unpack", "knot",
];

#[test]
fn every_dispatchable_verb_is_in_help() {
    let main_rs = read("src/main.rs");
    let dispatch = dispatch_verbs(&main_rs);
    let help = documented_verbs(&print_usage_body(&main_rs));

    assert!(
        dispatch.len() > 20,
        "dispatch extraction found only {} verbs — the extractor is broken, not the CLI",
        dispatch.len()
    );

    let missing: Vec<_> = dispatch
        .iter()
        .filter(|v| !help.contains(*v) && !NOT_COMMANDS.contains(&v.as_str()))
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "these verbs dispatch but `--help` never mentions them, so a user cannot \
         discover them: {missing:?}. Add them to print_usage()."
    );
}

#[test]
fn help_does_not_document_verbs_that_do_not_exist() {
    let main_rs = read("src/main.rs");
    let dispatch = dispatch_verbs(&main_rs);
    let help = documented_verbs(&print_usage_body(&main_rs));

    // `--help` legitimately names subcommands (`quipu import promote`) and
    // option words; only flag tokens that look like a top-level verb and are
    // not one. A verb removed from dispatch but left in --help is the drift
    // this catches.
    let phantom: Vec<_> = help
        .iter()
        .filter(|v| !dispatch.contains(*v) && v.len() > 2)
        .cloned()
        .collect();

    assert!(
        phantom.is_empty(),
        "`--help` documents verbs the binary does not dispatch: {phantom:?}"
    );
}

#[test]
fn the_sharing_reference_page_covers_every_sharing_verb() {
    let page = read("docs/book/src/reference/cli-sharing.md");
    let documented = documented_verbs(&page);

    let missing: Vec<_> = SHARING_VERBS
        .iter()
        .filter(|v| !documented.contains(**v))
        .collect();

    assert!(
        missing.is_empty(),
        "the sharing CLI reference page does not document: {missing:?}"
    );
}

#[test]
fn the_sharing_reference_page_matches_the_help_text_flags() {
    let main_rs = read("src/main.rs");
    let help = print_usage_body(&main_rs);
    let page = read("docs/book/src/reference/cli-sharing.md");

    // For each sharing verb, every long flag `--help` advertises must appear on
    // the page. The reverse is not asserted: a page may explain a flag that
    // `--help` compresses away, and prose is allowed to be richer than usage.
    let mut missing: Vec<String> = Vec::new();
    for verb in SHARING_VERBS {
        let needle = format!("quipu {verb} ");
        for line in help.lines().filter(|l| l.contains(&needle)) {
            for token in line.split_whitespace() {
                let flag = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                if flag.starts_with("--") && flag.len() > 2 && !page.contains(flag) {
                    missing.push(format!("{verb}: {flag}"));
                }
            }
        }
    }
    missing.sort();
    missing.dedup();

    assert!(
        missing.is_empty(),
        "flags advertised by `--help` but absent from the sharing reference page: {missing:?}"
    );
}

// ── Citation rot ───────────────────────────────────────────────────────────
//
// The sharing docs cite proof as (`path`, `symbol`) pairs rather than line
// numbers, so they survive code moving around. They do NOT survive a RENAME,
// and that is not hypothetical: `aegis-iv3df7.5` renamed
// `share_import::verify_request` to `verify_share` within hours of the chapter
// publishing, leaving a public page citing a symbol that no longer existed.
// The claim stayed true and its proof pointer died — which is exactly how a
// page that nobody edits becomes false.
//
// These tests read the PAGES, so they cannot drift from what the pages
// actually claim.

/// Pages whose citations must resolve.
const CITING_PAGES: &[&str] = &[
    "docs/book/src/sharing/README.md",
    "docs/book/src/reference/cli-sharing.md",
];

/// Every `` `src/...` `` path mentioned in a page, and every `` `symbol` ``
/// that immediately follows it in the same parenthesised citation.
fn citations(page: &str) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    for (index, _) in page.match_indices("`src/") {
        let rest = &page[index + 1..];
        let Some(end) = rest.find('`') else { continue };
        let path = rest[..end].to_string();
        // A citation of the form (`path`, `symbol`) — take the symbol when the
        // very next backticked token follows a comma before the paren closes.
        let after = &rest[end + 1..];
        // A symbol, not another path: `(`a.rs`: `x`, `y`.)` and prose that
        // simply mentions two files in a row must not pair file-with-file.
        let symbol = after
            .strip_prefix(", `")
            .and_then(|tail| tail.find('`').map(|e| tail[..e].to_string()))
            .filter(|candidate| !candidate.contains('/'));
        found.push((path, symbol));
    }
    found
}

#[test]
fn every_cited_source_path_exists() {
    for page in CITING_PAGES {
        let text = read(page);
        let cited = citations(&text);
        assert!(
            !cited.is_empty(),
            "{page} cites no source files — the extractor is broken, not the page"
        );
        for (path, _) in cited {
            // A page may cite a module directory (`src/provider/`) as well as a
            // file; both are real proof pointers.
            let target = repo_root().join(&path);
            assert!(
                target.is_file() || target.is_dir(),
                "{page} cites `{path}`, which does not exist"
            );
        }
    }
}

#[test]
fn every_cited_symbol_still_resolves() {
    let mut broken: Vec<String> = Vec::new();
    for page in CITING_PAGES {
        let text = read(page);
        for (path, symbol) in citations(&text) {
            let Some(symbol) = symbol else { continue };
            let Ok(source) = std::fs::read_to_string(repo_root().join(&path)) else {
                continue; // the path test above owns this failure
            };
            if !source.contains(&symbol) {
                broken.push(format!("{page}: `{path}` no longer contains `{symbol}`"));
            }
        }
    }
    broken.sort();
    assert!(
        broken.is_empty(),
        "documentation cites symbols that have been renamed or removed. The claim may \
         still be true, but its proof is dead — fix the citation or the code: {broken:#?}"
    );
}
