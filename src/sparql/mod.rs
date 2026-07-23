//! SPARQL query engine -- evaluates SPARQL queries over the EAVT fact log.
//!
//! Parses SPARQL via spargebra, then evaluates against the `SQLite` fact store.
//! Supports: SELECT, ASK, CONSTRUCT, DESCRIBE with BGP, JOIN, UNION, FILTER,
//! OPTIONAL (`LeftJoin`), ORDER BY, GROUP BY, aggregates, HAVING, EXTEND, RDFS
//! subclass inference, PROJECT, DISTINCT, REDUCED, LIMIT/OFFSET.

pub mod aggregate;
pub mod filter;
pub mod pattern;
pub mod pattern_util;
pub mod property_path;
pub mod rdfs;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use spargebra::{Query, SparqlParser};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

/// A single row of variable bindings from a query result.
pub type Bindings = HashMap<String, Value>;

/// An RDF triple (used by CONSTRUCT and DESCRIBE results).
#[derive(Debug, Clone, PartialEq)]
pub struct Triple {
    pub subject: String,
    pub predicate: String,
    pub object: Value,
}

/// Result of a SPARQL query.
#[derive(Debug)]
pub enum QueryResult {
    /// SELECT query result with variable names and binding rows.
    Select {
        variables: Vec<String>,
        rows: Vec<Bindings>,
    },
    /// ASK query result -- true if the pattern has at least one solution.
    Ask(bool),
    /// CONSTRUCT / DESCRIBE result -- a set of triples.
    Graph(Vec<Triple>),
}

impl QueryResult {
    /// Get the variable names (only meaningful for Select results).
    pub fn variables(&self) -> &[String] {
        match self {
            Self::Select { variables, .. } => variables,
            _ => &[],
        }
    }

    /// Get the result rows (only meaningful for Select results).
    pub fn rows(&self) -> &[Bindings] {
        match self {
            Self::Select { rows, .. } => rows,
            _ => &[],
        }
    }
}

/// Temporal context for time-travel SPARQL queries.
#[derive(Debug, Clone, Default)]
pub struct TemporalContext {
    /// Point-in-time for valid-time filtering (None = current only).
    pub valid_at: Option<String>,
    /// Maximum transaction id to consider (None = all).
    pub as_of_tx: Option<i64>,
    /// Abort evaluation once this instant passes (None = derive from the
    /// store's `query_timeout_ms`; see [`query_temporal`]). Checked between
    /// operators in the pattern evaluator and, via a `SQLite` progress
    /// handler, inside long-running `sqlite3_step` calls.
    pub deadline: Option<std::time::Instant>,
}

/// Execute a SPARQL query against the store (current state).
pub fn query(store: &Store, sparql: &str) -> Result<QueryResult> {
    query_temporal(store, sparql, &TemporalContext::default())
}

/// Clears the `SQLite` progress handler on drop, so an early return or error
/// cannot leave a stale deadline interrupting the NEXT query on this
/// connection.
struct ProgressGuard<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> ProgressGuard<'a> {
    /// ~4096 VM instructions between checks: coarse enough to be free on
    /// healthy queries, fine enough to stop a grinding scan within
    /// milliseconds of the deadline.
    fn install(conn: &'a rusqlite::Connection, deadline: std::time::Instant) -> Self {
        conn.progress_handler(4096, Some(move || std::time::Instant::now() >= deadline));
        Self { conn }
    }
}

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        self.conn.progress_handler(0, None::<fn() -> bool>);
    }
}

/// Execute a SPARQL query with temporal context (time-travel).
///
/// Enforces the store's query budget (`[quipu.search] query_timeout_ms`): unless `ctx.deadline` is already set by the caller, a deadline
/// is derived from the store config, a `SQLite` progress handler interrupts
/// statement execution past it, and the evaluator checks it between operators.
/// Past-deadline failures surface as [`Error::QueryTimeout`], not whatever
/// error the interrupt happened to produce.
///
/// The progress handler is installed per top-level query on the store's one
/// connection; a nested `query_temporal` without its own deadline runs under
/// the outer query's handler and budget, which is the intended accounting.
pub fn query_temporal(store: &Store, sparql: &str, ctx: &TemporalContext) -> Result<QueryResult> {
    let parsed = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::InvalidValue(format!("SPARQL parse error: {e}")))?;

    let started = std::time::Instant::now();
    let deadline = ctx.deadline.or_else(|| {
        let limit_ms = store.search_config().query_timeout_ms;
        (limit_ms > 0).then(|| started + std::time::Duration::from_millis(limit_ms))
    });
    let ctx = TemporalContext {
        deadline,
        ..ctx.clone()
    };
    let _guard = deadline.map(|dl| ProgressGuard::install(&store.conn, dl));

    let result = eval_parsed(store, parsed, &ctx);
    match result {
        // Any failure past the deadline is reported as the timeout it is: the
        // proximate error (a SQLITE_INTERRUPT, or the evaluator's own check)
        // is just the mechanism that stopped the query.
        Err(_) if deadline.is_some_and(|dl| std::time::Instant::now() >= dl) => {
            Err(Error::QueryTimeout {
                elapsed_ms: started.elapsed().as_millis(),
                limit_ms: deadline
                    .map(|dl| dl.saturating_duration_since(started).as_millis())
                    .unwrap_or_default(),
            })
        }
        other => other,
    }
}

