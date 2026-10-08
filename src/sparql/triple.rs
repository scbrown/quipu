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
    resolve_object_pattern, resolve_predicate_pattern, resolve_subject_pattern, triple_pattern_vars,
};
use super::rdfs::{
    collect_class_and_subclasses, eval_type_pattern_with_subclasses, is_rdf_type_pattern,
};
use super::triple_bind::{MatchedRow, bind_row};
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

    // The read model turns a pattern into hash lookups, which makes evaluating
    // it ONCE and joining cheap — so when it is available, take the hash join
    // and leave the nested loop below for SQL. The loop is where the O(n^2)
    // lives: re-evaluating each pattern per accumulated row is quadratic no
    // matter how fast an individual evaluation gets.
    // Only for a BGP that actually JOINS. A single pattern is the shape SQL is
    // already fast at — a bound-subject lookup is ~0.1ms against an index — and
    // routing it through the model would make it pay for building one, which
    // measured as a 0.12ms -> 320ms regression. Two or more patterns is exactly
    // where the nested loop turns quadratic and the build pays for itself.
    // The read model currently indexes asserted triples. Keep constant
    // rdf:type patterns on the entailment-aware evaluator so a join cannot
    // silently change the answer back to asserted-only.
    let needs_type_entailment = patterns.iter().any(is_rdf_type_pattern);
    if patterns.len() >= 2
        && !needs_type_entailment
        && crate::store::read_model::read_model_applicable(store, ctx)
    {
        return super::join::eval_bgp_hash_join(store, patterns, ctx, seed);
    }

    let mut result_rows: Vec<Bindings> = vec![seed.clone()];
    let mut all_vars = Vec::new();

    for (pi, tp) in patterns.iter().enumerate() {
        // LIMIT pushdown (quipu-0lr): only the LAST pattern may stop early —
        // an earlier pattern's rows are join input, and any of them might
        // survive the remaining patterns. For the last one, every bound row
        // IS a solution, so `row_limit` of them is enough; rusqlite steps the
        // statement lazily, so breaking out genuinely stops the scan instead
        // of discarding a completed one.
        let last = pi + 1 == patterns.len();
        let mut new_rows = Vec::new();
        for (i, existing) in result_rows.iter().enumerate() {
            // BGP accumulation multiplies row counts pattern-by-pattern —
            // the SQLite handler interrupts a grinding statement, but the
            // row-count explosion itself is only visible here.
            super::pattern_util::check_eval_budget(ctx, i, new_rows.len())?;
            let remaining = match (last, ctx.row_limit) {
                (true, Some(cap)) => {
                    if new_rows.len() >= cap {
                        break;
                    }
                    Some(cap - new_rows.len())
                }
                _ => None,
            };
            let matches = eval_triple_pattern_limited(store, tp, existing, ctx, remaining)?;
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

// The SQL `IN`-clause builders live in `sql_in` (size ratchet split).
use super::sql_in::{sql_graph_in, sql_id_in, sql_ref_in};

/// Evaluate a single triple pattern against the store, extending existing bindings.
pub fn eval_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    eval_triple_pattern_limited(store, tp, bindings, ctx, None)
}

/// [`eval_triple_pattern`] that stops after `limit` BOUND rows (quipu-0lr).
///
/// The cap counts rows that actually bound — not SQL rows — so a row the
/// binding step rejects (e.g. a repeated variable the SQL cannot express)
/// never causes an undershoot. Enforced in the row loop rather than as a SQL
/// `LIMIT` because rusqlite steps lazily: breaking out stops the scan just as
/// surely, and stays correct for the reject-a-row case.
fn eval_triple_pattern_limited(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
    limit: Option<usize>,
) -> Result<Vec<Bindings>> {
    let mut results = Vec::new();
    visit_triple_pattern_limited(
        store,
        tp,
        bindings,
        ctx,
        limit,
        false,
        Some(&mut |row| {
            results.push(row);
            Ok(())
        }),
    )?;
    Ok(results)
}

/// Visit exact matches without retaining their bindings. SQL and canonical
/// binding rules are shared with the ordinary evaluator.
pub(super) fn visit_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
    emit: &mut dyn FnMut(Bindings) -> Result<()>,
) -> Result<usize> {
    visit_triple_pattern_limited(store, tp, bindings, ctx, None, true, Some(emit))
}

/// Count a leaf without decoding terms. Caller must prove independent variable
/// positions, no attachments, no inference and no seed compatibility checks.
pub(super) fn count_triple_pattern(
    store: &Store,
    tp: &TriplePattern,
    ctx: &TemporalContext,
) -> Result<usize> {
    visit_triple_pattern_limited(store, tp, &Bindings::new(), ctx, None, true, None)
}

