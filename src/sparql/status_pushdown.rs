//! Narrow a typed named-graph scan by absence of a plain literal property.
//!
//! A singleton type scan followed by a correlated NOT EXISTS used to decode
//! every typed subject before discovering that most already have the property.
//! The SQL anti-join drops only subjects with a proven plain string value.
//! The original FILTER still evaluates every surviving binding.
//!
//! Root/default graphs, attachments, historical/seeded reads and every other
//! expression keep the exact existing evaluator, including type entailment.

use rusqlite::functions::FunctionFlags;
use spargebra::algebra::{Expression, Function, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use super::pattern_util::Bindings;
use super::{GraphScope, TemporalContext};
use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

struct Plan<'a> {
    subject: &'a str,
    class: &'a str,
    property: &'a str,
}

fn variable(expr: &Expression, name: &str) -> bool {
    matches!(expr, Expression::Variable(v) if v.as_str() == name)
}

fn str_of(expr: &Expression, name: &str) -> bool {
    matches!(expr, Expression::FunctionCall(Function::Str, args)
        if args.len() == 1 && variable(&args[0], name))
}

fn is_literal(expr: &Expression, name: &str) -> bool {
    matches!(expr, Expression::FunctionCall(Function::IsLiteral, args)
        if args.len() == 1 && variable(&args[0], name))
}

fn identity_string(expr: &Expression, name: &str) -> bool {
    matches!(expr, Expression::SameTerm(a, b)
        if (variable(a, name) && str_of(b, name))
        || (variable(b, name) && str_of(a, name)))
}

fn plain_guard(expr: &Expression, name: &str) -> bool {
    matches!(expr, Expression::And(a, b)
        if (is_literal(a, name) && identity_string(b, name))
        || (is_literal(b, name) && identity_string(a, name)))
}

fn literal_not_in(expr: &Expression, name: &str) -> bool {
    let Expression::Not(negated) = expr else {
        return false;
    };
    let Expression::In(value, choices) = negated.as_ref() else {
        return false;
    };
    str_of(value, name)
        && choices
            .iter()
            .all(|choice| matches!(choice, Expression::Literal(_)))
}

fn local_literal_not_in(expr: &Expression, name: &str) -> bool {
    matches!(expr, Expression::And(a, b)
        if (is_literal(a, name) && literal_not_in(b, name))
        || (is_literal(b, name) && literal_not_in(a, name)))
}

fn plan<'a>(expr: &'a Expression, inner: &'a GraphPattern) -> Option<Plan<'a>> {
    let GraphPattern::Bgp { patterns } = inner else {
        return None;
    };
    let [typed] = patterns.as_slice() else {
        return None;
    };
    let (
        TermPattern::Variable(subject),
        NamedNodePattern::NamedNode(predicate),
        TermPattern::NamedNode(class),
    ) = (&typed.subject, &typed.predicate, &typed.object)
    else {
        return None;
    };
    if predicate.as_str() != RDF_TYPE {
        return None;
    }
    let Expression::Not(negated) = expr else {
        return None;
    };
    let Expression::Exists(exists) = negated.as_ref() else {
        return None;
    };
    let GraphPattern::Filter {
        expr: guard,
        inner: property_pattern,
    } = exists.as_ref()
    else {
        return None;
    };
    let GraphPattern::Bgp { patterns } = property_pattern.as_ref() else {
        return None;
    };
    let [property] = patterns.as_slice() else {
        return None;
    };
    let (
        TermPattern::Variable(other_subject),
        NamedNodePattern::NamedNode(property_name),
        TermPattern::Variable(value),
    ) = (&property.subject, &property.predicate, &property.object)
    else {
        return None;
    };
    if other_subject != subject || value == subject || !plain_guard(guard, value.as_str()) {
        return None;
    }
    Some(Plan {
        subject: subject.as_str(),
        class: class.as_str(),
        property: property_name.as_str(),
    })
}

/// Register the packed-value predicate on writer and pooled read connections.
///
/// # Errors
/// Returns the SQLite registration error if the function cannot be installed.
pub fn register(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "quipu_plain_literal",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_INNOCUOUS
            | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let bytes: Vec<u8> = ctx.get(0)?;
            // A malformed value cannot prove the property present: retain its
            // subject for the ordinary decoder/FILTER to decide or fail.
            Ok(matches!(Value::from_bytes(&bytes), Ok(Value::Str(_))))
        },
    )
}

