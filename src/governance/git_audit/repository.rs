//! Read-only Git plumbing; no worktree contents, hooks or external diff drivers.
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use super::invalid;
use crate::error::Result;

pub(super) struct Window {
    pub from: String,
    pub to: String,
    pub commits: Vec<String>,
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_SHALLOW_FILE")
        .env_remove("GIT_GRAFT_FILE")
        .arg("--no-replace-objects")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| invalid(format!("cannot run git: {e}")))?;
    if !output.status.success() {
        return Err(invalid(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

fn text(repo: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(git(repo, args)?)
        .map_err(|_| invalid("non-UTF-8 Git metadata/path; coverage unproven"))
}

fn resolve(repo: &Path, reference: &str) -> Result<String> {
    Ok(text(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )?
    .trim()
    .into())
}

pub(super) fn window(repo: &Path, from: &str, to: &str) -> Result<Window> {
    if text(repo, &["rev-parse", "--is-shallow-repository"])?.trim() != "false" {
        return Err(invalid(
            "shallow repository: fetch full history before auditing",
        ));
    }
    if !text(repo, &["rev-parse", "--show-prefix"])?
        .trim()
        .is_empty()
    {
        return Err(invalid("--repo must name the repository root"));
    }
    let grafts = text(repo, &["rev-parse", "--git-path", "info/grafts"])?;
    let grafts = repo.join(grafts.trim());
    match std::fs::read(&grafts) {
        Ok(bytes) if !bytes.is_empty() => {
            return Err(invalid("Git grafts rewrite ancestry; coverage is unproven"));
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(invalid(format!("cannot inspect Git grafts: {e}")));
        }
        _ => {}
    }
    let from = resolve(repo, from)?;
    let to = resolve(repo, to)?;
    git(repo, &["merge-base", "--is-ancestor", &from, &to])?;
    let commits = text(
        repo,
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            &format!("{from}..{to}"),
            "--",
        ],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    Ok(Window { from, to, commits })
}

pub(super) fn paths(repo: &Path, commit: &str) -> Result<BTreeSet<String>> {
    // -m compares merges to every parent: neither combined-diff nor a net
    // window diff can see every crossing. --no-renames preserves both names.
    let output = text(
        repo,
        &[
            "diff-tree",
            "--root",
            "-m",
            "-r",
            "--no-commit-id",
            "--name-only",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            commit,
            "--",
        ],
    )?;
    Ok(output
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

pub(super) fn attribution(repo: &Path, commit: &str) -> Result<String> {
    let meta = text(
        repo,
        &["show", "-s", "--format=%an <%ae>%n%B", commit, "--"],
    )?;
    let mut lines = meta.lines();
    let author = lines.next().unwrap_or("unknown author");
    let trailers: Vec<&str> = lines
        .filter(|line| {
            line.split_once(':').is_some_and(|(key, _)| {
                ["Co-Authored-By", "Claude-Session", "SHANTY_AGENT"]
                    .iter()
                    .any(|k| key.eq_ignore_ascii_case(k))
            })
        })
        .collect();
    Ok(format!(
        "author {author:?}, declared trailers {trailers:?} (unauthenticated)"
    ))
}