fn eval_parsed(store: &Store, parsed: Query, ctx: &TemporalContext) -> Result<QueryResult> {
    match parsed {
        Query::Select { pattern, .. } => {
            let (rows, vars) = pattern::eval_pattern(store, &pattern, ctx)?;
            Ok(QueryResult::Select {
                variables: vars,
                rows,
            })
        }
        Query::Ask { pattern, .. } => {
            let (rows, _) = pattern::eval_pattern(store, &pattern, ctx)?;
            Ok(QueryResult::Ask(!rows.is_empty()))
        }
        Query::Construct {
            template, pattern, ..
        } => {
            let (rows, _) = pattern::eval_pattern(store, &pattern, ctx)?;
            let triples = eval_construct(store, &template, &rows)?;
            Ok(QueryResult::Graph(triples))
        }
        Query::Describe { pattern, .. } => {
            let (rows, _) = pattern::eval_pattern(store, &pattern, ctx)?;
            let triples = eval_describe(store, &rows)?;
            Ok(QueryResult::Graph(triples))
        }
    }
}

/// Instantiate a CONSTRUCT template with each row of bindings.
fn eval_construct(
    store: &Store,
    template: &[spargebra::term::TriplePattern],
    rows: &[Bindings],
) -> Result<Vec<Triple>> {
    use spargebra::term::{NamedNodePattern, TermPattern};

    let mut triples = Vec::new();

    for row in rows {
        for tp in template {
            let subject = match &tp.subject {
                TermPattern::NamedNode(n) => n.as_str().to_string(),
                TermPattern::Variable(v) => match row.get(v.as_str()) {
                    Some(Value::Ref(id)) => store.resolve(*id)?,
                    Some(Value::Str(s)) => s.clone(),
                    _ => continue,
                },
                TermPattern::BlankNode(b) => format!("_:{}", b.as_str()),
                TermPattern::Literal(_) => continue,
                #[cfg(feature = "shacl")]
                TermPattern::Triple(_) => continue,
            };

            let predicate = match &tp.predicate {
                NamedNodePattern::NamedNode(n) => n.as_str().to_string(),
                NamedNodePattern::Variable(v) => match row.get(v.as_str()) {
                    Some(Value::Ref(id)) => store.resolve(*id)?,
                    Some(Value::Str(s)) => s.clone(),
                    _ => continue,
                },
            };

            let object = match &tp.object {
                TermPattern::NamedNode(n) => {
                    if let Some(id) = store.lookup(n.as_str())? {
                        Value::Ref(id)
                    } else {
                        Value::Str(n.as_str().to_string())
                    }
                }
                TermPattern::Literal(lit) => filter::literal_to_value(lit),
                TermPattern::Variable(v) => match row.get(v.as_str()) {
                    Some(val) => val.clone(),
                    None => continue,
                },
                TermPattern::BlankNode(b) => Value::Str(format!("_:{}", b.as_str())),
                #[cfg(feature = "shacl")]
                TermPattern::Triple(_) => continue,
            };

            let triple = Triple {
                subject,
                predicate,
                object,
            };
            if !triples.contains(&triple) {
                triples.push(triple);
            }
        }
    }

    Ok(triples)
}

/// Gather all triples for each entity mentioned in the result rows.
fn eval_describe(store: &Store, rows: &[Bindings]) -> Result<Vec<Triple>> {
    let mut entity_ids = Vec::new();

    // Collect all Ref values from all bindings.
    for row in rows {
        for val in row.values() {
            if let Value::Ref(id) = val
                && !entity_ids.contains(id)
            {
                entity_ids.push(*id);
            }
        }
    }

    let mut triples = Vec::new();
    for eid in &entity_ids {
        let facts = store.entity_facts(*eid)?;
        let subject_iri = store.resolve(*eid)?;
        for fact in &facts {
            let predicate = store.resolve(fact.attribute)?;
            let object = match &fact.value {
                Value::Ref(id) => Value::Ref(*id),
                other => other.clone(),
            };
            let triple = Triple {
                subject: subject_iri.clone(),
                predicate,
                object,
            };
            if !triples.contains(&triple) {
                triples.push(triple);
            }
        }
    }

    Ok(triples)
}