/// Candidates for a proven absence filter, or None for the original evaluator.
///
/// # Errors
/// Propagates dictionary, SQL and query-budget failures without partial output.
pub fn candidates(
    store: &Store,
    expr: &Expression,
    inner: &GraphPattern,
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<Option<(Vec<Bindings>, Vec<String>)>> {
    if !seed.is_empty()
        || !store.attachments().is_empty()
        || ctx.valid_at.is_some()
        || ctx.as_of_tx.is_some()
        || ctx.row_limit.is_some()
        || ctx.string_narrows.is_some()
    {
        return Ok(None);
    }
    let GraphScope::Named(graphs) = &ctx.graph else {
        return Ok(None);
    };
    let [graph] = graphs.as_slice() else {
        return Ok(None);
    };
    if *graph == 0 {
        return Ok(None);
    }
    let Some(plan) = plan(expr, inner) else {
        return Ok(None);
    };
    let (Some(rdf_type), Some(class), Some(property)) = (
        store.lookup(RDF_TYPE)?,
        store.lookup(plan.class)?,
        store.lookup(plan.property)?,
    ) else {
        return Ok(None);
    };
    // No composition, so ids are canonical and no cross-layer anti-join is
    // needed. Named graph scope was already admitted by the GRAPH evaluator.
    // Intersect existing covering indexes by ROWID of the SAME fact. This
    // keeps graph/currentness checks off the large fact table pages. Readers
    // without the indexes retain the exact ordinary evaluator.
    let indexes: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN
            ('idx_active_vge','idx_current_aev','idx_current_g')",
        [],
        |r| r.get(0),
    )?;
    if indexes != 3 {
        return Ok(None);
    }
    let mut stmt = store.prepare(
        "SELECT DISTINCT t.e FROM facts AS t INDEXED BY idx_active_vge
        WHERE t.v=?2 AND t.g=?3 AND t.op=1 AND t.valid_to IS NULL
        AND EXISTS (SELECT 1 FROM facts AS typed INDEXED BY idx_current_aev
            WHERE typed.a=?1 AND typed.e=t.e AND typed.v=?2
            AND typed.rowid=t.rowid AND typed.op=1 AND typed.valid_to IS NULL)
        AND NOT EXISTS (SELECT 1 FROM facts AS p INDEXED BY idx_current_aev
            WHERE p.e=t.e AND p.a=?4 AND p.op=1 AND p.valid_to IS NULL
            AND quipu_plain_literal(p.v) AND EXISTS
                (SELECT 1 FROM facts AS gp INDEXED BY idx_current_g
                    WHERE gp.g=?3 AND gp.rowid=p.rowid
                    AND gp.op=1 AND gp.valid_to IS NULL))",
    )?;
    let mut rows = stmt.query(rusqlite::params![
        rdf_type,
        Value::Ref(class).to_bytes(),
        graph,
        property
    ])?;
    let mut bindings = Vec::new();
    while let Some(row) = rows.next()? {
        super::pattern_util::check_eval_budget(ctx, bindings.len(), bindings.len())?;
        let subject: i64 = row.get(0)?;
        let mut binding = Bindings::new();
        binding.insert(plan.subject.to_string(), Value::Ref(subject));
        bindings.push(binding);
    }
    Ok(Some((bindings, vec![plan.subject.to_string()])))
}

/// Project subjects of one fixed predicate/object in a current named graph.
///
/// The same fact's ROWID proves predicate membership without reading the wide
/// fact table. Every other context keeps the ordinary triple evaluator.
///
/// # Errors
/// Propagates dictionary, SQL, value decoding and query-budget errors.
pub fn constant_candidates(
    store: &Store,
    patterns: &[TriplePattern],
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<Option<(Vec<Bindings>, Vec<String>)>> {
    if !seed.is_empty()
        || !store.attachments().is_empty()
        || ctx.valid_at.is_some()
        || ctx.as_of_tx.is_some()
        || ctx.row_limit.is_some()
        || ctx.string_narrows.is_some()
        || ctx.entails_rdfs
    {
        return Ok(None);
    }
    let GraphScope::Named(graphs) = &ctx.graph else {
        return Ok(None);
    };
    let [graph] = graphs.as_slice() else {
        return Ok(None);
    };
    if *graph == 0 {
        return Ok(None);
    }
    let [pattern] = patterns else {
        return Ok(None);
    };
    let (TermPattern::Variable(subject), NamedNodePattern::NamedNode(predicate)) =
        (&pattern.subject, &pattern.predicate)
    else {
        return Ok(None);
    };
    if !matches!(
        pattern.object,
        TermPattern::NamedNode(_) | TermPattern::Literal(_)
    ) {
        return Ok(None);
    }
    let Some(predicate) = store.lookup(predicate.as_str())? else {
        return Ok(None);
    };
    let Some(object) = super::pattern_util::resolve_object_pattern(store, &pattern.object, seed)?
    else {
        return Ok(None);
    };
    if object == Value::Ref(-1) {
        return Ok(None);
    }
    let indexes: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN
            ('idx_active_vge','idx_current_aev')",
        [],
        |row| row.get(0),
    )?;
    if indexes != 2 {
        return Ok(None);
    }
    let mut statement = store.prepare(
        "SELECT DISTINCT value_fact.e FROM facts AS value_fact INDEXED BY idx_active_vge
        WHERE value_fact.v=?1 AND value_fact.g=?2
        AND value_fact.op=1 AND value_fact.valid_to IS NULL
        AND EXISTS (SELECT 1 FROM facts AS predicate_fact INDEXED BY idx_current_aev
            WHERE predicate_fact.a=?3 AND predicate_fact.e=value_fact.e
            AND predicate_fact.v=?1 AND predicate_fact.rowid=value_fact.rowid
            AND predicate_fact.op=1 AND predicate_fact.valid_to IS NULL)",
    )?;
    let mut rows = statement.query(rusqlite::params![object.to_bytes(), graph, predicate])?;
    let mut bindings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while let Some(row) = rows.next()? {
        super::pattern_util::check_eval_budget(ctx, bindings.len(), bindings.len())?;
        let subject_id = store.canonical_id(row.get(0)?)?;
        if !seen.insert(subject_id) {
            continue;
        }
        let mut binding = Bindings::new();
        binding.insert(subject.as_str().to_string(), Value::Ref(subject_id));
        bindings.push(binding);
    }
    Ok(Some((bindings, vec![subject.as_str().to_string()])))
}

