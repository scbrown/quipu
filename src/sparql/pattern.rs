//! Pattern evaluation — BGP, triple patterns, variable binding, and join logic.

use spargebra::algebra::GraphPattern;
use spargebra::algebra::OrderExpression;
use spargebra::algebra::PropertyPathExpression;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

use super::aggregate::eval_aggregate;
use super::filter::eval_filter;
use super::rdfs::{
    collect_class_and_subclasses, eval_type_pattern_with_subclasses, is_rdf_type_pattern,
};
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
        GraphPattern::Bgp { patterns } => eval_bgp(store, patterns, ctx, seed),

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
                    let gid = store.lookup(iri.as_str())?.unwrap_or(-1);
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

/// Evaluate a basic graph pattern -- a set of triple patterns.
pub fn eval_bgp(
    store: &Store,
    patterns: &[TriplePattern],
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<(Vec<Bindings>, Vec<String>)> {
    if patterns.is_empty() {
        return Ok((vec![seed.clone()], vec![]));
    }

    let mut result_rows: Vec<Bindings> = vec![seed.clone()];
    let mut all_vars = Vec::new();

    for tp in patterns {
        let mut new_rows = Vec::new();
        for (i, existing) in result_rows.iter().enumerate() {
            // BGP accumulation multiplies row counts pattern-by-pattern —
            // the SQLite handler interrupts a grinding statement, but the
            // row-count explosion itself is only visible here.
            super::pattern_util::check_eval_budget(ctx, i, new_rows.len())?;
            let matches = eval_triple_pattern(store, tp, existing, ctx)?;
            new_rows.extend(matches);
        }
        result_rows = new_rows;

        // Track variables.
        for var in triple_pattern_vars(tp) {
            if !all_vars.contains(&var) {
                all_vars.push(var);
            }
        }
    }

    Ok((result_rows, all_vars))
}

/// Build a graph-membership SQL condition over `facts.g`, pushing the ids as
/// bound params (quipu #36). An EMPTY set yields `0 = 1` — match nothing, never
/// a silent fall-through (e.g. a `FROM NAMED` with no `FROM` has an empty
/// default graph). A single id yields `g = ?N`; several yield `g IN (…)`.
fn sql_graph_in(
    gids: &[i64],
    sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> String {
    if gids.is_empty() {
        return "0 = 1".to_string();
    }
    let placeholders: Vec<String> = gids
        .iter()
        .map(|gid| {
            sql_params.push(Box::new(*gid));
            format!("?{}", sql_params.len())
        })
        .collect();
    if placeholders.len() == 1 {
        format!("g = {}", placeholders[0])
    } else {
        format!("g IN ({})", placeholders.join(", "))
    }
}

/// Evaluate a single triple pattern against the store, extending existing bindings.
pub fn eval_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    // RDFS type-hierarchy expansion. Only on the default graph (quipu #36):
    // subclass expansion reads g=0 via rdfs.rs, so inside a named GRAPH a
    // `?s a ?C` pattern is matched LITERALLY (export wants a graph's own
    // triples, not cross-graph inference).
    if ctx.graph.is_root_default()
        && is_rdf_type_pattern(tp)
        && let TermPattern::NamedNode(class_node) = &tp.object
    {
        let class_ids = collect_class_and_subclasses(store, class_node.as_str())?;
        if !class_ids.is_empty() {
            return eval_type_pattern_with_subclasses(store, tp, bindings, &class_ids, ctx);
        }
    }

    // Build SQL query with conditions based on bound values.
    let mut conditions = Vec::new();
    let mut sql_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    // Subject
    if let Some(iri) = resolve_subject_pattern(&tp.subject, bindings) {
        if let Some(id) = store.lookup(&iri)? {
            conditions.push(format!("e = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(id));
        } else {
            return Ok(vec![]); // IRI not in dictionary -> no matches
        }
    }

    // Predicate
    if let Some(iri) = resolve_predicate_pattern(&tp.predicate, bindings) {
        if let Some(id) = store.lookup(&iri)? {
            conditions.push(format!("a = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(id));
        } else {
            return Ok(vec![]);
        }
    }

    // Object (only if it's a concrete value, not a variable)
    if let Some(value) = resolve_object_pattern(store, &tp.object, bindings)? {
        let bytes = value.to_bytes();
        conditions.push(format!("v = ?{}", sql_params.len() + 1));
        sql_params.push(Box::new(bytes));
    }

    // Temporal filtering.
    conditions.push("op = 1".to_string());
    // Graph scope (quipu #36). `Default` matches the default-graph set (service
    // default [0], or a FROM union); `Named` scopes to one graph; `AnyNamed`
    // ranges the active named graphs (all g<>0, or a FROM NAMED set) and binds
    // the graph variable per row (below).
    let bind_graph_var: Option<String> = match &ctx.graph {
        GraphScope::Default(gids) => {
            conditions.push(sql_graph_in(gids, &mut sql_params));
            None
        }
        GraphScope::Named(gid) => {
            conditions.push(format!("g = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(*gid));
            None
        }
        GraphScope::AnyNamed { var, restrict } => {
            match restrict {
                None => conditions.push("g <> 0".to_string()),
                Some(ids) => conditions.push(sql_graph_in(ids, &mut sql_params)),
            }
            Some(var.clone())
        }
    };
    // Only `GRAPH ?g` (AnyNamed) needs `g` projected — to bind ?g. The others
    // keep DISTINCT on (e,a,v), so a triple present in several graphs of a FROM
    // union collapses to ONE solution (default-graph merge), not one per graph.
    let want_g = bind_graph_var.is_some();
    if let Some(vt) = &ctx.valid_at {
        conditions.push(format!("valid_from <= ?{}", sql_params.len() + 1));
        sql_params.push(Box::new(vt.clone()));
        conditions.push(format!(
            "(valid_to IS NULL OR valid_to > ?{})",
            sql_params.len()
        ));
    } else {
        conditions.push("valid_to IS NULL".to_string());
    }
    if let Some(tx) = ctx.as_of_tx {
        conditions.push(format!("tx <= ?{}", sql_params.len() + 1));
        sql_params.push(Box::new(tx));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    // DISTINCT: a (e,a,v) triple re-asserted across transactions leaves multiple
    // current (op=1, valid_to NULL) rows; without DISTINCT the BGP yields one
    // binding per duplicate, which multiplies under joins/OPTIONAL and inflates
    // COUNT (GH#13). DISTINCT collapses them to one solution per current triple.
    // `g` is projected only for `GRAPH ?g` (to bind ?g per row); for the default
    // and single-named scopes it is omitted so DISTINCT collapses cross-graph
    // duplicates in a FROM union.
    let sql = if want_g {
        format!("SELECT DISTINCT e, a, v, g FROM facts{where_clause}")
    } else {
        format!("SELECT DISTINCT e, a, v FROM facts{where_clause}")
    };
    let mut stmt = store.prepare(&sql)?;

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        sql_params.iter().map(std::convert::AsRef::as_ref).collect();
    let mut rows = stmt.query(param_refs.as_slice())?;

    let mut results = Vec::new();
    while let Some(row) = rows.next()? {
        let e_id: i64 = row.get(0)?;
        let a_id: i64 = row.get(1)?;
        let v_bytes: Vec<u8> = row.get(2)?;
        let v = Value::from_bytes(&v_bytes)?;
        let g_id: Option<i64> = if want_g { Some(row.get(3)?) } else { None };

        let mut new_bindings = bindings.clone();
        let mut compatible = true;

        // Bind subject variable (or blank node used as join variable).
        match &tp.subject {
            TermPattern::Variable(var) => {
                let e_iri = store.resolve(e_id)?;
                let e_val = if e_iri.starts_with("_:") {
                    Value::Str(e_iri)
                } else if let Some(term_id) = store.lookup(&e_iri)? {
                    Value::Ref(term_id)
                } else {
                    Value::Str(e_iri)
                };
                if !bind_var(&mut new_bindings, var.as_str(), e_val, &mut compatible) {
                    continue;
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
                    continue;
                }
            }
            _ => {}
        }

        // Bind predicate variable.
        if let NamedNodePattern::Variable(var) = &tp.predicate {
            let a_iri = store.resolve(a_id)?;
            let a_val = if let Some(term_id) = store.lookup(&a_iri)? {
                Value::Ref(term_id)
            } else {
                Value::Str(a_iri)
            };
            if !bind_var(&mut new_bindings, var.as_str(), a_val, &mut compatible) {
                continue;
            }
        }

        // Bind object variable (or blank node used as join variable).
        match &tp.object {
            TermPattern::Variable(var) => {
                if !bind_var(&mut new_bindings, var.as_str(), v, &mut compatible) {
                    continue;
                }
            }
            TermPattern::BlankNode(b)
                if !bind_var(&mut new_bindings, b.as_str(), v, &mut compatible) =>
            {
                continue;
            }
            _ => {}
        }

        // Bind the graph variable for `GRAPH ?g { … }` (quipu #36): ?g resolves
        // to the graph's IRI (g is the interned id of that IRI). Same ?g across
        // a BGP is enforced by the join in eval_bgp via bind_var compatibility.
        if let (Some(g_var), Some(gid)) = (&bind_graph_var, g_id) {
            // g is the interned term id of the graph IRI (schema.rs), and this
            // branch only runs for named graphs (g<>0), so gid is always a
            // valid term id — bind ?g to it directly as a ref.
            if !bind_var(&mut new_bindings, g_var, Value::Ref(gid), &mut compatible) {
                continue;
            }
        }

        if compatible {
            results.push(new_bindings);
        }
    }

    Ok(results)
}

// Re-export from pattern_util for callers that import from pattern.
pub use super::pattern_util::{
    bind_var, join_rows, merge_bindings, resolve_object_pattern, resolve_predicate_pattern,
    resolve_subject_pattern, triple_pattern_vars,
};
