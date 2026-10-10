//! Push `FILTER(?v = <iri>)` into the basic graph pattern it wraps (aegis-o3l46b).
//!
//! The asserted-only type form `?s a ?t . FILTER(?t = <T>)` is the documented
//! way to ask "is this node DIRECTLY typed T" (subclass inference does not apply
//! to a variable object). Evaluated naively, `?s a ?t` scans every `rdf:type`
//! fact in the store before the filter runs, so joined with anything else it
//! took the whole 10 s query budget (408) while each pattern alone answered in
//! milliseconds. Measured on the served store 2026-10-02: 3 of 3 at 10.1 s.
//!
//! The fix seeds the filtered variable with the IRI's id before the BGP runs.
//! `resolve_object_pattern` then pushes it into SQL exactly like a bound join
//! variable (quipu #367), and the pattern keeps its variable object, so the
//! subclass path is still NOT taken: asserted-only semantics are unchanged.
//! The filter itself still runs afterwards, so a seed can only narrow rows the
//! filter would have dropped anyway.
//!
//! Deliberately narrow — each condition is a correctness boundary, not a style:
//! - only a top-level conjunct of the filter (`a && b`), never under `||` or `!`;
//! - only `=` or `sameTerm` between a variable and an IRI (for literals `=`
//!   compares VALUES, so `1 = 1.0` holds and a term seed would drop rows);
//! - only when the filter wraps a plain BGP and the variable occurs in it (a
//!   variable that is unbound in the BGP makes the filter an error, i.e. false,
//!   and seeding would turn that into true);
//! - never a variable the caller already bound;
//! - only when the IRI has exactly ONE id across attached layers: bindings are
//!   compared by id, so a seed with one layer's id would reject an equal row
//!   carrying another layer's id, which the filter itself would accept.

use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use super::pattern_util::Bindings;
use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

/// The seed to evaluate `inner` under, or `None` when nothing can be pushed.
///
/// # Errors
/// [`crate::Error::Sqlite`] if looking up a filtered IRI fails.
pub fn seed_iri_equalities(
    store: &Store,
    expr: &Expression,
    inner: &GraphPattern,
    seed: &Bindings,
) -> Result<Option<Bindings>> {
    let GraphPattern::Bgp { patterns } = inner else {
        return Ok(None);
    };
    let mut pairs = Vec::new();
    collect_iri_equalities(expr, &mut pairs);
    let mut out: Option<Bindings> = None;
    for (var, iri) in pairs {
        if seed.contains_key(var) || !occurs_in(patterns, var) {
            continue;
        }
        let ids = store.lookup_all(iri)?;
        let [id] = ids.as_slice() else {
            continue;
        };
        out.get_or_insert_with(|| seed.clone())
            .insert(var.to_string(), Value::Ref(*id));
    }
    Ok(out)
}

/// `(variable name, iri)` for each top-level `?v = <iri>` / `sameTerm` conjunct.
fn collect_iri_equalities<'a>(expr: &'a Expression, out: &mut Vec<(&'a str, &'a str)>) {
    match expr {
        Expression::And(left, right) => {
            collect_iri_equalities(left, out);
            collect_iri_equalities(right, out);
        }
        Expression::Equal(a, b) | Expression::SameTerm(a, b) => match (a.as_ref(), b.as_ref()) {
            (Expression::Variable(v), Expression::NamedNode(n))
            | (Expression::NamedNode(n), Expression::Variable(v)) => {
                out.push((v.as_str(), n.as_str()));
            }
            _ => {}
        },
        _ => {}
    }
}

fn occurs_in(patterns: &[TriplePattern], var: &str) -> bool {
    let term = |t: &TermPattern| matches!(t, TermPattern::Variable(v) if v.as_str() == var);
    patterns.iter().any(|tp| {
        term(&tp.subject)
            || term(&tp.object)
            || matches!(&tp.predicate, NamedNodePattern::Variable(v) if v.as_str() == var)
    })
}
