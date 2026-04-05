//! Property path evaluation — SPARQL 1.1 property paths over the EAVT fact log.

use std::collections::{HashSet, VecDeque};

use spargebra::algebra::PropertyPathExpression;
use spargebra::term::TermPattern;

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::pattern_util::bind_var;
use super::{Bindings, TemporalContext};

/// Evaluate a property path pattern, returning bindings for subject/object variables.
pub fn eval_path_pattern(
    store: &Store,
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    let subj_id = resolve_term_to_id(store, subject, bindings)?;
    let obj_id = resolve_term_to_id(store, object, bindings)?;
    let pairs = eval_path_expr(store, path, subj_id, obj_id, ctx)?;

    let mut results = Vec::new();
    for (s_id, o_id) in pairs {
        let mut new_bindings = bindings.clone();
        let mut compatible = true;
        bind_term(
            &mut new_bindings,
            subject,
            id_to_value(store, s_id)?,
            &mut compatible,
        );
        if !compatible {
            continue;
        }
        bind_term(
            &mut new_bindings,
            object,
            id_to_value(store, o_id)?,
            &mut compatible,
        );
        if compatible {
            results.push(new_bindings);
        }
    }
    Ok(results)
}

/// Bind a `TermPattern` (Variable or `BlankNode`) in the bindings map.
fn bind_term(bindings: &mut Bindings, term: &TermPattern, val: Value, compat: &mut bool) {
    match term {
        TermPattern::Variable(v) => {
            bind_var(bindings, v.as_str(), val, compat);
        }
        TermPattern::BlankNode(b) => {
            bind_var(bindings, b.as_str(), val, compat);
        }
        _ => {}
    }
}

/// Evaluate a `PropertyPathExpression` to produce (subject, object) ID pairs.
fn eval_path_expr(
    store: &Store,
    path: &PropertyPathExpression,
    fixed_subj: Option<i64>,
    fixed_obj: Option<i64>,
    ctx: &TemporalContext,
) -> Result<Vec<(i64, i64)>> {
    match path {
        PropertyPathExpression::NamedNode(pred) => {
            eval_single_edge(store, pred.as_str(), fixed_subj, fixed_obj, ctx)
        }
        PropertyPathExpression::Reverse(inner) => {
            let pairs = eval_path_expr(store, inner, fixed_obj, fixed_subj, ctx)?;
            Ok(pairs.into_iter().map(|(s, o)| (o, s)).collect())
        }
        PropertyPathExpression::Sequence(left, right) => {
            eval_sequence(store, left, right, fixed_subj, fixed_obj, ctx)
        }
        PropertyPathExpression::Alternative(left, right) => {
            let mut pairs = eval_path_expr(store, left, fixed_subj, fixed_obj, ctx)?;
            for p in eval_path_expr(store, right, fixed_subj, fixed_obj, ctx)? {
                if !pairs.contains(&p) {
                    pairs.push(p);
                }
            }
            Ok(pairs)
        }
        PropertyPathExpression::ZeroOrMore(inner) => {
            eval_transitive(store, inner, fixed_subj, fixed_obj, true, ctx)
        }
        PropertyPathExpression::OneOrMore(inner) => {
            eval_transitive(store, inner, fixed_subj, fixed_obj, false, ctx)
        }
        PropertyPathExpression::ZeroOrOne(inner) => {
            let mut pairs = eval_path_expr(store, inner, fixed_subj, fixed_obj, ctx)?;
            add_identity_pairs(store, &mut pairs, fixed_subj, fixed_obj, ctx)?;
            Ok(pairs)
        }
        PropertyPathExpression::NegatedPropertySet(excluded) => {
            eval_negated_set(store, excluded, fixed_subj, fixed_obj, ctx)
        }
    }
}