/// Project one property's bindings for a local literal/STR NOT IN filter.
///
/// Both variables remain bound and the original FILTER still runs. Admission
/// excludes arbitrary expressions, outer variables and every unsupported scope.
///
/// # Errors
/// Propagates dictionary, SQL, value decoding and query-budget errors.
pub fn filtered_property_candidates(
    store: &Store,
    expr: &Expression,
    inner: &GraphPattern,
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<Option<(Vec<Bindings>, Vec<String>)>> {
    if !seed.is_empty()
        || !store.attachments().is_empty()
        || ctx.valid_at.is_some()
        || ctx.as_of_tx.is_some()
        || ctx.row_limit.is_some()
        || ctx.string_narrows.is_some()
        || ctx.entails_rdfs
    {
        return Ok(None);
    }
    let GraphScope::Named(graphs) = &ctx.graph else {
        return Ok(None);
    };
    let [graph] = graphs.as_slice() else {
        return Ok(None);
    };
    if *graph == 0 {
        return Ok(None);
    }
    let GraphPattern::Bgp { patterns } = inner else {
        return Ok(None);
    };
    let [pattern] = patterns.as_slice() else {
        return Ok(None);
    };
    let (
        TermPattern::Variable(subject),
        NamedNodePattern::NamedNode(predicate),
        TermPattern::Variable(object),
    ) = (&pattern.subject, &pattern.predicate, &pattern.object)
    else {
        return Ok(None);
    };
    if subject == object || !local_literal_not_in(expr, object.as_str()) {
        return Ok(None);
    }
    let Some(predicate) = store.lookup(predicate.as_str())? else {
        return Ok(None);
    };
    let indexes: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN
            ('idx_current_aev','idx_current_g')",
        [],
        |row| row.get(0),
    )?;
    if indexes != 2 {
        return Ok(None);
    }
    let mut statement = store.prepare(
        "SELECT DISTINCT property.e, property.v FROM facts AS property INDEXED BY idx_current_aev
        WHERE property.a=?1 AND property.op=1 AND property.valid_to IS NULL
        AND EXISTS (SELECT 1 FROM facts AS graph_fact INDEXED BY idx_current_g
            WHERE graph_fact.g=?2 AND graph_fact.rowid=property.rowid
            AND graph_fact.op=1 AND graph_fact.valid_to IS NULL)",
    )?;
    let mut rows = statement.query(rusqlite::params![predicate, graph])?;
    let mut bindings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while let Some(row) = rows.next()? {
        super::pattern_util::check_eval_budget(ctx, bindings.len(), bindings.len())?;
        let subject_id = store.canonical_id(row.get(0)?)?;
        let bytes: Vec<u8> = row.get(1)?;
        let value = match Value::from_bytes(&bytes)? {
            Value::Ref(id) => Value::Ref(store.canonical_id(id)?),
            value => value,
        };
        if !seen.insert((subject_id, value.to_bytes())) {
            continue;
        }
        let mut binding = Bindings::new();
        binding.insert(subject.as_str().to_string(), Value::Ref(subject_id));
        binding.insert(object.as_str().to_string(), value);
        bindings.push(binding);
    }
    Ok(Some((
        bindings,
        vec![subject.as_str().to_string(), object.as_str().to_string()],
    )))
}
