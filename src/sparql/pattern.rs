//! Pattern evaluation — BGP, triple patterns, variable binding, and join logic.

use std::collections::HashMap;

use spargebra::algebra::OrderExpression;
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

use super::aggregate::eval_aggregate;
use super::filter::eval_filter;
use super::rdfs::{collect_class_and_subclasses, eval_type_pattern_with_subclasses, is_rdf_type_pattern};
use super::{Bindings, TemporalContext};

/// Evaluate a graph pattern, returning rows and the variable names encountered.
pub fn eval_pattern(store: &Store, pattern: &GraphPattern, ctx: &TemporalContext) -> Result<(Vec<Bindings>, Vec<String>)> {
    match pattern {
        GraphPattern::Bgp { patterns } => eval_bgp(store, patterns, ctx),

        GraphPattern::Join { left, right } => {
            let (left_rows, left_vars) = eval_pattern(store, left, ctx)?;
            let (right_rows, right_vars) = eval_pattern(store, right, ctx)?;
            let joined = join_rows(&left_rows, &right_rows);
            let mut vars = left_vars;
            for v in right_vars {
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            Ok((joined, vars))
        }

        GraphPattern::Filter { expr, inner } => {
            let (rows, vars) = eval_pattern(store, inner, ctx)?;
            let filtered = rows
                .into_iter()
                .filter(|row| eval_filter(store, expr, row))
                .collect();
            Ok((filtered, vars))
        }

        GraphPattern::Project { inner, variables } => {
            let (rows, _) = eval_pattern(store, inner, ctx)?;
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
            let (rows, vars) = eval_pattern(store, inner, ctx)?;
            let mut seen = Vec::new();
            let mut unique = Vec::new();
            for row in rows {
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
            let (rows, vars) = eval_pattern(store, inner, ctx)?;
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
            let (left_rows, left_vars) = eval_pattern(store, left, ctx)?;
            let (right_rows, right_vars) = eval_pattern(store, right, ctx)?;
            let mut vars = left_vars;
            for v in &right_vars {
                if !vars.contains(v) {
                    vars.push(v.clone());
                }
            }
            let mut results = Vec::new();
            for l in &left_rows {
                let mut matched = false;
                for r in &right_rows {
                    if let Some(merged) = merge_bindings(l, r) {
                        let passes = expression
                            .as_ref()
                            .map(|e| eval_filter(store, e, &merged))
                            .unwrap_or(true);
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
            let (mut left_rows, left_vars) = eval_pattern(store, left, ctx)?;
            let (right_rows, right_vars) = eval_pattern(store, right, ctx)?;
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
            let (mut rows, vars) = eval_pattern(store, inner, ctx)?;
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

        GraphPattern::Reduced { inner } => eval_pattern(store, inner, ctx),

        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            let (rows, _) = eval_pattern(store, inner, ctx)?;
            let group_keys: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();
            let agg_vars: Vec<String> = aggregates.iter().map(|(v, _)| v.as_str().to_string()).collect();

            // Group rows by the group-by variables.
            let mut groups: Vec<(Vec<Option<Value>>, Vec<Bindings>)> = Vec::new();
            for row in &rows {
                let key: Vec<Option<Value>> = group_keys.iter().map(|k| row.get(k).cloned()).collect();
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
            let (rows, mut vars) = eval_pattern(store, inner, ctx)?;
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

        _ => Err(Error::InvalidValue(format!(
            "unsupported graph pattern: {pattern}"
        ))),
    }
}

/// Evaluate a basic graph pattern -- a set of triple patterns.
pub fn eval_bgp(store: &Store, patterns: &[TriplePattern], ctx: &TemporalContext) -> Result<(Vec<Bindings>, Vec<String>)> {
    if patterns.is_empty() {
        return Ok((vec![HashMap::new()], vec![]));
    }

    let mut result_rows: Vec<Bindings> = vec![HashMap::new()];
    let mut all_vars = Vec::new();

    for tp in patterns {
        let mut new_rows = Vec::new();
        for existing in &result_rows {
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

/// Evaluate a single triple pattern against the store, extending existing bindings.
pub fn eval_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    // RDFS type-hierarchy expansion
    if is_rdf_type_pattern(tp)
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

    let sql = format!("SELECT e, a, v FROM facts{where_clause}");
    let mut stmt = store.prepare(&sql)?;

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = sql_params.iter().map(|p| p.as_ref()).collect();
    let mut rows = stmt.query(param_refs.as_slice())?;

    let mut results = Vec::new();
    while let Some(row) = rows.next()? {
        let e_id: i64 = row.get(0)?;
        let a_id: i64 = row.get(1)?;
        let v_bytes: Vec<u8> = row.get(2)?;
        let v = Value::from_bytes(&v_bytes)?;

        let mut new_bindings = bindings.clone();
        let mut compatible = true;

        // Bind subject variable.
        if let TermPattern::Variable(var) = &tp.subject {
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

        // Bind object variable.
        if let TermPattern::Variable(var) = &tp.object
            && !bind_var(&mut new_bindings, var.as_str(), v, &mut compatible)
        {
            continue;
        }

        if compatible {
            results.push(new_bindings);
        }
    }

    Ok(results)
}

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
        _ => None,
    }
}

/// Resolve a predicate pattern to an IRI string if it's bound.
pub fn resolve_predicate_pattern(pattern: &NamedNodePattern, bindings: &Bindings) -> Option<String> {
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
        _ => Ok(None),
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
