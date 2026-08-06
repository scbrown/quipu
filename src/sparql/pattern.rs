//! SPARQL algebra evaluation — joins, FILTER, projection, GRAPH scoping.
//!
//! The leaf of the evaluator (BGP and single-triple-pattern matching against
//! the fact store) lives in [`super::triple`]; this module is the algebra over
//! the rows those leaves produce.

use spargebra::algebra::GraphPattern;
use spargebra::algebra::OrderExpression;
use spargebra::algebra::PropertyPathExpression;
use spargebra::term::{NamedNodePattern, TermPattern};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

use super::aggregate::eval_aggregate;
use super::filter::eval_filter;
use super::triple::eval_bgp as eval_bgp_inner;
use super::values::eval_values;
use super::{Bindings, GraphScope, TemporalContext};

/// Evaluate a graph pattern, returning rows and the variable names encountered.
pub fn eval_pattern(
    store: &Store,
    pattern: &GraphPattern,
    ctx: &TemporalContext,
) -> Result<(Vec<Bindings>, Vec<String>)> {
    eval_pattern_seeded(store, pattern, ctx, &Bindings::new())
}

/// Evaluate a graph pattern SEEDED with pre-bound variables (`seed`), which
/// initialise every leaf (BGP / property path) so evaluation is CONSTRAINED by
/// them — the SPARQL EXISTS substitution semantics. A bound ?s makes a path
/// traverse only from that ?s, not the whole graph (the unseeded
/// per-outer-row re-eval held the store mutex O(n x `inner_eval`)). An empty seed
/// reproduces the original unconstrained evaluation exactly. The deadline check
/// stays here so seeded recursion is budget-bounded like the wrapper was.
pub fn eval_pattern_seeded(
    store: &Store,
    pattern: &GraphPattern,
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<(Vec<Bindings>, Vec<String>)> {
    // Deadline check between operators. The SQLite progress
    // handler stops a grinding statement; this stops the Rust-side plan —
    // every operator recurses through here, so a multi-join over huge
    // intermediate rows cannot outlive the budget by more than one operator.
    // The zeros are placeholders: `query_temporal` rewrites any past-deadline
    // failure with the real elapsed/limit.
    if ctx
        .deadline
        .is_some_and(|dl| std::time::Instant::now() >= dl)
    {
        return Err(crate::error::Error::QueryTimeout {
            elapsed_ms: 0,
            limit_ms: 0,
        });
    }
    match pattern {
        GraphPattern::Bgp { patterns } => eval_bgp_inner(store, patterns, ctx, seed),

        // quipu #51: `VALUES` is an inline relation — a literal table of rows.
        // It is a leaf like a BGP, so it merges with the seed and joins with
        // the surrounding group through the ordinary Join/LeftJoin arms.
        GraphPattern::Values {
            variables,
            bindings,
        } => eval_values(store, variables, bindings, seed),

        GraphPattern::Join { left, right } => {
            let (left_rows, left_vars) = eval_pattern_seeded(store, left, ctx, seed)?;
            let (right_rows, right_vars) = eval_pattern_seeded(store, right, ctx, seed)?;
            let joined = join_rows(&left_rows, &right_rows, ctx)?;
            let mut vars = left_vars;
            for v in right_vars {
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            Ok((joined, vars))
        }

        GraphPattern::Filter { expr, inner } => {
            let (rows, vars) = eval_pattern_seeded(store, inner, ctx, seed)?;
            let mut filtered = Vec::with_capacity(rows.len());
            for (i, row) in rows.into_iter().enumerate() {
                // A pure-Rust filter over pre-materialized rows touches
                // neither SQLite (no progress handler) nor another operator
                // (no entry check) — poll the deadline in-loop, cheaply.
                if i % 1024 == 0
                    && ctx
                        .deadline
                        .is_some_and(|dl| std::time::Instant::now() >= dl)
                {
                    return Err(crate::error::Error::QueryTimeout {
                        elapsed_ms: 0,
                        limit_ms: 0,
                    });
                }
                if eval_filter(store, expr, &row, ctx)? {
                    filtered.push(row);
                }
            }
            Ok((filtered, vars))
        }

        GraphPattern::Project { inner, variables } => {
            let (rows, _) = eval_pattern_seeded(store, inner, ctx, seed)?;
            let var_names: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();
            let projected: Vec<Bindings> = rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .filter(|(k, _)| var_names.contains(k))
                        .collect()
                })
                .collect();
            Ok((projected, var_names))
        }

        GraphPattern::Distinct { inner } => {
            let (rows, vars) = eval_pattern_seeded(store, inner, ctx, seed)?;
            let mut seen = Vec::new();
            let mut unique = Vec::new();
            // `seen.contains` makes this loop quadratic in the row count —
            // over a large (row-cap-sized) input it is another pure-Rust burn
            // neither the SQLite handler nor the operator check can see.
            for (i, row) in rows.into_iter().enumerate() {
                super::pattern_util::check_eval_budget(ctx, i, unique.len())?;
                if !seen.contains(&row) {
                    seen.push(row.clone());
                    unique.push(row);
                }
            }
            Ok((unique, vars))
        }

        GraphPattern::Slice {
            inner,
            start,
            length,
        } => {
            let (rows, vars) = eval_pattern_seeded(store, inner, ctx, seed)?;
            let sliced: Vec<Bindings> = match length {
                Some(len) => rows.into_iter().skip(*start).take(*len).collect(),
                None => rows.into_iter().skip(*start).collect(),
            };
            Ok((sliced, vars))
        }

        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            let (left_rows, left_vars) = eval_pattern_seeded(store, left, ctx, seed)?;
            let (right_rows, right_vars) = eval_pattern_seeded(store, right, ctx, seed)?;
            let mut vars = left_vars;
            for v in &right_vars {
                if !vars.contains(v) {
                    vars.push(v.clone());
                }
            }
            let mut results = Vec::new();
            // Same nested join loop as `join_rows` — same budget enforcement,
            // for the same reason (OPTIONAL explodes exactly like Join).
            let mut i = 0usize;
            for l in &left_rows {
                let mut matched = false;
                for r in &right_rows {
                    super::pattern_util::check_eval_budget(ctx, i, results.len())?;
                    i += 1;
                    if let Some(merged) = merge_bindings(l, r) {
                        let passes = match expression.as_ref() {
                            Some(e) => eval_filter(store, e, &merged, ctx)?,
                            None => true,
                        };
                        if passes {
                            results.push(merged);
                            matched = true;
                        }
                    }
                }
                if !matched {
                    results.push(l.clone());
                }
            }
            Ok((results, vars))
        }

        GraphPattern::Union { left, right } => {
            let (mut left_rows, left_vars) = eval_pattern_seeded(store, left, ctx, seed)?;
            let (right_rows, right_vars) = eval_pattern_seeded(store, right, ctx, seed)?;
            left_rows.extend(right_rows);
            let mut vars = left_vars;
            for v in right_vars {
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            Ok((left_rows, vars))
        }

        GraphPattern::OrderBy { inner, expression } => {
            let (mut rows, vars) = eval_pattern_seeded(store, inner, ctx, seed)?;
            rows.sort_by(|a, b| {
                for ord_expr in expression {
                    let (expr, ascending) = match ord_expr {
                        OrderExpression::Asc(e) => (e, true),
                        OrderExpression::Desc(e) => (e, false),
                    };
                    let va = super::filter::eval_expr(store, expr, a);
                    let vb = super::filter::eval_expr(store, expr, b);
                    let cmp = super::aggregate::compare_option_values(&va, &vb);
                    let cmp = if ascending { cmp } else { cmp.reverse() };
                    if cmp != std::cmp::Ordering::Equal {
                        return cmp;
                    }
                }
                std::cmp::Ordering::Equal
            });
            Ok((rows, vars))
        }

        GraphPattern::Reduced { inner } => eval_pattern_seeded(store, inner, ctx, seed),

        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            let (rows, _) = eval_pattern_seeded(store, inner, ctx, seed)?;
            let group_keys: Vec<String> =
                variables.iter().map(|v| v.as_str().to_string()).collect();
            let agg_vars: Vec<String> = aggregates
                .iter()
                .map(|(v, _)| v.as_str().to_string())
                .collect();

            // Group rows by the group-by variables.
            let mut groups: Vec<(Vec<Option<Value>>, Vec<Bindings>)> = Vec::new();
            for row in &rows {
                let key: Vec<Option<Value>> =
                    group_keys.iter().map(|k| row.get(k).cloned()).collect();
                if let Some(group) = groups.iter_mut().find(|(k, _)| k == &key) {
                    group.1.push(row.clone());
                } else {
                    groups.push((key, vec![row.clone()]));
                }
            }

            // If no group keys, all rows form a single group.
            if group_keys.is_empty() && groups.is_empty() {
                groups.push((vec![], rows));
            }

            let mut result_rows = Vec::new();
            for (key, group_rows) in &groups {
                let mut result_row = Bindings::new();

                // Set group-by variable bindings.
                for (i, var) in group_keys.iter().enumerate() {
                    if let Some(val) = &key[i] {
                        result_row.insert(var.clone(), val.clone());
                    }
                }

                // Compute aggregates.
                for (i, (_, agg_expr)) in aggregates.iter().enumerate() {
                    let agg_val = eval_aggregate(store, agg_expr, group_rows);
                    result_row.insert(agg_vars[i].clone(), agg_val);
                }

                result_rows.push(result_row);
            }

            let mut vars = group_keys;
            vars.extend(agg_vars);
            Ok((result_rows, vars))
        }

        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => {
            let (rows, mut vars) = eval_pattern_seeded(store, inner, ctx, seed)?;
            let var_name = variable.as_str().to_string();
            let extended: Vec<Bindings> = rows
                .into_iter()
                .map(|mut row| {
                    if let Some(val) = super::filter::eval_expr(store, expression, &row) {
                        row.insert(var_name.clone(), val);
                    }
                    row
                })
                .collect();
            if !vars.contains(&var_name) {
                vars.push(var_name);
            }
            Ok((extended, vars))
        }

        GraphPattern::Path {
            subject,
            path,
            object,
        } => {
            // quipu #36 follow-up: property paths are not yet graph-scoped (the
            // path scan reads g=0). Fail loud inside a named GRAPH rather than
            // silently returning default-graph results.
            if !ctx.graph.is_root_default() {
                return Err(Error::InvalidValue(
                    "property paths are only supported on the ROOT default graph \
                     (quipu #36 follow-up); inside a named GRAPH or a FROM-redefined \
                     default graph the path would read the wrong graph"
                        .to_string(),
                ));
            }
            eval_path(store, subject, path, object, ctx, seed)
        }

        // quipu #36: `GRAPH <iri> { … }` / `GRAPH ?g { … }` re-scope the enclosed
        // patterns to a named graph. Clone the context so the outer scope is
        // restored when this block returns; nested Join/Union/Filter propagate
        // the scope by threading the cloned ctx.
        GraphPattern::Graph { name, inner } => {
            let mut scoped = ctx.clone();
            scoped.graph = match name {
                // Unknown graph IRI -> an id that matches nothing, never a
                // silent fall-through. A FROM NAMED restriction that excludes
                // this graph likewise makes it invisible (id -1).
                NamedNodePattern::NamedNode(iri) => {
                    let gid = match store.lookup(iri.as_str())? {
                        Some(g) => g,
                        None => {
                            // quipu #75: on a COMPOSED store, "unknown locally"
                            // may mean "lives in an attached layer", which is a
                            // real graph and not a typo. Refuse rather than
                            // match nothing; unattached stores are untouched.
                            store.refuse_if_attached_only(iri.as_str())?;
                            -1
                        }
                    };
                    let visible = ctx
                        .named_dataset
                        .as_ref()
                        .is_none_or(|set| set.contains(&gid));
                    GraphScope::Named(if visible { gid } else { -1 })
                }
                // GRAPH ?g ranges the active named graphs — all of them, or the
                // FROM NAMED restriction when the query set one.
                NamedNodePattern::Variable(v) => GraphScope::AnyNamed {
                    var: v.as_str().to_string(),
                    restrict: ctx.named_dataset.clone(),
                },
            };
            let (rows, mut vars) = eval_pattern_seeded(store, inner, &scoped, seed)?;
            if let NamedNodePattern::Variable(v) = name {
                let g_var = v.as_str().to_string();
                if !vars.contains(&g_var) {
                    vars.push(g_var);
                }
            }
            Ok((rows, vars))
        }

        _ => Err(Error::InvalidValue(format!(
            "unsupported graph pattern: {pattern}"
        ))),
    }
}

/// Evaluate a property path pattern (SPARQL 1.1 property paths).
fn eval_path(
    store: &Store,
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<(Vec<Bindings>, Vec<String>)> {
    use super::property_path::{eval_path_pattern, path_pattern_vars};

    let seed_rows = vec![seed.clone()];
    let mut all_rows = Vec::new();
    for existing in &seed_rows {
        let rows = eval_path_pattern(store, subject, path, object, existing, ctx)?;
        all_rows.extend(rows);
    }
    let vars = path_pattern_vars(subject, object);
    Ok((all_rows, vars))
}

// Re-export from pattern_util for callers that import from pattern.
pub use super::pattern_util::{
    bind_var, join_rows, merge_bindings, resolve_object_pattern, resolve_predicate_pattern,
    resolve_subject_pattern, triple_pattern_vars,
};
// Re-export the store-facing leaf so `pattern::eval_bgp` keeps resolving.
pub use super::triple::{eval_bgp, eval_triple_pattern};
