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

/// Which graph(s) a pattern is evaluated against (quipu #36).
///
/// `g = 0` is the reserved ROOT / default committed graph. A SPARQL `GRAPH`
/// clause re-scopes the patterns it encloses; without one, evaluation stays on
/// the default graph, which the service seeds to `[0]` (ROOT-only committed
/// reads, Decision 4) but a `FROM` clause (or the `graph` query param) can
/// replace. The scope is threaded through the pattern evaluator via
/// [`TemporalContext`] and swapped (on a clone) when recursing into a
/// `GRAPH { … }` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphScope {
    /// The default graph: facts whose `g` is in this set (the RDF merge of the
    /// listed graphs). The service default is `[0]` (ROOT); `FROM <g…>` replaces
    /// it with the union of those graphs, and an EMPTY set (e.g. `FROM NAMED`
    /// with no `FROM`) matches nothing — never a silent fall-through.
    Default(Vec<i64>),
    /// A single named graph by its interned graph id — `GRAPH <iri> { … }`.
    /// An unknown graph IRI resolves to an id that matches nothing (empty
    /// result), never a silent fall-through to the default graph.
    Named(i64),
    /// Named graphs binding `var` to each match's graph IRI — `GRAPH ?g { … }`.
    /// `restrict` is `None` for every named graph (`g <> 0`), or the `FROM NAMED`
    /// id set when the query restricts the active named graphs.
    AnyNamed {
        /// The `?g` variable name to bind.
        var: String,
        /// `FROM NAMED` restriction (interned ids), or `None` for all named graphs.
        restrict: Option<Vec<i64>>,
    },
}

impl Default for GraphScope {
    /// ROOT-only committed default graph (`g = 0`).
    fn default() -> Self {
        GraphScope::Default(vec![0])
    }
}

impl GraphScope {
    /// True only for the plain ROOT default graph (`g = 0`) — the scope where
    /// RDFS subclass inference and property paths are currently supported.
    /// A `FROM`-redefined default graph, a named graph, or `GRAPH ?g` are all
    /// outside it (those paths would otherwise silently read `g = 0`).
    pub(crate) fn is_root_default(&self) -> bool {
        matches!(self, GraphScope::Default(g) if g.as_slice() == [0])
    }
}

/// Temporal context for time-travel SPARQL queries.
#[derive(Debug, Clone, Default)]
pub struct TemporalContext {
    /// Point-in-time for valid-time filtering (None = current only).
    pub valid_at: Option<String>,
    /// Maximum transaction id to consider (None = all).
    pub as_of_tx: Option<i64>,
    /// Active graph scope for the patterns currently being evaluated (quipu
    /// #36). Defaults to the ROOT/default graph; a `GRAPH` clause re-scopes it.
    pub graph: GraphScope,
    /// The `FROM NAMED` restriction for the whole query (interned graph ids),
    /// set once at query entry (quipu #36). `None` = no `FROM NAMED`, so a
    /// `GRAPH` clause may range over every named graph; `Some(set)` limits which
    /// named graphs a `GRAPH ?g` / `GRAPH <iri>` can see.
    pub named_dataset: Option<Vec<i64>>,
    /// Abort evaluation once this instant passes (None = derive from the
    /// store's `query_timeout_ms`; see [`query_temporal`]). Checked between
    /// operators in the pattern evaluator, INSIDE the pure-Rust join/merge
    /// loops (the mfg0 wedge: an exploded join cloned binding tables at 100%
    /// CPU for hours without touching either of the other two checks), and,
    /// via a `SQLite` progress handler, inside long-running `sqlite3_step`
    /// calls.
    pub deadline: Option<std::time::Instant>,
    /// Abort evaluation when an intermediate binding set exceeds this many
    /// rows (None = derive from the store's `max_join_rows`; 0 there
    /// disables). A join explosion is stopped the moment it is recognizable
    /// instead of burning the whole wall-clock budget at 100% CPU.
    pub row_cap: Option<usize>,
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
    let row_cap = ctx.row_cap.or_else(|| {
        let cap = store.search_config().max_join_rows;
        (cap > 0).then_some(cap)
    });
    let ctx = TemporalContext {
        deadline,
        row_cap,
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
        Query::Select {
            pattern, dataset, ..
        } => {
            let ctx = apply_dataset(store, dataset.as_ref(), ctx)?;
            let (rows, vars) = pattern::eval_pattern(store, &pattern, &ctx)?;
            Ok(QueryResult::Select {
                variables: vars,
                rows,
            })
        }
        Query::Ask {
            pattern, dataset, ..
        } => {
            let ctx = apply_dataset(store, dataset.as_ref(), ctx)?;
            let (rows, _) = pattern::eval_pattern(store, &pattern, &ctx)?;
            Ok(QueryResult::Ask(!rows.is_empty()))
        }
        Query::Construct {
            template,
            pattern,
            dataset,
            ..
        } => {
            let ctx = apply_dataset(store, dataset.as_ref(), ctx)?;
            let (rows, _) = pattern::eval_pattern(store, &pattern, &ctx)?;
            let triples = eval_construct(store, &template, &rows)?;
            Ok(QueryResult::Graph(triples))
        }
        Query::Describe {
            pattern, dataset, ..
        } => {
            let ctx = apply_dataset(store, dataset.as_ref(), ctx)?;
            let (rows, _) = pattern::eval_pattern(store, &pattern, &ctx)?;
            let triples = eval_describe(store, &rows)?;
            Ok(QueryResult::Graph(triples))
        }
    }
}

/// Resolve a query's `FROM` / `FROM NAMED` clauses into a graph-scoped context
/// (quipu #36). With no dataset clause the caller's context is used unchanged
/// (default graph = ROOT, or whatever the `graph` query param set). With a
/// dataset:
///
/// - `FROM <g…>` sets the default graph to the union of those graphs (an unknown
///   graph contributes nothing; an all-unknown / empty set is an empty default).
/// - `FROM NAMED <g…>` restricts which named graphs a `GRAPH` clause can see;
///   a dataset with `FROM` but no `FROM NAMED` activates NO named graphs (per
///   SPARQL 1.1), so `GRAPH` matches nothing.
fn apply_dataset(
    store: &Store,
    dataset: Option<&spargebra::algebra::QueryDataset>,
    ctx: &TemporalContext,
) -> Result<TemporalContext> {
    let Some(ds) = dataset else {
        return Ok(ctx.clone());
    };
    let resolve = |nodes: &[spargebra::term::NamedNode]| -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        for nn in nodes {
            if let Some(g) = store.lookup(nn.as_str())? {
                ids.push(g);
            }
        }
        Ok(ids)
    };
    let default_ids = resolve(&ds.default)?;
    let named_ids = match &ds.named {
        // FROM present, FROM NAMED absent -> no active named graphs.
        None => Some(Vec::new()),
        Some(list) => Some(resolve(list)?),
    };
    Ok(TemporalContext {
        graph: GraphScope::Default(default_ids),
        named_dataset: named_ids,
        ..ctx.clone()
    })
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
