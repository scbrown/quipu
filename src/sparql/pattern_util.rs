//! Pattern evaluation utilities — variable binding, resolution, and join logic.

use std::collections::HashMap;

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::TemporalContext;

pub type Bindings = HashMap<String, Value>;

/// Try to bind a variable. Returns false if incompatible with existing binding.
pub fn bind_var(bindings: &mut Bindings, var: &str, value: Value, compatible: &mut bool) -> bool {
    if let Some(existing) = bindings.get(var) {
        if existing != &value {
            *compatible = false;
            return false;
        }
    } else {
        bindings.insert(var.to_string(), value);
    }
    true
}

/// Resolve a subject pattern to an IRI string if it's bound.
///
/// A variable (or a parser-generated blank node, which acts as one in path
/// sequences) that an earlier pattern already bound to a node is resolved to
/// that node's IRI, exactly as if the query had named it. Before this, a bound
/// node returned `None` (the first engine's "We'd need store to resolve, skip
/// for now"), so every pattern after the first in a nested-loop join scanned
/// EVERY fact with its predicate and filtered in Rust: one bound subject cost
/// ~0.85 s on a 2.6M-fact store where the named-IRI form costs ~9 ms
/// (aegis-o3l46b). Going through the IRI, not the raw id, keeps attached
/// layers correct: the caller's `lookup_all` maps it to every layer's id.
///
/// The `-1` "never matches" sentinel stays unresolved, which is the old,
/// correct-but-unpushed behaviour; `bind_var` still filters.
///
/// # Errors
/// [`crate::Error::Sqlite`] if resolving the bound id fails.
pub fn resolve_subject_pattern(
    store: &Store,
    pattern: &TermPattern,
    bindings: &Bindings,
) -> Result<Option<String>> {
    let bound = |name: &str| -> Result<Option<String>> {
        match bindings.get(name) {
            Some(Value::Ref(id)) if *id >= 0 => store.resolve(*id).map(Some),
            _ => Ok(None),
        }
    };
    match pattern {
        TermPattern::NamedNode(n) => Ok(Some(n.as_str().to_string())),
        TermPattern::BlankNode(b) => bound(b.as_str()),
        TermPattern::Variable(v) => bound(v.as_str()),
        TermPattern::Literal(_) => Ok(None),
        #[cfg(feature = "shacl")]
        TermPattern::Triple(_) => Ok(None),
    }
}

/// Resolve a predicate pattern to an IRI string if it's bound.
///
/// A predicate variable bound by an earlier pattern is resolved to its IRI,
/// for the same reason and with the same sentinel rule as
/// [`resolve_subject_pattern`].
///
/// # Errors
/// [`crate::Error::Sqlite`] if resolving the bound id fails.
pub fn resolve_predicate_pattern(
    store: &Store,
    pattern: &NamedNodePattern,
    bindings: &Bindings,
) -> Result<Option<String>> {
    match pattern {
        NamedNodePattern::NamedNode(n) => Ok(Some(n.as_str().to_string())),
        NamedNodePattern::Variable(v) => match bindings.get(v.as_str()) {
            Some(Value::Ref(id)) if *id >= 0 => store.resolve(*id).map(Some),
            _ => Ok(None),
        },
    }
}

/// Resolve an object pattern to a Value if it's a concrete term.
pub fn resolve_object_pattern(
    store: &Store,
    pattern: &TermPattern,
    bindings: &Bindings,
) -> Result<Option<Value>> {
    match pattern {
        TermPattern::NamedNode(n) => {
            if let Some(id) = store.lookup(n.as_str())? {
                Ok(Some(Value::Ref(id)))
            } else {
                Ok(Some(Value::Ref(-1))) // Will never match
            }
        }
        TermPattern::Literal(lit) => Ok(Some(super::filter::literal_to_value(lit))),
        TermPattern::Variable(v) => {
            // If already bound, use that value.
            Ok(bindings.get(v.as_str()).cloned())
        }
        TermPattern::BlankNode(b) => {
            // Blank nodes act as join variables — check existing bindings.
            Ok(bindings.get(b.as_str()).cloned())
        }
        #[cfg(feature = "shacl")]
        TermPattern::Triple(_) => Ok(None),
    }
}

