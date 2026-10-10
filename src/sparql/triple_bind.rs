//! Shared SQL and resident-model triple binding and compatibility.
use super::Bindings;
use super::pattern_util::bind_var;
use crate::types::Value;
use crate::{Result, Store};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

/// Turn one matched `(e, a, v, g)` row into extended bindings, or `None` when
/// the row is incompatible with what is already bound.
///
/// **Extracted so the SQL path and the read-model path cannot drift.** This is
/// where a triple becomes a `Value`, and the rules are subtle enough that two
/// copies would diverge: a subject resolving to a blank node binds as
/// `Value::Str`, everything else re-looks-up its IRI to decide between
/// `Value::Ref` and `Value::Str`, and the predicate has no blank-node case at
/// all. Any read model consulted instead of SQL must produce identical
/// bindings, and sharing this is how that is guaranteed rather than hoped for.
/// The matched row a [`bind_row`] call is binding — grouped so the function
/// stays under the argument limit and so the four values that describe ONE
/// triple travel together.
pub(super) struct MatchedRow {
    pub(super) e_id: i64,
    pub(super) a_id: i64,
    pub(super) v: Value,
    pub(super) g_id: Option<i64>,
}

pub(super) fn bind_row(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    row: MatchedRow,
    bind_graph_var: Option<&str>,
) -> Result<Option<Bindings>> {
    let MatchedRow {
        e_id,
        a_id,
        v,
        g_id,
    } = row;
    let mut new_bindings = bindings.clone();
    let mut compatible = true;

    // Bind subject variable (or blank node used as join variable).
    match &tp.subject {
        TermPattern::Variable(var) => {
            let e_iri = if super::count_stream::direct_refs() && !store.has_attachments() {
                if !bind_var(
                    &mut new_bindings,
                    var.as_str(),
                    Value::Ref(e_id),
                    &mut compatible,
                ) {
                    return Ok(None);
                }
                return bind_remaining_row(store, tp, new_bindings, a_id, v, g_id, bind_graph_var);
            } else {
                store.resolve(e_id)?
            };
            // A blank-node subject binds as the SAME kind of value the object
            // position gives it (a Ref), so `?x :p ?y . ?y a :c` joins when ?y
            // is a blank node. It used to bind as Value::Str("_:y") here while
            // the object binding was Value::Ref, so the two never compared
            // equal and every such join silently dropped its blank-node rows
            // (W3C entailment `owlds02`, aegis-56bvs2). Hidden until now
            // because the root-default `rdf:type <C>` path binds Ref itself.
            let e_val = if let Some(term_id) = store.lookup(&e_iri)? {
                Value::Ref(term_id)
            } else {
                Value::Str(e_iri)
            };
            if !bind_var(&mut new_bindings, var.as_str(), e_val, &mut compatible) {
                return Ok(None);
            }
        }
        TermPattern::BlankNode(b) => {
            let e_iri = store.resolve(e_id)?;
            let e_val = if let Some(term_id) = store.lookup(&e_iri)? {
                Value::Ref(term_id)
            } else {
                Value::Str(e_iri)
            };
            if !bind_var(&mut new_bindings, b.as_str(), e_val, &mut compatible) {
                return Ok(None);
            }
        }
        _ => {}
    }

    bind_remaining_row(store, tp, new_bindings, a_id, v, g_id, bind_graph_var)
}

fn bind_remaining_row(
    store: &Store,
    tp: &TriplePattern,
    mut new_bindings: Bindings,
    a_id: i64,
    v: Value,
    g_id: Option<i64>,
    bind_graph_var: Option<&str>,
) -> Result<Option<Bindings>> {
    let mut compatible = true;
    // Bind predicate variable.
    if let NamedNodePattern::Variable(var) = &tp.predicate {
        let a_val = if super::count_stream::direct_refs() && !store.has_attachments() {
            Value::Ref(a_id)
        } else {
            let a_iri = store.resolve(a_id)?;
            if let Some(term_id) = store.lookup(&a_iri)? {
                Value::Ref(term_id)
            } else {
                Value::Str(a_iri)
            }
        };
        if !bind_var(&mut new_bindings, var.as_str(), a_val, &mut compatible) {
            return Ok(None);
        }
    }

    // Bind object variable (or blank node used as join variable).
    match &tp.object {
        TermPattern::Variable(var) => {
            if !bind_var(&mut new_bindings, var.as_str(), v, &mut compatible) {
                return Ok(None);
            }
        }
        TermPattern::BlankNode(b)
            if !bind_var(&mut new_bindings, b.as_str(), v, &mut compatible) =>
        {
            return Ok(None);
        }
        _ => {}
    }

    // Bind the graph variable for `GRAPH ?g { … }` (quipu #36): ?g resolves
    // to the graph's IRI (g is the interned id of that IRI). Same ?g across
    // a BGP is enforced by the join in eval_bgp via bind_var compatibility.
    if let (Some(g_var), Some(gid)) = (bind_graph_var, g_id) {
        // g is the interned term id of the graph IRI (schema.rs), and this
        // branch only runs for named graphs (g<>0), so gid is always a
        // valid term id — bind ?g to it directly as a ref.
        if !bind_var(&mut new_bindings, g_var, Value::Ref(gid), &mut compatible) {
            return Ok(None);
        }
    }

    Ok(if compatible { Some(new_bindings) } else { None })
}
