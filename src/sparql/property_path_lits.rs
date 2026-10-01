//! Property-path matches that END on a literal (aegis-sxlptn).
//!
//! The node engine in the parent module works on (node id, node id) pairs and
//! cannot represent a literal object, so it dropped every one: `?s (p|p) ?v`
//! returned 0 rows where `?s p ?v` returned 1. A literal is legal only as a
//! path's final object, so this evaluates literal endings beside the node
//! engine rather than inside it.

use spargebra::algebra::PropertyPathExpression;
use spargebra::term::TermPattern;

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::{Bindings, TemporalContext};

/// The object as a LITERAL to match, when it is one: a constant literal, or a
/// variable/blank node an earlier pattern bound to a non-node value. `None`
/// for an unbound object or a node. A bound blank-node label (`_:`), which
/// [`super::id_to_value`] renders as a string, stays on the node path.
pub(super) fn fixed_literal(
    store: &Store,
    object: &TermPattern,
    bindings: &Bindings,
) -> Result<Option<Value>> {
    let value = match object {
        TermPattern::Literal(_) => {
            crate::sparql::pattern_util::resolve_object_pattern(store, object, bindings)?
        }
        TermPattern::Variable(v) => bindings.get(v.as_str()).cloned(),
        TermPattern::BlankNode(b) => bindings.get(b.as_str()).cloned(),
        TermPattern::NamedNode(_) => None,
        #[cfg(feature = "shacl")]
        TermPattern::Triple(_) => None,
    };
    Ok(value.filter(|v| match v {
        Value::Ref(_) => false,
        Value::Str(s) => !s.starts_with("_:"),
        _ => true,
    }))
}

/// Evaluate the matches of `path` that END on a literal, as (subject, literal)
/// pairs (aegis-sxlptn). A literal can only be a path's final object: it is
/// never a subject, so it cannot be an intermediate node of a sequence or a
/// closure, and a reversed step cannot end on one.
pub(super) fn eval_path_lits(
    store: &Store,
    path: &PropertyPathExpression,
    fixed_subj: Option<i64>,
    fixed_lit: Option<&Value>,
    ctx: &TemporalContext,
) -> Result<Vec<(i64, Value)>> {
    match path {
        PropertyPathExpression::NamedNode(pred) => {
            let Some(pred_id) = store.lookup(pred.as_str())? else {
                return Ok(vec![]);
            };
            let mut conds = vec!["a = ?1".to_string(), "op = 1".to_string()];
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pred_id)];
            literal_edge_conditions(&mut conds, &mut params, fixed_subj, fixed_lit, ctx)?;
            query_literal_pairs(store, &conds, &params)
        }
        PropertyPathExpression::NegatedPropertySet(excluded) => {
            let mut conds = vec!["op = 1".to_string()];
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            literal_edge_conditions(&mut conds, &mut params, fixed_subj, fixed_lit, ctx)?;
            let excluded_ids: Vec<i64> = excluded
                .iter()
                .filter_map(|n| store.lookup(n.as_str()).ok().flatten())
                .collect();
            if !excluded_ids.is_empty() {
                let ph: Vec<String> = excluded_ids
                    .iter()
                    .map(|id| {
                        params.push(Box::new(*id));
                        format!("?{}", params.len())
                    })
                    .collect();
                conds.push(format!("a NOT IN ({})", ph.join(", ")));
            }
            query_literal_pairs(store, &conds, &params)
        }
        PropertyPathExpression::Reverse(_) => Ok(vec![]),
        PropertyPathExpression::Alternative(left, right) => {
            let mut pairs = eval_path_lits(store, left, fixed_subj, fixed_lit, ctx)?;
            for p in eval_path_lits(store, right, fixed_subj, fixed_lit, ctx)? {
                if !pairs.contains(&p) {
                    pairs.push(p);
                }
            }
            Ok(pairs)
        }
        PropertyPathExpression::Sequence(left, right) => {
            let mut pairs = Vec::new();
            for (s, mid) in super::eval_path_expr(store, left, fixed_subj, None, ctx)? {
                for (_, value) in eval_path_lits(store, right, Some(mid), fixed_lit, ctx)? {
                    if !pairs.contains(&(s, value.clone())) {
                        pairs.push((s, value));
                    }
                }
            }
            Ok(pairs)
        }
        // A zero-length step maps a node to itself, never to a literal.
        PropertyPathExpression::ZeroOrOne(inner) => {
            eval_path_lits(store, inner, fixed_subj, fixed_lit, ctx)
        }
        PropertyPathExpression::ZeroOrMore(inner) | PropertyPathExpression::OneOrMore(inner) => {
            // Literal endings of a closure: a final `inner` step out of the start
            // node or any node the closure reaches from it.
            let seeds: Vec<i64> = match fixed_subj {
                Some(s) => vec![s],
                None => {
                    let mut seeds: Vec<i64> = Vec::new();
                    for (s, _) in super::eval_path_expr(store, inner, None, None, ctx)? {
                        if !seeds.contains(&s) {
                            seeds.push(s);
                        }
                    }
                    for (s, _) in eval_path_lits(store, inner, None, fixed_lit, ctx)? {
                        if !seeds.contains(&s) {
                            seeds.push(s);
                        }
                    }
                    seeds
                }
            };
            let mut pairs = Vec::new();
            for seed in seeds {
                let mut nodes = vec![seed];
                nodes.extend(super::bfs_forward(store, inner, seed, ctx)?);
                for node in nodes {
                    for (_, value) in eval_path_lits(store, inner, Some(node), fixed_lit, ctx)? {
                        if !pairs.contains(&(seed, value.clone())) {
                            pairs.push((seed, value));
                        }
                    }
                }
            }
            Ok(pairs)
        }
    }
}

fn literal_edge_conditions(
    conds: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    fixed_subj: Option<i64>,
    fixed_lit: Option<&Value>,
    ctx: &TemporalContext,
) -> Result<()> {
    super::add_graph_condition(conds, params, ctx)?;
    if let Some(s) = fixed_subj {
        conds.push(format!("e = ?{}", params.len() + 1));
        params.push(Box::new(s));
    }
    if let Some(lit) = fixed_lit {
        conds.push(format!("v = ?{}", params.len() + 1));
        params.push(Box::new(lit.to_bytes()));
    }
    super::add_temporal_conditions(conds, params, ctx);
    Ok(())
}

/// Like [`super::query_pairs`], but keeps the NON-node objects that one drops.
fn query_literal_pairs(
    store: &Store,
    conds: &[String],
    params: &[Box<dyn rusqlite::types::ToSql>],
) -> Result<Vec<(i64, Value)>> {
    let sql = format!(
        "SELECT DISTINCT e, v FROM {} WHERE {}",
        store.facts_source(),
        conds.join(" AND ")
    );
    let mut stmt = store.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(std::convert::AsRef::as_ref).collect();
    let mut rows = stmt.query(refs.as_slice())?;
    let mut results: Vec<(i64, Value)> = Vec::new();
    while let Some(row) = rows.next()? {
        let e_id: i64 = row.get(0)?;
        let v = Value::from_bytes(&row.get::<_, Vec<u8>>(1)?)?;
        if !matches!(v, Value::Ref(_)) && !results.contains(&(e_id, v.clone())) {
            results.push((e_id, v));
        }
    }
    Ok(results)
}
