//! Pattern evaluation utilities — variable binding, resolution, and join logic.

use std::collections::HashMap;

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

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
pub fn resolve_subject_pattern(pattern: &TermPattern, bindings: &Bindings) -> Option<String> {
    match pattern {
        TermPattern::NamedNode(n) => Some(n.as_str().to_string()),
        TermPattern::BlankNode(b) => Some(format!("_:{}", b.as_str())),
        TermPattern::Variable(v) => match bindings.get(v.as_str()) {
            Some(Value::Ref(_)) => None,
            _ => None,
        },
        TermPattern::Literal(_) => None,
        #[cfg(feature = "shacl")]
        TermPattern::Triple(_) => None,
    }
}

/// Resolve a predicate pattern to an IRI string if it's bound.
pub fn resolve_predicate_pattern(
    pattern: &NamedNodePattern,
    bindings: &Bindings,
) -> Option<String> {
    match pattern {
        NamedNodePattern::NamedNode(n) => Some(n.as_str().to_string()),
        NamedNodePattern::Variable(v) => match bindings.get(v.as_str()) {
            Some(Value::Ref(_)) => None,
            _ => None,
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
        TermPattern::BlankNode(_) => Ok(None),
        #[cfg(feature = "shacl")]
        TermPattern::Triple(_) => Ok(None),
    }
}

/// Get all variable names from a triple pattern.
pub fn triple_pattern_vars(tp: &TriplePattern) -> Vec<String> {
    let mut vars = Vec::new();
    if let TermPattern::Variable(v) = &tp.subject {
        vars.push(v.as_str().to_string());
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        vars.push(v.as_str().to_string());
    }
    if let TermPattern::Variable(v) = &tp.object {
        vars.push(v.as_str().to_string());
    }
    vars
}

/// Join two sets of bindings on shared variables.
pub fn join_rows(left: &[Bindings], right: &[Bindings]) -> Vec<Bindings> {
    let mut results = Vec::new();
    for l in left {
        for r in right {
            if let Some(merged) = merge_bindings(l, r) {
                results.push(merged);
            }
        }
    }
    results
}

/// Merge two binding rows. Returns None if they conflict on shared variables.
pub fn merge_bindings(a: &Bindings, b: &Bindings) -> Option<Bindings> {
    let mut merged = a.clone();
    for (k, v) in b {
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
