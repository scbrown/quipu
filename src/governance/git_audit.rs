//! Post-action path-policy coverage over an explicit, immutable Git window.
//!
//! This is an opt-in backstop, not a replacement for the pre-edit guard. It
//! checks ROOT's current policy catalogue against every commit in `(from, to]`.
//! Evidence must name the full commit id and exact repository-relative path;
//! an older evaluation of the same path cannot cover another commit. Neither
//! commit authors nor caller-supplied trace records are authenticated here.

use std::path::Path;

use crate::error::{Error, Result};
use crate::store::Store;

use super::audit::{Discrepancy, Pass, Report, Severity, TraceRecord};

mod policies;
mod repository;

/// Measured scope of the Git pass, separate from trace-record counts.
#[derive(Debug, Default, serde::Serialize)]
pub struct Scope {
    /// Immutable exclusive lower bound.
    pub from: String,
    /// Immutable inclusive upper bound.
    pub to: String,
    /// Commits actually enumerated, including side-branch commits.
    pub commits_checked: usize,
    /// Changed paths enumerated per commit (deduplicated across merge parents).
    pub paths_checked: usize,
    /// Path policies loaded from ROOT.
    pub policies_checked: usize,
    /// Coverage the path-only implementation could not establish.
    pub unresolved: usize,
}

/// Append Git findings to the existing trace audit report.
///
/// Git errors, shallow history, malformed globs and unresolvable refs return
/// errors, never an empty successful scan. Selector/predicate policies are
/// reported as unresolved: this pass cannot replay their claim yet.
///
/// # Errors
/// Inaccessible or incomplete repository, malformed policy, or store errors.
pub fn reconcile(
    store: &Store,
    trace: &[TraceRecord],
    repo: &Path,
    from: &str,
    to: &str,
    report: &mut Report,
) -> Result<Scope> {
    let window = repository::window(repo, from, to)?;
    let policies = policies::load(store)?;
    let mut scope = Scope {
        from: window.from,
        to: window.to,
        policies_checked: policies.len(),
        ..Scope::default()
    };
    if policies.is_empty() {
        unresolved(
            report,
            &mut scope,
            None,
            "no path policies loaded from ROOT; coverage is unproven".into(),
        );
    }
    for commit in window.commits {
        scope.commits_checked += 1;
        let paths = repository::paths(repo, &commit)?;
        scope.paths_checked += paths.len();
        let attribution = repository::attribution(repo, &commit)?;
        for path in paths {
            for policy in &policies {
                if !policy.globs.iter().any(|g| g.matches(&path)) {
                    continue;
                }
                let context = format!("commit {commit}, path {path:?}, {attribution}");
                let matched = trace.iter().any(|r| {
                    r.git_commit.as_deref() == Some(commit.as_str())
                        && r.path.as_deref() == Some(path.as_str())
                        && r.constraints.iter().any(|e| {
                            (e.id == policy.iri || e.id == policy.id)
                                && matches!(e.outcome.as_deref(), Some("satisfied" | "unsatisfied"))
                                && matches!(
                                    e.response.as_deref(),
                                    Some(
                                        "blocked" | "warned" | "logged" | "escalated" | "no-action"
                                    )
                                )
                        })
                });
                if !matched {
                    finding(
                        report,
                        Severity::Violation,
                        Some(&policy.iri),
                        format!(
                            "bypassed enforcement: {context}; no conclusive evaluation bound to this commit, path and policy"
                        ),
                    );
                }
                if policy.has_selector {
                    unresolved(
                        report,
                        &mut scope,
                        Some(&policy.iri),
                        format!(
                            "{context}; selector/predicate replay is unsupported by the path-only pass"
                        ),
                    );
                } else if policy.effect.as_deref() == Some("deny") {
                    // Even a trace saying 'blocked' cannot excuse a crossing
                    // Git proves happened. Signed exceptions are not inferred.
                    finding(
                        report,
                        Severity::Violation,
                        Some(&policy.iri),
                        format!(
                            "denied path committed: {context}; a trace record is not an exception verdict"
                        ),
                    );
                } else if !matches!(
                    policy.effect.as_deref(),
                    Some("record" | "warn" | "throttle" | "escalate" | "allow")
                ) {
                    unresolved(
                        report,
                        &mut scope,
                        Some(&policy.iri),
                        format!(
                            "{context}; missing or unsupported policy effect {:?}",
                            policy.effect
                        ),
                    );
                }
            }
        }
    }
    Ok(scope)
}

fn finding(report: &mut Report, severity: Severity, policy: Option<&str>, detail: String) {
    report.discrepancies.push(Discrepancy {
        pass: Pass::GitCoverage,
        severity,
        record: None,
        constraint: policy.map(str::to_owned),
        detail,
    });
}

fn unresolved(report: &mut Report, scope: &mut Scope, policy: Option<&str>, detail: String) {
    scope.unresolved += 1;
    finding(report, Severity::Incompleteness, policy, detail);
}

fn invalid(message: impl Into<String>) -> Error {
    Error::CannotVerify(message.into())
}

#[cfg(test)]
#[path = "git_audit_tests.rs"]
mod tests;
