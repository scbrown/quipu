//! BGP evaluation and single-triple-pattern matching against the fact store.
//!
//! This is the leaf of the pattern evaluator: everything in `pattern.rs` is
//! SPARQL algebra over rows, while everything here turns a triple pattern into
//! `SQL` over `facts` and binds the variables in the rows that come back.

use std::collections::HashSet;

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::pattern_util::{
    bind_var, resolve_object_pattern, resolve_predicate_pattern, resolve_subject_pattern,
    triple_pattern_vars,
};
use super::{Bindings, GraphScope, TemporalContext};

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
fn sql_graph_in(gids: &[i64], sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>) -> String {
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

/// Build an equality/IN predicate for a term-id column. The one-id spelling is
/// intentionally the pre-#76 plan; only composed stores widen it.
fn sql_id_in(
    column: &str,
    ids: &[i64],
    sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> String {
    if ids.is_empty() {
        return "0 = 1".to_string();
    }
    let placeholders: Vec<String> = ids
        .iter()
        .map(|id| {
            sql_params.push(Box::new(*id));
            format!("?{}", sql_params.len())
        })
        .collect();
    if placeholders.len() == 1 {
        format!("{column} = {}", placeholders[0])
    } else {
        format!("{column} IN ({})", placeholders.join(", "))
    }
}

fn sql_ref_in(ids: &[i64], sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>) -> String {
    if ids.is_empty() {
        return "0 = 1".to_string();
    }
    let placeholders: Vec<String> = ids
        .iter()
        .map(|id| {
            sql_params.push(Box::new(Value::Ref(*id).to_bytes()));
            format!("?{}", sql_params.len())
        })
        .collect();
    if placeholders.len() == 1 {
        format!("v = {}", placeholders[0])
    } else {
        format!("v IN ({})", placeholders.join(", "))
    }
}

/// Evaluate a single triple pattern against the store, extending existing bindings.
pub fn eval_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    // The read model answers this exact question when the guard admits, with
    // no SQL and no per-row dictionary round-trips. Everything it cannot answer
    // — time travel, non-ROOT graphs, attachments — falls through to the SQL
    // below, unchanged.
    if crate::store::read_model::read_model_applicable(store, ctx) {
        return eval_triple_pattern_from_model(store, tp, bindings);
    }

    // Build SQL query with conditions based on bound values.
    let mut conditions = Vec::new();
    let mut sql_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    // Subject
    if let Some(iri) = resolve_subject_pattern(&tp.subject, bindings) {
        let ids = store.lookup_all(&iri)?;
        if ids.is_empty() {
            return Ok(vec![]); // IRI not in dictionary -> no matches
        }
        conditions.push(sql_id_in("e", &ids, &mut sql_params));
    }

    // Predicate
    if let Some(iri) = resolve_predicate_pattern(&tp.predicate, bindings) {
        let ids = store.lookup_all(&iri)?;
        if ids.is_empty() {
            return Ok(vec![]);
        }
        conditions.push(sql_id_in("a", &ids, &mut sql_params));
    }

    // Object (only if it's a concrete value, not a variable)
    if let Some(value) = resolve_object_pattern(store, &tp.object, bindings)? {
        if let Value::Ref(id) = value
            && id != -1
        {
            let iri = store.resolve(id)?;
            let ids = store.lookup_all(&iri)?;
            conditions.push(sql_ref_in(&ids, &mut sql_params));
        } else {
            let bytes = value.to_bytes();
            conditions.push(format!("v = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(bytes));
        }
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
        GraphScope::Named(gids) => {
            conditions.push(sql_graph_in(gids, &mut sql_params));
            None
        }
        GraphScope::AnyNamed { var, restrict } => {
            match restrict {
                // quipu #70: an UNRESTRICTED `GRAPH ?g` excludes the reserved
                // label meta-graph as well as ROOT.
                //
                // The meta-graph holds labels *about* graphs. Letting `?g` range
                // over it means `GRAPH ?g { ?s ?p ?o }` — the natural "give me
                // every named graph's triples" — starts returning freshness and
                // trust facts as if they were data, and a consumer's result set
                // silently changes the first time anyone labels anything.
                //
                // It stays reachable by EXPLICIT name, which is what §6's
                // precedence query uses (`GRAPH <urn:quipu:graph:meta> { … }`).
                // Naming it is deliberate; ranging over it is not. A `FROM NAMED`
                // restriction naming it explicitly is likewise honoured below.
                //
                // Not a regression: the meta-graph is new in #65, so no existing
                // query could have been reading it.
                None => {
                    conditions.push(format!("g <> 0 AND g <> ?{}", sql_params.len() + 1));
                    let meta_g = store
                        .lookup(crate::namespace::META_GRAPH_IRI)?
                        .unwrap_or(-1);
                    sql_params.push(Box::new(meta_g));
                }
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
    } else if let Some(tx) = ctx.as_of_tx {
        // quipu #83: as-of-TRANSACTION liveness, not present-tense liveness.
        //
        // This used to push `valid_to IS NULL` and then merely ADD `tx <= N`,
        // so a fact live at N but retracted since was invisible at every N —
        // silently, as a smaller answer rather than an error. The row is live
        // at N when it was asserted by then AND was not retracted by then.
        //
        // A legacy row closed before the #83 migration has `retracted_tx` NULL,
        // so `retracted_tx > N` is NULL and the row stays invisible exactly as
        // it is today. That is deliberate: the tx that closed it was never
        // recorded, and guessing would place it in windows it may not have been
        // live in.
        conditions.push(format!(
            "(valid_to IS NULL OR retracted_tx > ?{})",
            sql_params.len() + 1
        ));
        sql_params.push(Box::new(tx));
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
    // quipu #75: `facts_source()` is the literal `facts` for a store with no
    // attachments, so this is byte-identical to the SQL above for every store
    // that did not ask to compose. With attachments it is a `UNION ALL` over
    // main and each layer's CONTRIBUTED graphs; the conditions above stay
    // outside it and SQLite pushes them into each branch — measured, and
    // asserted by `graph_predicate_is_pushed_into_each_union_branch`.
    //
    // This is the ONLY query-path site that composes. The other readers of
    // `facts` are either the write path, local bookkeeping, or deliberately
    // ROOT-scoped — and an attachment contributes only NAMED graphs, so a
    // ROOT-scoped read could not see one even if it composed.
    let facts = store.facts_source();
    let sql = if want_g {
        format!("SELECT DISTINCT e, a, v, g FROM {facts}{where_clause}")
    } else {
        format!("SELECT DISTINCT e, a, v FROM {facts}{where_clause}")
    };
    let mut stmt = store.prepare(&sql)?;

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        sql_params.iter().map(std::convert::AsRef::as_ref).collect();
    let mut rows = stmt.query(param_refs.as_slice())?;

    let mut results = Vec::new();
    // SQL DISTINCT sees raw ids, so aliases survive it. Dedup the canonical
    // triple key in O(n): comparing each completed binding against the whole
    // result vector made an unbound production scan quadratic (aegis-h7rtt).
    let mut canonical_rows = HashSet::new();
    while let Some(row) = rows.next()? {
        let e_id = store.canonical_id(row.get(0)?)?;
        let a_id = store.canonical_id(row.get(1)?)?;
        let v_bytes: Vec<u8> = row.get(2)?;
        let v = match Value::from_bytes(&v_bytes)? {
            Value::Ref(id) => Value::Ref(store.canonical_id(id)?),
            other => other,
        };
        let g_id: Option<i64> = if want_g {
            Some(store.canonical_id(row.get(3)?)?)
        } else {
            None
        };
        let canonical_key = (e_id, a_id, v.to_bytes(), g_id);
        if !canonical_rows.insert(canonical_key) {
            continue;
        }
        let matched = MatchedRow {
            e_id,
            a_id,
            v,
            g_id,
        };
        if let Some(row) = bind_row(store, tp, bindings, matched, bind_graph_var.as_deref())? {
            results.push(row);
        }
    }

    Ok(results)
}

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
struct MatchedRow {
    e_id: i64,
    a_id: i64,
    v: Value,
    g_id: Option<i64>,
}

fn bind_row(
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
            let e_iri = store.resolve(e_id)?;
            let e_val = if e_iri.starts_with("_:") {
                Value::Str(e_iri)
            } else if let Some(term_id) = store.lookup(&e_iri)? {
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

    // Bind predicate variable.
    if let NamedNodePattern::Variable(var) = &tp.predicate {
        let a_iri = store.resolve(a_id)?;
        let a_val = if let Some(term_id) = store.lookup(&a_iri)? {
            Value::Ref(term_id)
        } else {
            Value::Str(a_iri)
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

/// Evaluate one triple pattern against the resident read model instead of SQL.
///
/// Only ever reached when `read_model_applicable` admits (see
/// `src/store/read_model.rs`), which is what makes the shortcuts here sound:
/// the graph is plain ROOT so no graph variable binds and `g_id` is always
/// `None`, and there are no attachments so `canonical_id` is the identity and
/// `lookup_all` degenerates to `lookup`.
///
/// Rows go through the same [`bind_row`] as the SQL path, so the two cannot
/// disagree about what a triple binds to.
///
/// Candidates are sorted by `(e, a, v)`. The SQL this replaces carries no
/// `ORDER BY`, so its order was incidental — index order, usually `idx_eavt`.
/// Sorting makes the fast path deterministic rather than merely different.
fn eval_triple_pattern_from_model(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
) -> Result<Vec<Bindings>> {
    let subject = match resolve_subject_pattern(&tp.subject, bindings) {
        Some(iri) => match store.lookup(&iri)? {
            Some(id) => Some(id),
            None => return Ok(vec![]), // not in the dictionary -> no matches
        },
        None => None,
    };
    let predicate = match resolve_predicate_pattern(&tp.predicate, bindings) {
        Some(iri) => match store.lookup(&iri)? {
            Some(id) => Some(id),
            None => return Ok(vec![]),
        },
        None => None,
    };
    let object = resolve_object_pattern(store, &tp.object, bindings)?;

    let model = store.read_model()?;
    let mut candidates: Vec<(i64, i64, Value)> = match (subject, predicate, &object) {
        (Some(e), Some(a), Some(v)) => {
            if model.contains(e, a, v) {
                vec![(e, a, v.clone())]
            } else {
                vec![]
            }
        }
        (Some(e), Some(a), None) => model
            .by_subject(e)
            .iter()
            .filter(|(pa, _)| *pa == a)
            .map(|(pa, v)| (e, *pa, v.clone()))
            .collect(),
        (Some(e), None, Some(v)) => model
            .by_subject(e)
            .iter()
            .filter(|(_, pv)| *pv == *v)
            .map(|(pa, pv)| (e, *pa, pv.clone()))
            .collect(),
        (Some(e), None, None) => model
            .by_subject(e)
            .iter()
            .map(|(pa, pv)| (e, *pa, pv.clone()))
            .collect(),
        (None, Some(a), Some(v)) => model
            .by_predicate_object(a, v)
            .iter()
            .map(|e| (*e, a, v.clone()))
            .collect(),
        (None, Some(a), None) => model
            .by_predicate(a)
            .iter()
            .map(|(e, pv)| (*e, a, pv.clone()))
            .collect(),
        (None, None, Some(v)) => model
            .by_object(v)
            .iter()
            .map(|(e, a)| (*e, *a, v.clone()))
            .collect(),
        (None, None, None) => model
            .iter_triples()
            .map(|(e, a, v)| (e, a, v.clone()))
            .collect(),
    };
    candidates.sort_unstable_by(|l, r| (l.0, l.1, l.2.to_bytes()).cmp(&(r.0, r.1, r.2.to_bytes())));

    let mut results = Vec::with_capacity(candidates.len());
    for (e_id, a_id, v) in candidates {
        let matched = MatchedRow {
            e_id,
            a_id,
            v,
            g_id: None,
        };
        if let Some(row) = bind_row(store, tp, bindings, matched, None)? {
            results.push(row);
        }
    }
    Ok(results)
}
