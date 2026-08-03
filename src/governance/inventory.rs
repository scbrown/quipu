//! The dispatch-graph inventory — SARC I7, enforcement completeness.
//!
//! I7 is a property of the **dispatch graph**, not of any one constraint: an
//! agent harness exposes N classes of tool call, and enforcement completeness is
//! the question of whether every class that can change state passes through a
//! point where a constraint could stop it.
//!
//! ## Why this is data and not a paragraph
//!
//! hank's `docs/work-scoped-governance.md` §"What this cannot reach" is an
//! honest list of bypass surfaces — CI pipelines, cron, the far side of a remote
//! shell, a sibling session's VCS index, a hostile agent. It is also prose, and
//! prose goes stale the first time a harness adds a tool: nothing recomputes,
//! nothing notices, and the list quietly becomes a description of last year's
//! deployment. Declaring the classes as `aegis:ToolClass` facts makes the same
//! statements checkable, and makes a new ungoverned class an error rather than
//! an omission nobody sees.
//!
//! ## Three findings, and the distinction that matters most
//!
//! An executable class with no enforcement point and **no stated reason** is an
//! unknown hole: a violation. The same class with an `ungovernedReason` is an
//! **acknowledged bypass surface** — reported, because it is still ungoverned,
//! but as an incompleteness, because somebody has looked at it and said why.
//! Neither is "governed", and this module never reports one as the other. That
//! distinction is the entire value of writing the list down: without it, an
//! operator cannot tell a decision from an oversight.
//!
//! The third is the cross-check the other direction: a constraint placed at a
//! point **no executable class traverses** can never fire. It reads as
//! governance in the catalog and is inert in the deployment, which is the
//! failure mode hardest to see from either side alone.

use std::collections::BTreeSet;

use super::audit::{Discrepancy, Pass, Report, Severity};
use super::audit_spec;
use crate::error::Result;
use crate::namespace::DEFAULT_BASE_NS;
use crate::sparql::{self, QueryResult};
use crate::store::Store;
use crate::types::Value;

/// One declared class of dispatchable action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolClass {
    /// The class's IRI.
    pub iri: String,
    /// Its human name.
    pub label: String,
    /// Who dispatches it.
    pub dispatched_by: Option<String>,
    /// Whether invoking it can change state.
    pub executable: Option<bool>,
    /// Enforcement points it actually traverses.
    pub governed_at: BTreeSet<String>,
    /// Why it traverses none, when it traverses none.
    pub ungoverned_reason: Option<String>,
    /// Where enforcement happens instead.
    pub enforced_instead_at: Option<String>,
}

impl ToolClass {
    /// Whether this class passes through any enforcement point.
    #[must_use]
    pub fn is_governed(&self) -> bool {
        !self.governed_at.is_empty()
    }
}

/// Read the declared inventory from `store`.
pub fn load(store: &Store) -> Result<Vec<ToolClass>> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
         SELECT ?c ?label ?by ?executable ?point ?reason ?instead WHERE {{ \
            ?c a a:ToolClass . \
            OPTIONAL {{ ?c rdfs:label ?label }} \
            OPTIONAL {{ ?c a:dispatchedBy ?by }} \
            OPTIONAL {{ ?c a:executable ?executable }} \
            OPTIONAL {{ ?c a:governedAt ?point }} \
            OPTIONAL {{ ?c a:ungovernedReason ?reason }} \
            OPTIONAL {{ ?c a:enforcedInsteadAt ?instead }} \
         }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &q)? else {
        return Ok(Vec::new());
    };

    let mut classes: Vec<ToolClass> = Vec::new();
    for row in &rows {
        let Some(iri) = text(store, row.get("c")) else {
            continue;
        };
        // `governedAt` is repeatable, so one class spans several rows. Merging
        // rather than replacing is what keeps a two-point class from being
        // recorded as whichever point the last row happened to bind.
        let entry = match classes.iter_mut().find(|c| c.iri == iri) {
            Some(existing) => existing,
            None => {
                classes.push(ToolClass {
                    iri: iri.clone(),
                    ..ToolClass::default()
                });
                classes.last_mut().expect("just pushed")
            }
        };
        if entry.label.is_empty() {
            entry.label = text(store, row.get("label")).unwrap_or_else(|| iri.clone());
        }
        merge(&mut entry.dispatched_by, text(store, row.get("by")));
        merge(&mut entry.ungoverned_reason, text(store, row.get("reason")));
        merge(
            &mut entry.enforced_instead_at,
            text(store, row.get("instead")),
        );
        if entry.executable.is_none() {
            entry.executable = boolean(store, row.get("executable"));
        }
        if let Some(point) = text(store, row.get("point")) {
            entry.governed_at.insert(point);
        }
    }
    classes.sort_by(|a, b| a.iri.cmp(&b.iri));
    Ok(classes)
}

