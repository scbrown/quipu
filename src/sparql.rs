//! SPARQL query engine — evaluates SPARQL queries over the EAVT fact log.
//!
//! Parses SPARQL via spargebra, then evaluates against the SQLite fact store.
//! Currently supports: SELECT queries with basic graph patterns (BGP),
//! FILTER (comparison, BOUND, regex), PROJECT, DISTINCT, LIMIT/OFFSET.

use std::collections::HashMap;

use oxrdf::Literal;
use spargebra::algebra::{AggregateExpression, AggregateFunction, Expression, GraphPattern, OrderExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

/// A single row of variable bindings from a query result.
pub type Bindings = HashMap<String, Value>;

/// Result of a SPARQL SELECT query.
#[derive(Debug)]
pub struct QueryResult {
    /// Ordered variable names from the SELECT clause.
    pub variables: Vec<String>,
    /// Result rows, each mapping variable names to values.
    pub rows: Vec<Bindings>,
}

/// Execute a SPARQL SELECT query against the store.
pub fn query(store: &Store, sparql: &str) -> Result<QueryResult> {
    let parsed = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::InvalidValue(format!("SPARQL parse error: {e}")))?;

    match parsed {
        Query::Select { pattern, .. } => eval_select(store, &pattern),
        _ => Err(Error::InvalidValue(
            "only SELECT queries are currently supported".into(),
        )),
    }
}

/// Evaluate a SELECT query's graph pattern.
fn eval_select(store: &Store, pattern: &GraphPattern) -> Result<QueryResult> {
    let (rows, vars) = eval_pattern(store, pattern)?;
    Ok(QueryResult {
        variables: vars,
        rows,
    })
}

