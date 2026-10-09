//! Narrow streaming OPTIONAL admission; all other left joins keep the old path.
use super::filter::eval_filter;
use super::pattern_util::{check_eval_budget, triple_pattern_vars};
use super::{Bindings, GraphScope, TemporalContext};
use crate::{Result, Store};
use spargebra::algebra::{Expression, Function, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern};

fn local(expr: &Expression, subject: &str, object: &str) -> bool {
    match expr {
        Expression::Variable(v) => v.as_str() == subject || v.as_str() == object,
        Expression::Literal(_) | Expression::NamedNode(_) => true,
        Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Equal(a, b)
        | Expression::SameTerm(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::And(a, b)
        | Expression::Or(a, b) => local(a, subject, object) && local(b, subject, object),
        Expression::Not(a) | Expression::UnaryPlus(a) | Expression::UnaryMinus(a) => {
            local(a, subject, object)
        }
        Expression::FunctionCall(
            Function::IsNumeric | Function::IsLiteral | Function::IsIri | Function::Str,
            args,
        ) => args.iter().all(|a| local(a, subject, object)),
        _ => false,
    }
}
fn binds(pattern: &GraphPattern, variable: &str, mandatory: bool) -> bool {
    match pattern {
        GraphPattern::Bgp { patterns } => patterns
            .iter()
            .any(|p| triple_pattern_vars(p).iter().any(|v| v == variable)),
        GraphPattern::Graph { inner, .. } | GraphPattern::Filter { inner, .. } => {
            binds(inner, variable, mandatory)
        }
        GraphPattern::Extend {
            inner, variable: v, ..
        } => (!mandatory && v.as_str() == variable) || binds(inner, variable, mandatory),
        GraphPattern::Join { left, right } => {
            binds(left, variable, mandatory) || binds(right, variable, mandatory)
        }
        GraphPattern::Union { left, right } => {
            if mandatory {
                binds(left, variable, true) && binds(right, variable, true)
            } else {
                binds(left, variable, false) || binds(right, variable, false)
            }
        }
        GraphPattern::LeftJoin { left, right, .. } => {
            binds(left, variable, mandatory) || (!mandatory && binds(right, variable, false))
        }
        _ => false,
    }
}
fn shape<'a>(
    left: &GraphPattern,
    right: &'a GraphPattern,
    expression: Option<&Expression>,
) -> Option<(&'a str, &'a str)> {
    let (inner, guard) = if let GraphPattern::Filter { inner, expr } = right {
        (inner.as_ref(), Some(expr))
    } else {
        (right, None)
    };
    let GraphPattern::Bgp { patterns } = inner else {
        return None;
    };
    let [tp] = patterns.as_slice() else {
        return None;
    };
    let (
        TermPattern::Variable(subject),
        NamedNodePattern::NamedNode(predicate),
        TermPattern::Variable(object),
    ) = (&tp.subject, &tp.predicate, &tp.object)
    else {
        return None;
    };
    let (subject, object) = (subject.as_str(), object.as_str());
    if (guard.is_none() && expression.is_none())
        || subject == object
        || predicate.as_str() == crate::namespace::RDF_TYPE
        || !binds(left, subject, true)
        || binds(left, object, false)
        || guard
            .into_iter()
            .chain(expression)
            .any(|e| !local(e, subject, object))
    {
        return None;
    }
    Some((subject, object))
}
pub(super) fn recognized(
    left: &GraphPattern,
    right: &GraphPattern,
    expression: Option<&Expression>,
) -> bool {
    shape(left, right, expression).is_some()
}
/// Preflight the complete tree before any input is visited: no partial fallback.
pub(super) fn allowed(
    store: &Store,
    pattern: &GraphPattern,
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<bool> {
    match pattern {
        GraphPattern::Graph {
            name: NamedNodePattern::NamedNode(name),
            inner,
        } => allowed(
            store,
            inner,
            &super::count_stream::graph_context(store, name.as_str(), ctx)?,
            seed,
        ),
        GraphPattern::Filter { inner, .. } | GraphPattern::Extend { inner, .. } => {
            allowed(store, inner, ctx, seed)
        }
        GraphPattern::Union { left, right } | GraphPattern::Join { left, right } => {
            Ok(allowed(store, left, ctx, seed)? && allowed(store, right, ctx, seed)?)
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            let Some((_, object)) = shape(left, right, expression.as_ref()) else {
                return Ok(false);
            };
            let GraphScope::Named(graphs) = &ctx.graph else {
                return Ok(false);
            };
            if graphs.len() != 1
                || graphs[0] == 0
                || store.has_attachments()
                || ctx.valid_at.is_some()
                || ctx.as_of_tx.is_some()
                || seed.contains_key(object)
            {
                return Ok(false);
            }
            allowed(store, left, ctx, seed)
        }
        _ => Ok(true),
    }
}
/// Match one parent's fresh property, retaining that parent once when unmatched.
pub(super) fn visit(
    store: &Store,
    right: &GraphPattern,
    expression: Option<&Expression>,
    ctx: &TemporalContext,
    parent: &Bindings,
    emitted: &mut usize,
    emit: &mut dyn FnMut(Bindings) -> Result<()>,
) -> Result<()> {
    let mut matched = false;
    super::count_stream::visit(store, right, ctx, parent, &mut |row| {
        if match expression {
            Some(e) => eval_filter(store, e, &row, ctx)?,
            None => true,
        } {
            check_eval_budget(ctx, *emitted, *emitted)?;
            *emitted += 1;
            matched = true;
            emit(row)?;
        }
        Ok(())
    })?;
    if !matched {
        check_eval_budget(ctx, *emitted, *emitted)?;
        *emitted += 1;
        emit(parent.clone())?;
    }
    Ok(())
}