/// Query facts for a single predicate edge, returning (subject, object) pairs.
fn eval_single_edge(
    store: &Store,
    pred_iri: &str,
    fixed_subj: Option<i64>,
    fixed_obj: Option<i64>,
    ctx: &TemporalContext,
) -> Result<Vec<(i64, i64)>> {
    let Some(pred_id) = store.lookup(pred_iri)? else {
        return Ok(vec![]);
    };
    let mut conds = vec!["a = ?1".to_string(), "op = 1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pred_id)];
    if let Some(s) = fixed_subj {
        conds.push(format!("e = ?{}", params.len() + 1));
        params.push(Box::new(s));
    }
    if let Some(o) = fixed_obj {
        conds.push(format!("v = ?{}", params.len() + 1));
        params.push(Box::new(Value::Ref(o).to_bytes()));
    }
    add_temporal_conditions(&mut conds, &mut params, ctx);
    query_pairs(store, &conds, &params)
}

/// Evaluate a sequence path (left / right).
fn eval_sequence(
    store: &Store,
    left: &PropertyPathExpression,
    right: &PropertyPathExpression,
    fixed_subj: Option<i64>,
    fixed_obj: Option<i64>,
    ctx: &TemporalContext,
) -> Result<Vec<(i64, i64)>> {
    let left_pairs = eval_path_expr(store, left, fixed_subj, None, ctx)?;
    let mut results = Vec::new();
    for &(s, mid) in &left_pairs {
        for (_, o) in eval_path_expr(store, right, Some(mid), fixed_obj, ctx)? {
            let pair = (s, o);
            if !results.contains(&pair) {
                results.push(pair);
            }
        }
    }
    Ok(results)
}

/// Evaluate transitive closure via BFS.
fn eval_transitive(
    store: &Store,
    inner: &PropertyPathExpression,
    fixed_subj: Option<i64>,
    fixed_obj: Option<i64>,
    include_zero: bool,
    ctx: &TemporalContext,
) -> Result<Vec<(i64, i64)>> {
    if let Some(s) = fixed_subj {
        let reachable = bfs_forward(store, inner, s, ctx)?;
        let mut pairs: Vec<(i64, i64)> = reachable
            .into_iter()
            .filter(|&o| fixed_obj.is_none_or(|fo| fo == o))
            .map(|o| (s, o))
            .collect();
        if include_zero && fixed_obj.is_none_or(|fo| fo == s) && !pairs.contains(&(s, s)) {
            pairs.insert(0, (s, s));
        }
        Ok(pairs)
    } else if let Some(o) = fixed_obj {
        let reversed = PropertyPathExpression::Reverse(Box::new(inner.clone()));
        let reachable = bfs_forward(store, &reversed, o, ctx)?;
        let mut pairs: Vec<(i64, i64)> = reachable.into_iter().map(|s| (s, o)).collect();
        if include_zero && !pairs.contains(&(o, o)) {
            pairs.insert(0, (o, o));
        }
        Ok(pairs)
    } else {
        // Neither endpoint fixed: seed from one-step results, BFS from each.
        let one_step = eval_path_expr(store, inner, None, None, ctx)?;
        let mut seeds: Vec<i64> = Vec::new();
        for &(s, _) in &one_step {
            if !seeds.contains(&s) {
                seeds.push(s);
            }
        }
        let mut results = Vec::new();
        for seed in &seeds {
            for o in bfs_forward(store, inner, *seed, ctx)? {
                push_unique(&mut results, (*seed, o));
            }
            if include_zero {
                push_unique(&mut results, (*seed, *seed));
            }
        }
        if include_zero {
            for &(_, o) in &one_step {
                push_unique(&mut results, (o, o));
            }
        }
        Ok(results)
    }
}

/// BFS forward: follow `path` one step at a time from `start`, returning reachable IDs.
fn bfs_forward(
    store: &Store,
    path: &PropertyPathExpression,
    start: i64,
    ctx: &TemporalContext,
) -> Result<Vec<i64>> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(start);
    let mut reachable = Vec::new();
    while let Some(current) = queue.pop_front() {
        for (_, next) in eval_path_expr(store, path, Some(current), None, ctx)? {
            if visited.insert(next) {
                reachable.push(next);
                queue.push_back(next);
            }
        }
    }
    Ok(reachable)
}

/// Add identity pairs for `ZeroOrOne`.
fn add_identity_pairs(
    store: &Store,
    pairs: &mut Vec<(i64, i64)>,
    fixed_subj: Option<i64>,
    fixed_obj: Option<i64>,
    ctx: &TemporalContext,
) -> Result<()> {
    match (fixed_subj, fixed_obj) {
        (Some(s), Some(o)) if s == o => push_unique(pairs, (s, s)),
        (Some(_), Some(_)) => {} // s != o: identity won't match
        (Some(s), None) => push_unique(pairs, (s, s)),
        (None, Some(o)) => push_unique(pairs, (o, o)),
        (None, None) => {
            let mut nodes: HashSet<i64> = pairs.iter().flat_map(|&(s, o)| [s, o]).collect();
            for id in all_entity_ids(store, ctx)? {
                nodes.insert(id);
            }
            for n in nodes {
                push_unique(pairs, (n, n));
            }
        }
    }
    Ok(())
}

