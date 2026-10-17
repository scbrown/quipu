//! Read-only evaluation of an explicitly pinned premise view.
//!
//! This is the evaluation seam, not catalogue admission: callers must select
//! and validate the governed edition and certify retained-history coverage in
//! the same source snapshot. No current companion is imported as a premise.

use std::collections::BTreeSet;

use crate::sparql::{GraphScope, QueryResult, TemporalContext, query_temporal};
use crate::store::Store;
use crate::types::Value;

use super::RuleSet;

/// A derived relation whose terms remain valid after the isolated store dies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct HistoricalFact {
    /// Subject IRI, not a dictionary ID from either store.
    pub subject: String,
    /// Predicate IRI.
    pub predicate: String,
    /// Object IRI. The existing Datalog engine supports reference tuples.
    pub object: String,
}

/// Explicit refusal at the historical evaluation boundary.
#[derive(Debug, thiserror::Error)]
pub enum HistoricalError {
    /// Both independent time coordinates must be supplied.
    #[error("historical evaluation requires valid_at and as_of_tx")]
    MissingPin,
    /// No silent truncation of fractional seconds or malformed input.
    #[error("invalid or unsupported valid-time instant: {0}")]
    InvalidInstant(String),
    /// A cutoff must identify a real transaction in this source.
    #[error("unknown transaction snapshot: {0}")]
    UnknownSnapshot(i64),
    /// File-local transaction IDs cannot pin attached providers.
    #[error("historical evaluation refuses attached providers")]
    AttachedProviders,
    /// Scope must be a single explicit committed premise graph.
    #[error("historical evaluation requires one explicit premise graph")]
    UnsupportedScope,
    /// Empty rules are not a successful historical answer.
    #[error("historical evaluation refuses an empty ruleset")]
    EmptyRules,
    /// Preserve unknown coverage rather than returning a truncated answer.
    #[error("historical evaluation exceeds the bounded execution profile")]
    Capacity,
    /// The current engine ignores literal tuples; historical mode refuses.
    #[error("historical Datalog evaluation does not support literal premises")]
    LiteralPremise,
    /// Store/query failure, including timeout and policy refusals.
    #[error(transparent)]
    Store(#[from] crate::Error),
    /// Rule compilation or evaluation failure.
    #[error(transparent)]
    Reasoner(#[from] super::ReasonerError),
}

/// Evaluate selected rules over a dual-time premise view without source writes.
///
/// The caller owns the read transaction spanning catalogue selection AND this
/// call. This function does not assert that the source retained all history:
/// that evidence belongs to the catalogue admission boundary. It preserves
/// the caller's temporal query budgets and refuses attached-store snapshots.
/// Every derivation happens in owned bounded relations, so repeated and
/// concurrent contexts cannot share output or mutate the source.
pub fn evaluate_temporal_view(
    source: &Store,
    rules: &RuleSet,
    context: &TemporalContext,
) -> Result<Vec<HistoricalFact>, HistoricalError> {
    let (Some(at), Some(tx)) = (&context.valid_at, context.as_of_tx) else {
        return Err(HistoricalError::MissingPin);
    };
    if at.as_bytes().get(19) == Some(&b'.') || at.get(17..19) == Some("60") {
        return Err(HistoricalError::InvalidInstant(at.clone()));
    }
    let at = crate::time::normalize_rfc3339_utc(at)
        .ok_or_else(|| HistoricalError::InvalidInstant(at.clone()))?;
    if at.len() != 20 || !at.as_bytes()[..4].iter().all(u8::is_ascii_digit) {
        return Err(HistoricalError::InvalidInstant(at));
    }
    if !source.attachments().is_empty() {
        return Err(HistoricalError::AttachedProviders);
    }
    if tx <= 0 || source.get_transaction(tx)?.is_none() {
        return Err(HistoricalError::UnknownSnapshot(tx));
    }
    let graph = match &context.graph {
        GraphScope::Default(g) | GraphScope::Named(g) if g.len() == 1 => g[0],
        _ => return Err(HistoricalError::UnsupportedScope),
    };
    if graph != crate::schema::ROOT_GRAPH
        && crate::store::inferred::is_inferred_graph_iri(&source.resolve(graph)?)
    {
        return Err(HistoricalError::UnsupportedScope);
    }
    if rules.is_empty() {
        return Err(HistoricalError::EmptyRules);
    }
    // A caller-imposed output LIMIT can silently truncate the premise query.
    // Reject it rather than dropping a policy ceiling or claiming completeness.
    if context.row_limit.is_some() || context.result_limit.is_some() {
        return Err(HistoricalError::Capacity);
    }
    let predicates: BTreeSet<_> = rules
        .rules
        .iter()
        .flat_map(|r| r.body.iter().map(|a| a.atom().predicate.as_str()))
        .collect();
    if predicates.len() > 64 {
        return Err(HistoricalError::Capacity);
    }
    let values = predicates
        .iter()
        .map(|p| oxrdf::NamedNode::new(*p).map(|iri| iri.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::Error::InvalidValue(e.to_string()))?
        .join(" ");
    let ctx = TemporalContext {
        deadline: context
            .deadline
            .or_else(crate::time::request_deadline)
            .or_else(|| {
                let ms = source.search_config().query_timeout_ms;
                (ms > 0).then(|| crate::time::Deadline::after_millis(ms))
            }),
        valid_at: Some(at.clone()),
        ..context.clone()
    };
    // LIMIT has one sentinel row: reaching the cap is an explicit refusal,
    // never a successful partial premise view. Existing smaller caps still win.
    let configured_cap = source.search_config().max_sparql_rows;
    let sentinel = if configured_cap == 0 {
        10_001
    } else {
        configured_cap.min(10_001)
    };
    let query =
        format!("SELECT ?s ?p ?o WHERE {{ VALUES ?p {{ {values} }} ?s ?p ?o }} LIMIT {sentinel}");
    let QueryResult::Select { rows, .. } = query_temporal(source, &query, &ctx)? else {
        unreachable!("a SELECT query returns SELECT rows")
    };
    if rows.len() >= sentinel {
        return Err(HistoricalError::Capacity);
    }
    let mut premises = Vec::with_capacity(rows.len());
    for row in rows {
        let (Some(Value::Ref(s)), Some(Value::Ref(p)), Some(Value::Ref(o))) =
            (row.get("s"), row.get("p"), row.get("o"))
        else {
            return Err(HistoricalError::LiteralPremise);
        };
        premises.push(HistoricalFact {
            subject: source.resolve(*s)?,
            predicate: source.resolve(*p)?,
            object: source.resolve(*o)?,
        });
    }
    let configured_work = context
        .row_cap
        .unwrap_or(source.search_config().max_join_rows);
    let work_cap = if configured_work == 0 {
        1_000_000
    } else {
        configured_work.min(1_000_000)
    };
    super::historical_bounded::evaluate(rules, premises, sentinel, work_cap, ctx.deadline)
}