/// Get all variable names from a triple pattern.
/// Includes `BlankNode` names since spargebra uses them as join variables for
/// property path sequences (p/q is expanded into BGP with intermediate blank nodes).
pub fn triple_pattern_vars(tp: &TriplePattern) -> Vec<String> {
    let mut vars = Vec::new();
    match &tp.subject {
        TermPattern::Variable(v) => vars.push(v.as_str().to_string()),
        TermPattern::BlankNode(b) => vars.push(b.as_str().to_string()),
        _ => {}
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        vars.push(v.as_str().to_string());
    }
    match &tp.object {
        TermPattern::Variable(v) => vars.push(v.as_str().to_string()),
        TermPattern::BlankNode(b) => vars.push(b.as_str().to_string()),
        _ => {}
    }
    vars
}

/// How many pure-Rust loop iterations between deadline polls. Row-cap checks
/// are a usize compare and run every iteration; the deadline poll (`Deadline::passed`) is the only
/// cost worth amortizing.
pub const BUDGET_POLL_STRIDE: usize = 1024;

/// Enforce the evaluation budget from inside a pure-Rust loop.
///
/// The mfg0 wedge: `merge_bindings` cloned exploded binding tables at 100%
/// CPU for ~4 hours while holding the store mutex — the `SQLite` progress
/// handler never fired (no `SQLite` in the loop) and the between-operator
/// check never ran (control never left the join). Any loop that can grow
/// with the product of its inputs must call this.
///
/// `produced` is checked against the row cap on every call; the deadline is
/// polled only when `i` is a multiple of [`BUDGET_POLL_STRIDE`]. The zero
/// timeout fields are placeholders `query_temporal` rewrites with real
/// elapsed/limit values.
#[inline]
pub fn check_eval_budget(
    ctx: &TemporalContext,
    i: usize,
    produced: usize,
) -> crate::error::Result<()> {
    if let Some(cap) = ctx.row_cap
        && produced > cap
    {
        return Err(crate::error::Error::QueryComplexity { limit: cap });
    }
    if i.is_multiple_of(BUDGET_POLL_STRIDE) && ctx.deadline.is_some_and(|dl| dl.passed()) {
        return Err(crate::error::Error::QueryTimeout {
            elapsed_ms: 0,
            limit_ms: 0,
        });
    }
    Ok(())
}

/// Join two sets of bindings on shared variables, under the evaluation
/// budget: the nested loop is exactly where a join explodes, so the budget is
/// enforced here and not merely around the call.
pub fn join_rows(
    left: &[Bindings],
    right: &[Bindings],
    ctx: &TemporalContext,
) -> crate::error::Result<Vec<Bindings>> {
    let mut results = Vec::new();
    let mut i = 0usize;
    for l in left {
        for r in right {
            check_eval_budget(ctx, i, results.len())?;
            i += 1;
            if let Some(merged) = merge_bindings(l, r) {
                results.push(merged);
            }
        }
    }
    Ok(results)
}

/// Merge two binding rows. Returns None if they conflict on shared variables.
pub fn merge_bindings(a: &Bindings, b: &Bindings) -> Option<Bindings> {
    let mut merged = a.clone();
    for (k, v) in b {
        // Federation labels are response metadata, not SPARQL variables. Two
        // SERVICE operands may legitimately name different providers; their
        // labels must never turn otherwise-compatible RDF bindings into a
        // failed join. The later operand's metadata describes the merged row.
        if matches!(k.as_str(), "_provider" | "_trust" | "_freshness") {
            merged.insert(k.clone(), v.clone());
            continue;
        }
        if let Some(existing) = merged.get(k) {
            if existing != v {
                return None;
            }
        } else {
            merged.insert(k.clone(), v.clone());
        }
    }
    Some(merged)
}