fn visit_triple_pattern_limited(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
    limit: Option<usize>,
    streaming: bool,
    mut emit: Option<&mut dyn FnMut(Bindings) -> Result<()>>,
) -> Result<usize> {
    if limit == Some(0) {
        return Ok(0);
    }
    // Formal default: a constant rdf:type query uses the loaded RDFS class
    // hierarchy. Named graphs remain literal because the hierarchy is rooted
    // in the default graph and must not leak across graph boundaries.
    //
    // `includes_root_default` is the entailment-regime case (aegis-g6bu6d).
    // Requesting `entailment: "rdfs"` composes the scope into `[0, companion]`,
    // which is no longer `[0]` — so gating on `is_root_default` alone made the
    // REGIME drop the expansion and return a SUBSET of the unentailed answer
    // while labelling it entailed. The regime must be a SUPERSET of the default,
    // so it keeps the live expansion AND gains the companion graph. Expanding is
    // sound here precisely because graph 0 is still in scope: the hierarchy is
    // read from `g = 0` and applied to an answer that already contains it, not
    // across a boundary. A `FROM` that redefines the default away from ROOT
    // still fails both predicates and stays literal.
    if (ctx.graph.is_root_default() || (ctx.entails_rdfs && ctx.graph.includes_root_default()))
        && is_rdf_type_pattern(tp)
        && let TermPattern::NamedNode(class_node) = &tp.object
    {
        let class_ids = collect_class_and_subclasses(store, class_node.as_str())?;
        if !class_ids.is_empty() {
            let rows =
                eval_type_pattern_with_subclasses(store, tp, bindings, &class_ids, ctx, limit)?;
            let count = rows.len();
            if let Some(emit) = emit.as_mut() {
                for row in rows {
                    emit(row)?;
                }
            }
            return Ok(count);
        }
    }
    // Build SQL query with conditions based on bound values.
    let mut conditions = Vec::new();
    let mut sql_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let direct_refs = super::count_stream::direct_refs() && !store.has_attachments();

    // Subject
    let subject_ref = match &tp.subject {
        TermPattern::Variable(v) => bindings.get(v.as_str()),
        TermPattern::BlankNode(v) => bindings.get(v.as_str()),
        _ => None,
    };
    if direct_refs
        && let Some(Value::Ref(id)) = subject_ref
        && *id >= 0
    {
        conditions.push(format!("e = ?{}", sql_params.len() + 1));
        sql_params.push(Box::new(*id));
    } else if let Some(iri) = resolve_subject_pattern(store, &tp.subject, bindings)? {
        let ids = store.lookup_all(&iri)?;
        if ids.is_empty() {
            return Ok(0); // IRI not in dictionary -> no matches
        }
        conditions.push(sql_id_in("e", &ids, &mut sql_params));
    }

    // Predicate
    // Optional RDF-star features add predicate pattern variants.
    #[allow(clippy::match_wildcard_for_single_variants)]
    let predicate_ref = match &tp.predicate {
        NamedNodePattern::Variable(v) => bindings.get(v.as_str()),
        _ => None,
    };
    if direct_refs
        && let Some(Value::Ref(id)) = predicate_ref
        && *id >= 0
    {
        conditions.push(format!("a = ?{}", sql_params.len() + 1));
        sql_params.push(Box::new(*id));
    } else if let Some(iri) = resolve_predicate_pattern(store, &tp.predicate, bindings)? {
        let ids = store.lookup_all(&iri)?;
        if ids.is_empty() {
            return Ok(0);
        }
        conditions.push(sql_id_in("a", &ids, &mut sql_params));
    }

    // Object (only if it's a concrete value, not a variable)
    if let Some(value) = resolve_object_pattern(store, &tp.object, bindings)? {
        if let Value::Ref(id) = value
            && id != -1
            && (!direct_refs || id < 0)
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

    // aegis-tl2q4j: string FILTERs over this BGP narrow the scan. Only an
    // UNBOUND variable position; a bound one is already an id lookup.
    let narrows_active =
        push_string_narrows(store, tp, bindings, ctx, &mut conditions, &mut sql_params);

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
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        sql_params.iter().map(std::convert::AsRef::as_ref).collect();
    // The scalar functions read the predicates while this statement steps.
    let _narrows = narrows_active.then(|| {
        super::string_pushdown::Active::install(ctx.string_narrows.clone().unwrap_or_default())
    });
    if emit.is_none() {
        return super::count_mmap::scalar(store, || {
            let mut stmt = store.prepare(&format!("SELECT COUNT(*) FROM ({sql})"))?;
            let count: i64 = stmt.query_row(param_refs.as_slice(), |row| row.get(0))?;
            usize::try_from(count).map_err(|_| {
                crate::Error::InvalidValue("COUNT exceeds addressable row count".into())
            })
        });
    }
    let mut stmt = store.prepare(&sql)?;
    let mut rows = stmt.query(param_refs.as_slice())?;

    let mut count = 0;
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
        // Without attached dictionaries SQL DISTINCT already deduplicates the
        // exact canonical triple. Retaining a second set would make a streaming
        // scan grow with the whole graph again.
        if (!streaming || store.has_attachments())
            && !canonical_rows.insert((e_id, a_id, v.to_bytes(), g_id))
        {
            continue;
        }
        let matched = MatchedRow {
            e_id,
            a_id,
            v,
            g_id,
        };
        if let Some(row) = bind_row(store, tp, bindings, matched, bind_graph_var.as_deref())? {
            emit.as_mut().expect("binding visitor was checked above")(row)?;
            count += 1;
            if limit.is_some_and(|cap| count >= cap) {
                break;
            }
        }
    }

    Ok(count)
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
pub(super) fn eval_triple_pattern_from_model(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    graph: i64,
) -> Result<Vec<Bindings>> {
    let subject = match resolve_subject_pattern(store, &tp.subject, bindings)? {
        Some(iri) => match store.lookup(&iri)? {
            Some(id) => Some(id),
            None => return Ok(vec![]), // not in the dictionary -> no matches
        },
        None => None,
    };
    let predicate = match resolve_predicate_pattern(store, &tp.predicate, bindings)? {
        Some(iri) => match store.lookup(&iri)? {
            Some(id) => Some(id),
            None => return Ok(vec![]),
        },
        None => None,
    };
    let object = resolve_object_pattern(store, &tp.object, bindings)?;

    let model = store.read_model_for(graph)?;
    let mut candidates: Vec<(i64, i64, Value)> = match (subject, predicate, &object) {
        (Some(e), Some(a), Some(v)) => {
            if model.contains(store, e, a, v)? {
                vec![(e, a, v.clone())]
            } else {
                vec![]
            }
        }
        (Some(e), Some(a), None) => model
            .by_subject(store, e)?
            .iter()
            .filter(|(pa, _)| *pa == a)
            .map(|(pa, v)| (e, *pa, v.clone()))
            .collect(),
        (Some(e), None, Some(v)) => model
            .by_subject(store, e)?
            .iter()
            .filter(|(_, pv)| *pv == *v)
            .map(|(pa, pv)| (e, *pa, pv.clone()))
            .collect(),
        (Some(e), None, None) => model
            .by_subject(store, e)?
            .iter()
            .map(|(pa, pv)| (e, *pa, pv.clone()))
            .collect(),
        (None, Some(a), Some(v)) => model
            .by_predicate_object(store, a, v)?
            .iter()
            .map(|e| (*e, a, v.clone()))
            .collect(),
        (None, Some(a), None) => model
            .by_predicate(store, a)?
            .iter()
            .map(|(e, pv)| (*e, a, pv.clone()))
            .collect(),
        (None, None, Some(v)) => model
            .by_object(store, v)?
            .iter()
            .map(|(e, a)| (*e, *a, v.clone()))
            .collect(),
        (None, None, None) => model.triples(store)?,
    };
    candidates.sort_unstable_by_key(|l| (l.0, l.1, l.2.to_bytes()));

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

/// Add one SQL condition per string narrowing on an unbound variable of `tp`
/// (aegis-tl2q4j). Returns whether any was added.
fn push_string_narrows(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    ctx: &TemporalContext,
    conditions: &mut Vec<String>,
    sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> bool {
    let Some(narrows) = &ctx.string_narrows else {
        return false;
    };
    let unbound = |name: &str| !bindings.contains_key(name);
    let subject = match &tp.subject {
        TermPattern::Variable(v) if unbound(v.as_str()) => Some(v.as_str()),
        _ => None,
    };
    let predicate = match &tp.predicate {
        NamedNodePattern::Variable(v) if unbound(v.as_str()) => Some(v.as_str()),
        NamedNodePattern::NamedNode(_) | NamedNodePattern::Variable(_) => None,
    };
    let object = match &tp.object {
        TermPattern::Variable(v) if unbound(v.as_str()) => Some(v.as_str()),
        _ => None,
    };
    // A `terms` scan costs ~0.06 s on the production store, once per
    // statement. With the subject or object already bound this statement is a
    // cheap lookup that runs once PER OUTER ROW, so a scan per statement turned
    // a 0.5 s join into a 30 s timeout (measured). There, narrow only by value.
    let scan_terms = subject.is_some() && object.is_some();
    let mut added = false;
    for (k, narrow) in narrows.iter().enumerate() {
        let var = Some(narrow.var.as_str());
        let n = sql_params.len() + 1;
        // Every term space the composed `facts` source can return.
        let mut ids = vec![format!(
            "SELECT id FROM main.terms WHERE quipu_narrow_text(iri, ?{n})"
        )];
        for a in store.attachments() {
            ids.push(format!(
                "SELECT id FROM {}.terms WHERE quipu_narrow_text(iri, ?{n})",
                a.alias
            ));
        }
        let ids = ids.join(" UNION ALL ");
        let mut pushed = false;
        if scan_terms {
            for (position, column) in [(subject, "e"), (predicate, "a")] {
                if position == var {
                    conditions.push(format!("{column} IN ({ids})"));
                    pushed = true;
                }
            }
        }
        if object == var {
            conditions.push(if scan_terms {
                format!(
                    "quipu_narrow_value(v, ?{n}) AND \
                     (quipu_ref_id(v) IS NULL OR quipu_ref_id(v) IN ({ids}))"
                )
            } else {
                format!("quipu_narrow_value(v, ?{n})")
            });
            pushed = true;
        }
        if pushed {
            sql_params.push(Box::new(i64::try_from(k).unwrap_or(i64::MAX)));
            added = true;
        }
    }
    added
}
