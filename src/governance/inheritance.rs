//! Constraint inheritance under delegation — SARC §9.5's **laundering** path.
//!
//! An orchestrator is bound by a constraint. It dispatches a sub-agent. The
//! constraint is not re-applied at the deeper layer, and the action happens
//! anyway. Nobody decided to drop it; it simply was not carried, and the record
//! looks exactly like a constraint that legitimately did not apply.
//!
//! ## The decidability rescue, and why "drop" is not expressible
//!
//! The reason a constraint stops applying deeper down is usually honest: the
//! evidence it needs is not available there. SARC's answer is not to relax it
//! but to **evaluate at the deepest layer where it still decides, and escalate
//! otherwise**. `aegis:onUndecidable` therefore admits only `"escalate"` — the
//! same shape as `aegis:onTimeout` admitting only `"deny"`. A constraint that
//! silently stops applying where it cannot be checked is one an agent escapes by
//! dispatching into a context where the evidence is absent.
//!
//! ## What this pass can decide, and what it cannot
//!
//! It works over the reconstructed attribution tree
//! ([`crate::governance::tree`]), asking whether constraints evaluated at a
//! dispatch node were also evaluated below it.
//!
//! **The strong finding.** Constraint *C* was evaluated on target *T* at some
//! depth, and a later record for the **same target** deeper in the same chain
//! does not evaluate it. *C* was demonstrably decidable for *T* — it decided,
//! once — so its absence deeper is a drop rather than an inapplicability. That
//! is a violation.
//!
//! **The weak finding.** *C* was evaluated at a dispatch node and never appears
//! anywhere in its subtree. This *might* be laundering and might be a selector
//! that legitimately matched nothing below; quipu has neither the file nor the
//! parser to tell. Reported as an incompleteness, because the record genuinely
//! cannot distinguish the two, and calling it a violation would flag every
//! constraint that simply did not apply.
//!
//! The difference between those two is the whole reason this is a separate pass
//! rather than a rule inside the coverage one: the strong case has evidence and
//! the weak case has a question, and reporting them at the same severity would
//! make the strong one unfindable.

use std::collections::{BTreeMap, BTreeSet};

use super::audit::{Discrepancy, Pass, Report, Severity, TraceRecord};
use super::audit_spec::{self, Spec};
use crate::error::Result;
use crate::store::Store;

/// Constraints Σ declares as inherited by delegates.
fn inherited(spec: &Spec) -> BTreeSet<&str> {
    spec.values()
        .filter(|c| c.inherited_by_delegates == Some(true))
        .map(|c| c.id.as_str())
        .collect()
}

fn finding(severity: Severity, record: Option<usize>, id: &str, detail: String) -> Discrepancy {
    Discrepancy {
        pass: Pass::Inheritance,
        severity,
        record,
        constraint: Some(id.to_string()),
        detail,
    }
}

/// Whether `deeper` is strictly below `chain` in the dispatch tree.
fn is_below(chain: &[String], deeper: &[String]) -> bool {
    deeper.len() > chain.len() && deeper.starts_with(chain)
}

/// Check a trace for constraints that stopped applying under delegation.
pub fn check(store: &Store, trace: &[TraceRecord]) -> Result<Report> {
    let spec = audit_spec::load(store)?;
    let mut report = Report {
        records_checked: trace.len(),
        constraints_in_scope: spec.len(),
        ..Report::default()
    };
    let inherited = inherited(&spec);

    // Σ declaring nothing inheritable is itself worth saying. Otherwise a
    // deployment that never declared inheritance reads as one where inheritance
    // was checked and found clean.
    if inherited.is_empty() {
        report.discrepancies.push(Discrepancy {
            pass: Pass::Inheritance,
            severity: Severity::Incompleteness,
            record: None,
            constraint: None,
            detail: "no constraint in Σ declares aegis:inheritedByDelegates, so \
                     nothing here can be checked. That is not the same as no \
                     constraint being inherited — it means none has said."
                .to_string(),
        });
        return Ok(report);
    }

    // Where each (constraint, target) was last seen decided, and by which chain.
    let mut decided: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    // Every chain a constraint was evaluated under, for the weak finding.
    let mut evaluated_under: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();

    for (index, record) in trace.iter().enumerate() {
        let chain = &record.principal_chain;
        let evaluated: BTreeSet<&str> = record
            .constraints
            .iter()
            .map(|e| e.id.as_str())
            .filter(|id| inherited.contains(id))
            .collect();

        // The STRONG finding, first: a target this record touches, for which an
        // inherited constraint decided higher up the same chain and is now
        // absent. The constraint proved it could decide for this target, so its
        // absence here is a drop.
        if let Some(target) = record.path.as_deref() {
            for id in &inherited {
                if evaluated.contains(id) {
                    continue;
                }
                let key = ((*id).to_string(), target.to_string());
                let Some(shallower) = decided.get(&key) else {
                    continue;
                };
                if !is_below(shallower, chain) {
                    continue;
                }
                report.discrepancies.push(finding(
                    Severity::Violation,
                    Some(index),
                    id,
                    format!(
                        "'{id}' is declared inheritedByDelegates and decided on \
                         '{target}' under chain [{}], but this deeper action on \
                         the same target under [{}] did not evaluate it. It was \
                         demonstrably decidable for this target, so its absence \
                         here is a DROP, not an inapplicability — evaluate it at \
                         the deepest layer where it still decides, or escalate.",
                        shallower.join(" → "),
                        chain.join(" → "),
                    ),
                ));
            }
        }

        for id in evaluated {
            evaluated_under
                .entry(id.to_string())
                .or_default()
                .push(chain.clone());
            if let Some(target) = record.path.as_deref() {
                decided.insert((id.to_string(), target.to_string()), chain.clone());
            }
        }
    }

    weak_findings(&evaluated_under, &mut report);
    Ok(report)
}

/// Constraints evaluated at a dispatch node and never in its subtree.
fn weak_findings(evaluated_under: &BTreeMap<String, Vec<Vec<String>>>, report: &mut Report) {
    for (id, chains) in evaluated_under {
        // A chain with something strictly below it somewhere in the window is a
        // dispatch node. If the constraint was never evaluated under any of
        // those deeper chains, it stopped at the boundary.
        let deeper_exists = chains.iter().any(|a| chains.iter().any(|b| is_below(a, b)));
        if deeper_exists {
            continue;
        }
        let dispatch_nodes: Vec<&Vec<String>> = chains.iter().filter(|c| c.len() > 1).collect();
        if dispatch_nodes.is_empty() && chains.iter().all(|c| c.len() <= 1) {
            // Only ever evaluated at a root, with no delegation in the window.
            // Nothing to say: inheritance was never exercised.
            continue;
        }
        report.discrepancies.push(finding(
            Severity::Incompleteness,
            None,
            id,
            format!(
                "'{id}' is declared inheritedByDelegates and was never evaluated \
                 anywhere below the chain it fired on. That is either laundering \
                 — the constraint stopping at a delegation boundary — or a \
                 selector that legitimately matched nothing deeper, and this \
                 record cannot tell them apart. Deciding needs the files as they \
                 stood, which quipu does not have."
            ),
        ));
    }
}

/// Check a raw JSONL trace.
pub fn check_jsonl(store: &Store, jsonl: &str) -> Result<Report> {
    let (records, unreadable) = super::audit::parse_trace(jsonl);
    let mut report = check(store, &records)?;
    report.records_unreadable = unreadable;
    Ok(report)
}

#[cfg(test)]
#[path = "inheritance_tests.rs"]
mod tests;