/// Evaluate a graph pattern, returning rows and the variable names encountered.
fn eval_pattern(store: &Store, pattern: &GraphPattern) -> Result<(Vec<Bindings>, Vec<String>)> {
    match pattern {
        GraphPattern::Bgp { patterns } => eval_bgp(store, patterns),

        GraphPattern::Join { left, right } => {
            let (left_rows, left_vars) = eval_pattern(store, left)?;
            let (right_rows, right_vars) = eval_pattern(store, right)?;
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
            let (rows, vars) = eval_pattern(store, inner)?;
            let filtered = rows
                .into_iter()
                .filter(|row| eval_filter(store, expr, row))
                .collect();
            Ok((filtered, vars))
        }

        GraphPattern::Project { inner, variables } => {
            let (rows, _) = eval_pattern(store, inner)?;
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
            let (rows, vars) = eval_pattern(store, inner)?;
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
            let (rows, vars) = eval_pattern(store, inner)?;
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
            let (left_rows, left_vars) = eval_pattern(store, left)?;
            let (right_rows, right_vars) = eval_pattern(store, right)?;
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
            let (mut left_rows, left_vars) = eval_pattern(store, left)?;
            let (right_rows, right_vars) = eval_pattern(store, right)?;
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
            let (mut rows, vars) = eval_pattern(store, inner)?;
            rows.sort_by(|a, b| {
                for ord_expr in expression {
                    let (expr, ascending) = match ord_expr {
                        OrderExpression::Asc(e) => (e, true),
                        OrderExpression::Desc(e) => (e, false),
                    };
                    let va = eval_expr(store, expr, a);
                    let vb = eval_expr(store, expr, b);
                    let cmp = compare_option_values(&va, &vb);
                    let cmp = if ascending { cmp } else { cmp.reverse() };
                    if cmp != std::cmp::Ordering::Equal {
                        return cmp;
                    }
                }
                std::cmp::Ordering::Equal
            });
            Ok((rows, vars))
        }

        GraphPattern::Reduced { inner } => eval_pattern(store, inner),

        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            let (rows, _) = eval_pattern(store, inner)?;
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
            let (rows, mut vars) = eval_pattern(store, inner)?;
            let var_name = variable.as_str().to_string();
            let extended: Vec<Bindings> = rows
                .into_iter()
                .map(|mut row| {
                    if let Some(val) = eval_expr(store, expression, &row) {
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

/// Evaluate a basic graph pattern — a set of triple patterns.
fn eval_bgp(store: &Store, patterns: &[TriplePattern]) -> Result<(Vec<Bindings>, Vec<String>)> {
    if patterns.is_empty() {
        return Ok((vec![HashMap::new()], vec![]));
    }

    let mut result_rows: Vec<Bindings> = vec![HashMap::new()];
    let mut all_vars = Vec::new();

    for tp in patterns {
        let mut new_rows = Vec::new();
        for existing in &result_rows {
            let matches = eval_triple_pattern(store, tp, existing)?;
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
fn eval_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
) -> Result<Vec<Bindings>> {
    // ── RDFS type-hierarchy expansion ────────────────────────────
    // If the pattern is `?x a <SomeClass>`, expand to also match
    // instances of all subclasses of SomeClass.
    if is_rdf_type_pattern(tp) {
        if let TermPattern::NamedNode(class_node) = &tp.object {
            let class_ids = collect_class_and_subclasses(store, class_node.as_str())?;
            if !class_ids.is_empty() {
                return eval_type_pattern_with_subclasses(store, tp, bindings, &class_ids);
            }
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
            return Ok(vec![]); // IRI not in dictionary → no matches
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

    // Always filter to current state.
    conditions.push("op = 1".to_string());
    conditions.push("valid_to IS NULL".to_string());

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
fn bind_var(bindings: &mut Bindings, var: &str, value: Value, compatible: &mut bool) -> bool {
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
fn resolve_subject_pattern(pattern: &TermPattern, bindings: &Bindings) -> Option<String> {
    match pattern {
        TermPattern::NamedNode(n) => Some(n.as_str().to_string()),
        TermPattern::BlankNode(b) => Some(format!("_:{}", b.as_str())),
        TermPattern::Variable(v) => match bindings.get(v.as_str()) {
            Some(Value::Ref(_)) => None, // We'd need store to resolve, skip for now
            _ => None,
        },
        _ => None,
    }
}

/// Resolve a predicate pattern to an IRI string if it's bound.
fn resolve_predicate_pattern(pattern: &NamedNodePattern, bindings: &Bindings) -> Option<String> {
    match pattern {
        NamedNodePattern::NamedNode(n) => Some(n.as_str().to_string()),
        NamedNodePattern::Variable(v) => match bindings.get(v.as_str()) {
            Some(Value::Ref(_)) => None, // Can't resolve without store
            _ => None,
        },
    }
}

/// Resolve an object pattern to a Value if it's a concrete term.
fn resolve_object_pattern(
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
        TermPattern::Literal(lit) => Ok(Some(literal_to_value(lit))),
        TermPattern::Variable(v) => {
            // If already bound, use that value.
            Ok(bindings.get(v.as_str()).cloned())
        }
        _ => Ok(None),
    }
}

/// Convert an oxrdf Literal to a Value (same logic as rdf module).
fn literal_to_value(lit: &Literal) -> Value {
    let dt = lit.datatype().as_str();
    match dt {
        "http://www.w3.org/2001/XMLSchema#integer"
        | "http://www.w3.org/2001/XMLSchema#long"
        | "http://www.w3.org/2001/XMLSchema#int" => {
            if let Ok(n) = lit.value().parse::<i64>() {
                Value::Int(n)
            } else {
                Value::Str(lit.value().to_string())
            }
        }
        "http://www.w3.org/2001/XMLSchema#double"
        | "http://www.w3.org/2001/XMLSchema#float"
        | "http://www.w3.org/2001/XMLSchema#decimal" => {
            if let Ok(f) = lit.value().parse::<f64>() {
                Value::Float(f)
            } else {
                Value::Str(lit.value().to_string())
            }
        }
        "http://www.w3.org/2001/XMLSchema#boolean" => {
            Value::Bool(matches!(lit.value(), "true" | "1"))
        }
        _ => {
            if let Some(lang) = lit.language() {
                Value::Str(format!("{}@{}", lit.value(), lang))
            } else {
                Value::Str(lit.value().to_string())
            }
        }
    }
}

/// Get all variable names from a triple pattern.
fn triple_pattern_vars(tp: &TriplePattern) -> Vec<String> {
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
fn join_rows(left: &[Bindings], right: &[Bindings]) -> Vec<Bindings> {
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
fn merge_bindings(a: &Bindings, b: &Bindings) -> Option<Bindings> {
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

/// Evaluate a FILTER expression against a binding row.
fn eval_filter(store: &Store, expr: &Expression, row: &Bindings) -> bool {
    match expr {
        Expression::Equal(left, right) => {
            match (eval_expr(store, left, row), eval_expr(store, right, row)) {
                (Some(l), Some(r)) => l == r,
                _ => false,
            }
        }
        Expression::Greater(left, right) => compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Greater
        }),
        Expression::GreaterOrEqual(left, right) => compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Greater || o == std::cmp::Ordering::Equal
        }),
        Expression::Less(left, right) => {
            compare_values(store, left, right, row, |o| o == std::cmp::Ordering::Less)
        }
        Expression::LessOrEqual(left, right) => compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Less || o == std::cmp::Ordering::Equal
        }),
        Expression::And(left, right) => {
            eval_filter(store, left, row) && eval_filter(store, right, row)
        }
        Expression::Or(left, right) => {
            eval_filter(store, left, row) || eval_filter(store, right, row)
        }
        Expression::Not(inner) => !eval_filter(store, inner, row),
        Expression::Bound(var) => row.contains_key(var.as_str()),
        Expression::FunctionCall(
            spargebra::algebra::Function::Regex,
            args,
        ) => {
            if args.len() >= 2 {
                if let (Some(Value::Str(text)), Some(Value::Str(pattern))) =
                    (eval_expr(store, &args[0], row), eval_expr(store, &args[1], row))
                {
                    // Simple regex: just check contains for now.
                    text.contains(&pattern)
                } else {
                    false
                }
            } else {
                false
            }
        }
        _ => true, // Unknown expressions pass through.
    }
}

/// Evaluate an expression to a Value.
fn eval_expr(store: &Store, expr: &Expression, row: &Bindings) -> Option<Value> {
    match expr {
        Expression::Variable(var) => row.get(var.as_str()).cloned(),
        Expression::NamedNode(n) => {
            store.lookup(n.as_str()).ok().flatten().map(Value::Ref)
        }
        Expression::Literal(lit) => Some(literal_to_value(lit)),
        _ => None,
    }
}

/// Compare two expressions with an ordering predicate.
fn compare_values(
    store: &Store,
    left: &Expression,
    right: &Expression,
    row: &Bindings,
    pred: impl Fn(std::cmp::Ordering) -> bool,
) -> bool {
    match (eval_expr(store, left, row), eval_expr(store, right, row)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => pred(a.cmp(&b)),
        (Some(Value::Float(a)), Some(Value::Float(b))) => {
            a.partial_cmp(&b).is_some_and(&pred)
        }
        (Some(Value::Int(a)), Some(Value::Float(b))) => {
            (a as f64).partial_cmp(&b).is_some_and(&pred)
        }
        (Some(Value::Float(a)), Some(Value::Int(b))) => {
            a.partial_cmp(&(b as f64)).is_some_and(&pred)
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => pred(a.cmp(&b)),
        _ => false,
    }
}

// ── Aggregate evaluation ────────────────────────────────────────

/// Evaluate an aggregate expression over a group of rows.
fn eval_aggregate(store: &Store, agg: &AggregateExpression, rows: &[Bindings]) -> Value {
    match agg {
        AggregateExpression::CountSolutions { distinct } => {
            if *distinct {
                let mut seen: Vec<&Bindings> = Vec::new();
                let count = rows.iter().filter(|r| {
                    if seen.contains(r) { false } else { seen.push(r); true }
                }).count();
                Value::Int(count as i64)
            } else {
                Value::Int(rows.len() as i64)
            }
        }
        AggregateExpression::FunctionCall { name, expr, distinct } => {
            let mut values: Vec<Value> = rows
                .iter()
                .filter_map(|row| eval_expr(store, expr, row))
                .collect();
            if *distinct {
                let mut deduped = Vec::new();
                for v in values {
                    if !deduped.contains(&v) {
                        deduped.push(v);
                    }
                }
                values = deduped;
            }
            match name {
                AggregateFunction::Count => Value::Int(values.len() as i64),
                AggregateFunction::Sum => {
                    let mut sum = 0.0f64;
                    let mut all_int = true;
                    for v in &values {
                        match v {
                            Value::Int(n) => sum += *n as f64,
                            Value::Float(f) => { sum += f; all_int = false; }
                            _ => {}
                        }
                    }
                    if all_int { Value::Int(sum as i64) } else { Value::Float(sum) }
                }
                AggregateFunction::Avg => {
                    if values.is_empty() { return Value::Int(0); }
                    let mut sum = 0.0f64;
                    let mut count = 0usize;
                    for v in &values {
                        match v {
                            Value::Int(n) => { sum += *n as f64; count += 1; }
                            Value::Float(f) => { sum += f; count += 1; }
                            _ => {}
                        }
                    }
                    if count == 0 { Value::Int(0) } else { Value::Float(sum / count as f64) }
                }
                AggregateFunction::Min => {
                    values.into_iter().reduce(|a, b| {
                        if compare_option_values(&Some(a.clone()), &Some(b.clone())) == std::cmp::Ordering::Less { a } else { b }
                    }).unwrap_or(Value::Int(0))
                }
                AggregateFunction::Max => {
                    values.into_iter().reduce(|a, b| {
                        if compare_option_values(&Some(a.clone()), &Some(b.clone())) == std::cmp::Ordering::Greater { a } else { b }
                    }).unwrap_or(Value::Int(0))
                }
                AggregateFunction::Sample => {
                    values.into_iter().next().unwrap_or(Value::Int(0))
                }
                AggregateFunction::GroupConcat { separator } => {
                    let sep = separator.as_deref().unwrap_or(" ");
                    let strs: Vec<String> = values.iter().map(|v| match v {
                        Value::Str(s) => s.clone(),
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => String::new(),
                    }).collect();
                    Value::Str(strs.join(sep))
                }
                AggregateFunction::Custom(_) => Value::Int(0),
            }
        }
    }
}

// ── RDFS type-hierarchy helpers ──────────────────────────────────

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

/// Check if a triple pattern has rdf:type as predicate and a concrete class as object.
fn is_rdf_type_pattern(tp: &TriplePattern) -> bool {
    matches!(&tp.predicate, NamedNodePattern::NamedNode(n) if n.as_str() == RDF_TYPE)
        && matches!(&tp.object, TermPattern::NamedNode(_))
}

/// Collect a class and all its subclasses (transitive) from the fact log.
///
/// Uses rdfs:subClassOf triples: `SubClass rdfs:subClassOf SuperClass`.
/// Returns the term IDs of the class and all subclasses.
fn collect_class_and_subclasses(store: &Store, class_iri: &str) -> Result<Vec<i64>> {
    let class_id = match store.lookup(class_iri)? {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    let subclass_pred = match store.lookup(RDFS_SUBCLASS_OF)? {
        Some(id) => id,
        None => return Ok(vec![class_id]), // No subClassOf pred → just the class itself
    };

    // BFS to find all subclasses.
    let mut result = vec![class_id];
    let mut frontier = vec![class_id];

    while !frontier.is_empty() {
        let mut next_frontier = Vec::new();
        for super_id in &frontier {
            // Find all X where X rdfs:subClassOf super_id (as a Ref value)
            let target_bytes = Value::Ref(*super_id).to_bytes();
            let mut stmt = store.prepare(
                "SELECT e FROM facts WHERE a = ?1 AND v = ?2 AND op = 1 AND valid_to IS NULL",
            )?;
            let mut rows = stmt.query(rusqlite::params![subclass_pred, target_bytes])?;
            while let Some(row) = rows.next()? {
                let sub_id: i64 = row.get(0)?;
                if !result.contains(&sub_id) {
                    result.push(sub_id);
                    next_frontier.push(sub_id);
                }
            }
        }
        frontier = next_frontier;
    }

    Ok(result)
}

/// Evaluate a `?x a <Class>` pattern with subclass expansion.
fn eval_type_pattern_with_subclasses(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    class_ids: &[i64],
) -> Result<Vec<Bindings>> {
    let type_pred_id = match store.lookup(RDF_TYPE)? {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    // Subject filter (if bound).
    let subject_id = if let Some(iri) = resolve_subject_pattern(&tp.subject, bindings) {
        match store.lookup(&iri)? {
            Some(id) => Some(id),
            None => return Ok(vec![]),
        }
    } else {
        None
    };

    let mut results = Vec::new();

    // For each class in the hierarchy, find instances.
    for class_id in class_ids {
        let v_bytes = Value::Ref(*class_id).to_bytes();
        let sql = if subject_id.is_some() {
            "SELECT e, v FROM facts WHERE a = ?1 AND v = ?2 AND e = ?3 AND op = 1 AND valid_to IS NULL"
        } else {
            "SELECT e, v FROM facts WHERE a = ?1 AND v = ?2 AND op = 1 AND valid_to IS NULL"
        };

        let mut stmt = store.prepare(sql)?;
        let mut rows = if let Some(sid) = subject_id {
            stmt.query(rusqlite::params![type_pred_id, v_bytes, sid])?
        } else {
            stmt.query(rusqlite::params![type_pred_id, v_bytes])?
        };

        while let Some(row) = rows.next()? {
            let e_id: i64 = row.get(0)?;
            let v_bytes_row: Vec<u8> = row.get(1)?;
            let v = Value::from_bytes(&v_bytes_row)?;

            let mut new_bindings = bindings.clone();
            let mut compatible = true;

            // Bind subject variable.
            if let TermPattern::Variable(var) = &tp.subject {
                let e_iri = store.resolve(e_id)?;
                let e_val = if let Some(term_id) = store.lookup(&e_iri)? {
                    Value::Ref(term_id)
                } else {
                    Value::Str(e_iri)
                };
                if !bind_var(&mut new_bindings, var.as_str(), e_val, &mut compatible) {
                    continue;
                }
            }

            // Bind object variable (always the matched class).
            if let TermPattern::Variable(var) = &tp.object {
                if !bind_var(&mut new_bindings, var.as_str(), v, &mut compatible) {
                    continue;
                }
            }

            if compatible {
                results.push(new_bindings);
            }
        }
    }

    Ok(results)
}

/// Compare two optional Values for ordering (used by ORDER BY).
fn compare_option_values(a: &Option<Value>, b: &Option<Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(va), Some(vb)) => match (va, vb) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Ref(a), Value::Ref(b)) => a.cmp(b),
            _ => std::cmp::Ordering::Equal,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;
    use oxrdfio::RdfFormat;

    fn test_store_with_data() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "30"^^xsd:integer ;
    ex:knows ex:bob .

ex:bob a ex:Person ;
    ex:name "Bob" ;
    ex:age "25"^^xsd:integer ;
    ex:knows ex:alice .

ex:carol a ex:Employee ;
    ex:name "Carol" ;
    ex:age "35"^^xsd:integer .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    #[test]
    fn select_all_triples() {
        let store = test_store_with_data();
        let result = query(
            &store,
            "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",
        )
        .unwrap();

        assert_eq!(result.variables, vec!["s", "p", "o"]);
        // 4 for alice + 4 for bob + 3 for carol = 11
        assert_eq!(result.rows.len(), 11);
    }

    #[test]
    fn select_with_bound_predicate() {
        let store = test_store_with_data();
        let result = query(
            &store,
            "SELECT ?s ?name WHERE { ?s <http://example.org/name> ?name }",
        )
        .unwrap();

        assert_eq!(result.variables, vec!["s", "name"]);
        assert_eq!(result.rows.len(), 3);

        let names: Vec<&Value> = result
            .rows
            .iter()
            .map(|r| r.get("name").unwrap())
            .collect();
        assert!(names.contains(&&Value::Str("Alice".into())));
        assert!(names.contains(&&Value::Str("Bob".into())));
        assert!(names.contains(&&Value::Str("Carol".into())));
    }

    #[test]
    fn select_with_bound_subject() {
        let store = test_store_with_data();
        let result = query(
            &store,
            "SELECT ?p ?o WHERE { <http://example.org/alice> ?p ?o }",
        )
        .unwrap();

        assert_eq!(result.variables, vec!["p", "o"]);
        assert_eq!(result.rows.len(), 4); // type, name, age, knows
    }

    #[test]
    fn select_with_filter_comparison() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?s ?age WHERE {
                ?s <http://example.org/age> ?age .
                FILTER(?age > 28)
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 2); // Alice (30) and Carol (35)
        for row in &result.rows {
            let age = row.get("age").unwrap();
            match age {
                Value::Int(n) => assert!(*n > 28),
                _ => panic!("expected Int"),
            }
        }
    }

    #[test]
    fn select_with_join() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?name ?friend_name WHERE {
                ?s <http://example.org/name> ?name .
                ?s <http://example.org/knows> ?friend .
                ?friend <http://example.org/name> ?friend_name .
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 2); // Alice→Bob and Bob→Alice
        let pairs: Vec<(&Value, &Value)> = result
            .rows
            .iter()
            .map(|r| {
                (
                    r.get("name").unwrap(),
                    r.get("friend_name").unwrap(),
                )
            })
            .collect();
        assert!(pairs.contains(&(
            &Value::Str("Alice".into()),
            &Value::Str("Bob".into())
        )));
        assert!(pairs.contains(&(
            &Value::Str("Bob".into()),
            &Value::Str("Alice".into())
        )));
    }

    #[test]
    fn select_with_filter_equality() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?s WHERE {
                ?s <http://example.org/name> ?name .
                FILTER(?name = "Alice")
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn select_distinct() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT DISTINCT ?type WHERE {
                ?s a ?type .
            }"#,
        )
        .unwrap();

        // Person appears twice but DISTINCT deduplicates.
        assert_eq!(result.rows.len(), 2); // Person, Employee
    }

    #[test]
    fn select_limit_offset() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?name WHERE {
                ?s <http://example.org/name> ?name .
            } LIMIT 2"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn select_with_filter_bound() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?s ?name WHERE {
                ?s <http://example.org/name> ?name .
                FILTER(BOUND(?name))
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 3);
    }

    #[test]
    fn select_order_by_asc() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?name ?age WHERE {
                ?s <http://example.org/name> ?name .
                ?s <http://example.org/age> ?age .
            } ORDER BY ?age"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 3);
        let ages: Vec<&Value> = result.rows.iter().map(|r| r.get("age").unwrap()).collect();
        assert_eq!(ages, vec![&Value::Int(25), &Value::Int(30), &Value::Int(35)]);
    }

    #[test]
    fn select_order_by_desc() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?name ?age WHERE {
                ?s <http://example.org/name> ?name .
                ?s <http://example.org/age> ?age .
            } ORDER BY DESC(?age)"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 3);
        let ages: Vec<&Value> = result.rows.iter().map(|r| r.get("age").unwrap()).collect();
        assert_eq!(ages, vec![&Value::Int(35), &Value::Int(30), &Value::Int(25)]);
    }

    #[test]
    fn select_optional() {
        let store = test_store_with_data();
        // All people have names; only alice and bob have "knows" relationships.
        // Carol doesn't know anyone, but should still appear with unbound ?friend.
        let result = query(
            &store,
            r#"SELECT ?name ?friend WHERE {
                ?s <http://example.org/name> ?name .
                OPTIONAL { ?s <http://example.org/knows> ?friend }
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 3);
        // Carol's row should have name but no friend binding.
        let carol_row = result
            .rows
            .iter()
            .find(|r| r.get("name") == Some(&Value::Str("Carol".into())))
            .expect("Carol should appear");
        assert!(!carol_row.contains_key("friend"), "Carol should have no friend binding");

        // Alice and Bob should have friend bindings.
        let alice_row = result
            .rows
            .iter()
            .find(|r| r.get("name") == Some(&Value::Str("Alice".into())))
            .expect("Alice should appear");
        assert!(alice_row.contains_key("friend"), "Alice should have a friend binding");
    }

    #[test]
    fn select_order_by_with_limit() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?name ?age WHERE {
                ?s <http://example.org/name> ?name .
                ?s <http://example.org/age> ?age .
            } ORDER BY ?age LIMIT 2"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 2);
        let ages: Vec<&Value> = result.rows.iter().map(|r| r.get("age").unwrap()).collect();
        assert_eq!(ages, vec![&Value::Int(25), &Value::Int(30)]);
    }

    #[test]
    fn rdfs_subclass_type_query() {
        // Set up a class hierarchy: Employee rdfs:subClassOf Person
        let mut store = Store::open_in_memory().unwrap();
        let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:Employee rdfs:subClassOf ex:Person .
ex:Manager rdfs:subClassOf ex:Employee .

ex:alice a ex:Person ; ex:name "Alice" .
ex:bob a ex:Employee ; ex:name "Bob" .
ex:carol a ex:Manager ; ex:name "Carol" .
ex:dave a ex:Other ; ex:name "Dave" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        // Query for all Person instances — should include Person, Employee, and Manager.
        let result = query(
            &store,
            "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
        )
        .unwrap();

        assert_eq!(result.rows.len(), 3, "alice + bob + carol are all Persons");

        // Query for Employee — should include Employee and Manager.
        let result = query(
            &store,
            "SELECT ?s WHERE { ?s a <http://example.org/Employee> }",
        )
        .unwrap();

        assert_eq!(result.rows.len(), 2, "bob + carol are Employees");

        // Query for Manager — only carol.
        let result = query(
            &store,
            "SELECT ?s WHERE { ?s a <http://example.org/Manager> }",
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1, "only carol is a Manager");

        // Query for Other — only dave (no subclass hierarchy).
        let result = query(
            &store,
            "SELECT ?s WHERE { ?s a <http://example.org/Other> }",
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1, "only dave is Other");
    }

    #[test]
    fn rdfs_subclass_no_hierarchy() {
        // Without any rdfs:subClassOf triples, queries work normally.
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?s WHERE { ?s a <http://example.org/Person> }"#,
        )
        .unwrap();

        // Alice and Bob are Person, Carol is Employee.
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn select_count_all() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT (COUNT(*) AS ?count) WHERE { ?s <http://example.org/name> ?name }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].get("count"), Some(&Value::Int(3)));
    }

    #[test]
    fn select_group_by_with_count() {
        let store = test_store_with_data();
        let result = query(
            &store,
            r#"SELECT ?type (COUNT(?s) AS ?n) WHERE { ?s a ?type } GROUP BY ?type"#,
        )
        .unwrap();

        // Two types: Person (2 instances), Employee (1 instance)
        assert_eq!(result.rows.len(), 2);

        for row in &result.rows {
            let count = row.get("n").unwrap();
            match count {
                Value::Int(1) | Value::Int(2) => {} // valid counts
                other => panic!("unexpected count: {other:?}"),
            }
        }
    }

    #[test]
    fn select_sum_and_avg() {
        let store = test_store_with_data();

        let result = query(
            &store,
            r#"SELECT (SUM(?age) AS ?total) (AVG(?age) AS ?mean) WHERE {
                ?s <http://example.org/age> ?age
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1);
        // Ages: 30 + 25 + 35 = 90
        assert_eq!(result.rows[0].get("total"), Some(&Value::Int(90)));
        // Avg: 90 / 3 = 30.0
        assert_eq!(result.rows[0].get("mean"), Some(&Value::Float(30.0)));
    }

    #[test]
    fn select_min_max() {
        let store = test_store_with_data();

        let result = query(
            &store,
            r#"SELECT (MIN(?age) AS ?youngest) (MAX(?age) AS ?oldest) WHERE {
                ?s <http://example.org/age> ?age
            }"#,
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].get("youngest"), Some(&Value::Int(25)));
        assert_eq!(result.rows[0].get("oldest"), Some(&Value::Int(35)));
    }
}