/// Check the dispatch graph against I7, and Σ against the dispatch graph.
pub fn check(store: &Store) -> Result<Report> {
    let classes = load(store)?;
    let spec = audit_spec::load(store)?;
    let mut report = Report {
        records_checked: classes.len(),
        constraints_in_scope: spec.len(),
        ..Report::default()
    };

    // An empty inventory is not a clean bill of health, and reporting it as one
    // would be the single most misleading thing this module could do: every
    // downstream check would pass by having nothing to check.
    if classes.is_empty() {
        report.discrepancies.push(finding(
            Severity::Incompleteness,
            None,
            "no aegis:ToolClass is declared, so enforcement completeness (I7) \
             cannot be evaluated at all. An empty inventory is not an empty \
             dispatch graph — it is an unwritten one."
                .to_string(),
        ));
        return Ok(report);
    }

    // Coverage counts EXECUTABLE classes only, matching what the finding below
    // says. A read-only class that happens to traverse a gate does not make a
    // constraint placed there able to stop anything.
    let mut covered: BTreeSet<&str> = BTreeSet::new();
    for class in &classes {
        if class.executable == Some(true) {
            for point in &class.governed_at {
                covered.insert(point.as_str());
            }
        }
        check_class(class, &mut report);
    }

    // The other direction: a constraint placed where nothing traverses.
    for constraint in spec.values() {
        let Some(point) = constraint.point.as_deref() else {
            continue;
        };
        if covered.contains(point) {
            continue;
        }
        report.discrepancies.push(finding(
            Severity::Violation,
            Some(&constraint.id),
            format!(
                "constraint '{}' is placed at \"{point}\", which no declared \
                 executable tool class traverses. It reads as governance in the \
                 catalog and can never fire in this deployment. Re-place it, or \
                 declare the class that reaches that point.",
                constraint.id
            ),
        ));
    }
    Ok(report)
}

fn check_class(class: &ToolClass, report: &mut Report) {
    let label = &class.label;
    let Some(executable) = class.executable else {
        report.discrepancies.push(finding(
            Severity::Incompleteness,
            Some(label),
            format!(
                "tool class '{label}' does not declare aegis:executable, so \
                 whether it needs an enforcement point is undecidable. Declare \
                 it — a read-only class needs none, and guessing either way is \
                 wrong in a direction that matters."
            ),
        ));
        return;
    };
    if !executable || class.is_governed() {
        return;
    }

    let Some(reason) = class.ungoverned_reason.as_deref() else {
        report.discrepancies.push(finding(
            Severity::Violation,
            Some(label),
            format!(
                "tool class '{label}' can change state and traverses no \
                 enforcement point, and no aegis:ungovernedReason says why. That \
                 is an unknown hole in the dispatch graph (I7). Route it through \
                 a point, or state why it cannot be."
            ),
        ));
        return;
    };
    // Acknowledged, not governed. Reported every time, because a bypass surface
    // an operator has stopped seeing is one they have stopped weighing.
    let instead = class.enforced_instead_at.as_deref().map_or_else(
        || {
            " No aegis:enforcedInsteadAt is declared, so nothing says where it \
             IS enforced."
                .to_string()
        },
        |where_| format!(" Enforced instead at: {where_}."),
    );
    report.discrepancies.push(finding(
        Severity::Incompleteness,
        Some(label),
        format!(
            "tool class '{label}' can change state and traverses no enforcement \
             point. Acknowledged: {reason}.{instead}"
        ),
    ));
}

/// A one-line summary for an inventory report.
///
/// Separate from [`Report::summary`] because that one counts trace records and
/// unreadable lines, and neither exists here. Reusing it would print "0 line(s)
/// unreadable" about a check that never read a line — a true statement that
/// makes a reader believe they are looking at a trace result.
#[must_use]
pub fn summary(report: &Report) -> String {
    let ungoverned = report.of(Severity::Violation).len();
    let acknowledged = report.of(Severity::Incompleteness).len();
    format!(
        "dispatch graph: {ungoverned} unknown hole(s), {acknowledged} \
         acknowledged/undecidable surface(s) over {classes} declared tool \
         class(es) against {constraints} constraint(s)",
        classes = report.records_checked,
        constraints = report.constraints_in_scope,
    )
}

fn finding(severity: Severity, subject: Option<&str>, detail: String) -> Discrepancy {
    Discrepancy {
        pass: Pass::Inventory,
        severity,
        record: None,
        constraint: subject.map(str::to_string),
        detail,
    }
}

fn merge(slot: &mut Option<String>, value: Option<String>) {
    if slot.is_none() {
        *slot = value;
    }
}

fn text(store: &Store, value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Ref(id)) => store.resolve(*id).ok(),
        _ => None,
    }
}

/// A boolean, however the store happens to hold it. Anything unrecognised is
/// `None` rather than `false`: "not declared" and "declared read-only" are
/// different claims, and only one of them means no enforcement point is needed.
fn boolean(store: &Store, value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Int(i)) => Some(*i != 0),
        Some(Value::Str(s)) => parse_bool(s),
        Some(Value::Ref(id)) => store.resolve(*id).ok().and_then(|s| parse_bool(&s)),
        _ => None,
    }
}

fn parse_bool(text: &str) -> Option<bool> {
    match text {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