/// Match any edge whose predicate is NOT in the excluded set.
fn eval_negated_set(
    store: &Store,
    excluded: &[oxrdf::NamedNode],
    fixed_subj: Option<i64>,
    fixed_obj: Option<i64>,
    ctx: &TemporalContext,
) -> Result<Vec<(i64, i64)>> {
    let excluded_ids: Vec<i64> = excluded
        .iter()
        .filter_map(|n| store.lookup(n.as_str()).ok().flatten())
        .collect();
    let mut conds = vec!["op = 1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(s) = fixed_subj {
        conds.push(format!("e = ?{}", params.len() + 1));
        params.push(Box::new(s));
    }
    if let Some(o) = fixed_obj {
        conds.push(format!("v = ?{}", params.len() + 1));
        params.push(Box::new(Value::Ref(o).to_bytes()));
    }
    add_temporal_conditions(&mut conds, &mut params, ctx);
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
    query_pairs(store, &conds, &params)
}

// ── Helpers ──────────────────────────────────────────────────────

/// Execute a fact query and return deduplicated (entity, ref-object) pairs.
fn query_pairs(
    store: &Store,
    conds: &[String],
    params: &[Box<dyn rusqlite::types::ToSql>],
) -> Result<Vec<(i64, i64)>> {
    let sql = format!("SELECT e, v FROM facts WHERE {}", conds.join(" AND "));
    let mut stmt = store.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(std::convert::AsRef::as_ref).collect();
    let mut rows = stmt.query(refs.as_slice())?;
    let mut results = Vec::new();
    while let Some(row) = rows.next()? {
        let e_id: i64 = row.get(0)?;
        let v = Value::from_bytes(&row.get::<_, Vec<u8>>(1)?)?;
        if let Value::Ref(o_id) = v {
            push_unique(&mut results, (e_id, o_id));
        }
    }
    Ok(results)
}

fn all_entity_ids(store: &Store, ctx: &TemporalContext) -> Result<Vec<i64>> {
    let mut conds = vec!["op = 1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    add_temporal_conditions(&mut conds, &mut params, ctx);
    let sql = format!("SELECT DISTINCT e FROM facts WHERE {}", conds.join(" AND "));
    let mut stmt = store.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(std::convert::AsRef::as_ref).collect();
    let mut rows = stmt.query(refs.as_slice())?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next()? {
        ids.push(row.get(0)?);
    }
    Ok(ids)
}

fn add_temporal_conditions(
    conds: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ctx: &TemporalContext,
) {
    if let Some(vt) = &ctx.valid_at {
        conds.push(format!("valid_from <= ?{}", params.len() + 1));
        params.push(Box::new(vt.clone()));
        conds.push(format!(
            "(valid_to IS NULL OR valid_to > ?{})",
            params.len()
        ));
    } else {
        conds.push("valid_to IS NULL".to_string());
    }
    if let Some(tx) = ctx.as_of_tx {
        conds.push(format!("tx <= ?{}", params.len() + 1));
        params.push(Box::new(tx));
    }
}

fn resolve_term_to_id(
    store: &Store,
    term: &TermPattern,
    bindings: &Bindings,
) -> Result<Option<i64>> {
    match term {
        TermPattern::NamedNode(n) => Ok(store.lookup(n.as_str())?),
        TermPattern::BlankNode(b) => match bindings.get(b.as_str()) {
            Some(Value::Ref(id)) => Ok(Some(*id)),
            _ => Ok(None),
        },
        TermPattern::Variable(v) => match bindings.get(v.as_str()) {
            Some(Value::Ref(id)) => Ok(Some(*id)),
            _ => Ok(None),
        },
        TermPattern::Literal(_) => Ok(None),
        #[cfg(feature = "shacl")]
        TermPattern::Triple(_) => Ok(None),
    }
}

fn id_to_value(store: &Store, id: i64) -> Result<Value> {
    let iri = store.resolve(id)?;
    if iri.starts_with("_:") {
        Ok(Value::Str(iri))
    } else if let Some(term_id) = store.lookup(&iri)? {
        Ok(Value::Ref(term_id))
    } else {
        Ok(Value::Str(iri))
    }
}

fn push_unique(vec: &mut Vec<(i64, i64)>, pair: (i64, i64)) {
    if !vec.contains(&pair) {
        vec.push(pair);
    }
}

/// Get variable names from subject/object `TermPattern`s in a path pattern.
pub fn path_pattern_vars(subject: &TermPattern, object: &TermPattern) -> Vec<String> {
    let mut vars = Vec::new();
    match subject {
        TermPattern::Variable(v) => vars.push(v.as_str().to_string()),
        TermPattern::BlankNode(b) => vars.push(b.as_str().to_string()),
        _ => {}
    }
    match object {
        TermPattern::Variable(v) => vars.push(v.as_str().to_string()),
        TermPattern::BlankNode(b) => vars.push(b.as_str().to_string()),
        _ => {}
    }
    vars
}
