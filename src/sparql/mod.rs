//! SPARQL query engine -- evaluates SPARQL queries over the EAVT fact log.
//!
//! Parses SPARQL via spargebra, then evaluates against the `SQLite` fact store.
//! Supports: SELECT, ASK, CONSTRUCT, DESCRIBE with BGP, JOIN, UNION, FILTER,
//! OPTIONAL (`LeftJoin`), VALUES, ORDER BY, GROUP BY, aggregates, HAVING,
//! EXTEND, RDFS subclass inference, PROJECT, DISTINCT, REDUCED, LIMIT/OFFSET.

pub mod aggregate;
pub mod filter;
pub mod pattern;
pub mod pattern_util;
pub mod property_path;
pub mod rdfs;
#[cfg(test)]
mod tests;

mod construct;
mod join;
mod progress;
mod sql_in;
pub mod triple;
pub mod values;

use std::collections::HashMap;

use spargebra::{Query, SparqlParser};

use crate::error::{Error, Result};
use crate::store::Store;
// The query-budget progress guard lives in `progress` (size ratchet split).
use crate::types::Value;
use progress::ProgressGuard;

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
    Named(Vec<i64>),
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

    /// The one graph a single-graph scope names (quipu-nip): `Some(g)` for a
    /// one-element `Default` (the service default, a one-graph `FROM`, or the
    /// `graph` request param) or `Named` (`GRAPH <iri>`), `None` for unions
    /// and `GRAPH ?g`.
    pub(crate) fn single_graph(&self) -> Option<i64> {
        match self {
            GraphScope::Default(g) | GraphScope::Named(g) if g.len() == 1 => Some(g[0]),
            _ => None,
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
    pub deadline: Option<crate::time::Deadline>,
    /// Abort evaluation when an intermediate binding set exceeds this many
    /// rows (None = derive from the store's `max_join_rows`; 0 there
    /// disables). A join explosion is stopped the moment it is recognizable
    /// instead of burning the whole wall-clock budget at 100% CPU.
    pub row_cap: Option<usize>,
    /// LIMIT pushdown (quipu-0lr): stop producing BGP solutions once this
    /// many exist. Installed by the `Slice` operator ONLY when every operator
    /// between it and the BGP leaf is prefix-safe (`Project`/`Reduced`), so
    /// stopping early cannot change the answer — a bounded query stops
    /// scanning once it has enough rows instead of completing the scan and
    /// discarding. `None` = no cap (every pre-existing path).
    pub row_limit: Option<usize>,
}

/// Execute a SPARQL query against the store (current state).
pub fn query(store: &Store, sparql: &str) -> Result<QueryResult> {
    query_temporal(store, sparql, &TemporalContext::default())
}

/// The variable a `GRAPH ?g { … }` binds, if the query has one (quipu #70).
///
/// Per-row labels are only free — and only *meaningful* — where the graph is
/// already bound per row. Everywhere else the dataset label (#67) is the
/// correct granularity, and projecting `g` to fake a per-row one would change
/// the results rather than annotate them (§4).
///
/// The parse is what identifies the graph variable: a row binding an IRI is
/// indistinguishable from any other IRI-valued variable once evaluation is
/// done.
fn graph_variable(pattern: &spargebra::algebra::GraphPattern) -> Option<String> {
    use spargebra::algebra::GraphPattern as GP;
    use spargebra::term::NamedNodePattern;
    match pattern {
        GP::Graph { name, inner } => match name {
            NamedNodePattern::Variable(v) => Some(v.as_str().to_string()),
            NamedNodePattern::NamedNode(_) => graph_variable(inner),
        },
        GP::Join { left, right } | GP::Union { left, right } => {
            graph_variable(left).or_else(|| graph_variable(right))
        }
        GP::LeftJoin { left, right, .. } => graph_variable(left).or_else(|| graph_variable(right)),
        GP::Minus { left, right } => graph_variable(left).or_else(|| graph_variable(right)),
        GP::Filter { inner, .. }
        | GP::Extend { inner, .. }
        | GP::OrderBy { inner, .. }
        | GP::Project { inner, .. }
        | GP::Distinct { inner }
        | GP::Reduced { inner }
        | GP::Slice { inner, .. }
        | GP::Group { inner, .. } => graph_variable(inner),
        _ => None,
    }
}

/// Annotate each row with the label of the graph it came from (quipu #70).
///
/// Adds `_freshness` / `_trust` / `_policy` columns using the exact pattern
/// `FederatedProvider::query_all` already uses for `_provider`.
///
/// # Errors
/// When the annotated rows span **more than one trust chain**. Nothing here
/// composes ranks — but the worked example this exists for is
/// `ORDER BY DESC(?rank)`, and ordering ranks drawn from two chains is the
/// silent cross-chain comparison the trust axis refuses (§6). Better to refuse
/// the annotation than to hand back a result that sorts meaninglessly.
fn annotate_row_labels(store: &Store, result: QueryResult, g_var: &str) -> Result<QueryResult> {
    let QueryResult::Select {
        mut variables,
        rows,
    } = result
    else {
        // ASK has no rows and CONSTRUCT emits triples, not bindings — neither
        // has a per-row graph to annotate.
        return Ok(result);
    };
    for col in ["_freshness", "_trust", "_policy"] {
        if !variables.iter().any(|v| v == col) {
            variables.push(col.to_string());
        }
    }

    let mut seen_chain: Option<String> = None;
    let mut annotated = Vec::with_capacity(rows.len());
    for row in rows {
        let mut row = row;
        if let Some(&Value::Ref(gid)) = row.get(g_var) {
            let l = store.label_of_id(gid)?;
            if let Some(f) = l.freshness.value {
                row.insert("_freshness".into(), Value::Str(f.as_str().into()));
            }
            if let Some(t) = &l.trust.value {
                match &seen_chain {
                    Some(prev) if prev != &t.chain => {
                        return Err(Error::InvalidValue(format!(
                            "per-row trust labels span two chains ('{prev}' and '{}'). \
                             Ranks are only comparable within a declared chain, so an \
                             ORDER BY over these rows would be meaningless — refused \
                             rather than sorted.",
                            t.chain
                        )));
                    }
                    Some(_) => {}
                    None => seen_chain = Some(t.chain.clone()),
                }
                row.insert("_trust".into(), Value::Str(t.iri.clone()));
            }
            if let Some(p) = &l.policy.value {
                row.insert("_policy".into(), Value::Str(p.tokens().join(" ")));
            }
        }
        annotated.push(row);
    }

    Ok(QueryResult::Select {
        variables,
        rows: annotated,
    })
}

/// Execute a query, annotating each row with its source graph's label.
///
/// **Opt-in, and only under `GRAPH ?g`.** A query with no graph variable is
/// evaluated and returned unchanged: there is no per-row graph to label, and
/// inventing one would mean projecting `g`, which changes results rather than
/// annotating them.
///
/// # Errors
/// As [`query_temporal`], plus a refusal when the annotated rows span more than
/// one trust chain.
pub fn query_row_labeled(
    store: &Store,
    sparql: &str,
    ctx: &TemporalContext,
) -> Result<QueryResult> {
    let parsed = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::InvalidValue(format!("SPARQL parse error: {e}")))?;
    let g_var = match &parsed {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => graph_variable(pattern),
    };

    let result = query_temporal(store, sparql, ctx)?;
    match g_var {
        Some(v) => annotate_row_labels(store, result, &v),
        None => Ok(result),
    }
}

/// A query result together with its dataset's composed label (quipu #67).
///
/// Not `Clone`: `QueryResult` is not, and adding it would mean touching the
/// type this issue is explicit about leaving alone.
#[derive(Debug)]
pub struct LabeledResult {
    /// The ordinary result — **byte-identical** to what [`query_temporal`]
    /// returns for the same query.
    pub result: QueryResult,
    /// The dataset's composed label, or `None` when no member graph declared
    /// anything. `None` is *undeclared*, never a fabricated `fresh`/⊤/⊥.
    pub labels: Option<crate::store::labels::DatasetLabels>,
}

/// Execute a query and also report its dataset's composed label.
///
/// A **new entry point, deliberately**: [`QueryResult`] is unchanged and
/// [`query_temporal`] keeps its signature, because many internal callers match
/// on the result and want nothing more (graph-labels.md §4.1). Nothing on the
/// existing path is touched, so there is no regression to measure on it — the
/// extra parse below is paid only by callers that ask for labels.
///
/// The label is a property of the **dataset**, computed once here — not per
/// row. See [`Store::dataset_labels`] for why per-solution labelling is a
/// semantic blocker rather than a cost knob.
///
/// # Errors
/// Parse and evaluation errors as [`query_temporal`], plus a refusal when the
/// dataset's member graphs carry trust from different chains (#66).
pub fn query_labeled(store: &Store, sparql: &str, ctx: &TemporalContext) -> Result<LabeledResult> {
    let labels = dataset_labels_for(store, sparql, ctx)?;
    let result = query_temporal(store, sparql, ctx)?;
    Ok(LabeledResult { result, labels })
}

/// The composed label of the dataset a query would read, **without running it**.
///
/// Split out so the `/query` and `quipu_query` surfaces can attach a `"labels"`
/// key beside their existing result without executing the query twice, and so
/// there is exactly ONE implementation of "which graphs does this query read".
///
/// Returns `None` when no member graph declared anything on any axis —
/// *undeclared*, reported as `"labels": null` rather than as a fabricated
/// `fresh`/⊤/⊥.
///
/// # Errors
/// Parse errors, and a refusal when member graphs carry trust from different
/// chains (#66).
pub fn dataset_labels_for(
    store: &Store,
    sparql: &str,
    ctx: &TemporalContext,
) -> Result<Option<crate::store::labels::DatasetLabels>> {
    let member_ids = dataset_member_ids(store, sparql, ctx)?;

    // quipu #68: the floor is checked against the MEMBERS, before composing —
    // the fold says the dataset is stale, only the members say which one is.
    // A no-op when no floor is configured.
    store.check_label_floor(&member_ids)?;

    let composed = store.dataset_labels(&member_ids)?;
    Ok(if composed.is_undeclared() {
        None
    } else {
        Some(composed)
    })
}

/// The interned ids of the graphs a query's dataset would read.
///
/// The ONE implementation of "which graphs does this query read", shared by the
/// label fold and the #68 floor check. Both resolve by calling the same
/// `apply_dataset` evaluation uses, so neither can drift from what is actually
/// read — labelling or gating a dataset the query does not read is the failure
/// this exists to prevent.
///
/// # Errors
/// SPARQL parse errors, and store errors while resolving graph IRIs.
pub fn dataset_member_ids(store: &Store, sparql: &str, ctx: &TemporalContext) -> Result<Vec<i64>> {
    // Resolve the dataset exactly the way evaluation will, by running the same
    // `apply_dataset` over the same parsed dataset clause. A second
    // implementation of the resolution would be free to drift from the one that
    // decides what the query actually reads — labelling a dataset the query
    // does not read is precisely the failure this is meant to prevent.
    let parsed = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::InvalidValue(format!("SPARQL parse error: {e}")))?;
    let dataset = match &parsed {
        Query::Select { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. }
        | Query::Ask { dataset, .. } => dataset.clone(),
    };
    let scoped = apply_dataset(store, dataset.as_ref(), ctx)?;

    let member_ids: Vec<i64> = match &scoped.graph {
        GraphScope::Default(ids) => ids.clone(),
        GraphScope::Named(ids) => ids.clone(),
        // `GRAPH ?g` with no `FROM NAMED` ranges every named graph, so the fold
        // does too — the label has to cover what the query could read.
        GraphScope::AnyNamed { restrict, .. } => match restrict {
            Some(ids) => ids.clone(),
            None => store.all_named_graph_ids()?,
        },
    };

    Ok(member_ids)
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
    if ctx.as_of_tx.is_some() && !store.attachments().is_empty() {
        return Err(Error::Store(
            "as_of_tx is refused for a store with attached databases: transaction IDs are \
             file-local and have no cross-file ordering; see \
             docs/design/multi-db-composition.md §6"
                .to_string(),
        ));
    }

    let parsed = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::InvalidValue(format!("SPARQL parse error: {e}")))?;

    let started = crate::time::Stopwatch::start();
    let deadline = ctx.deadline.or_else(|| {
        let limit_ms = store.search_config().query_timeout_ms;
        (limit_ms > 0).then(|| crate::time::Deadline::after_millis(limit_ms))
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
    let _guard = deadline
        .map(|dl| ProgressGuard::install(&store.conn, dl))
        .transpose()?;

    let result = eval_parsed(store, parsed, &ctx);
    match result {
        // Any failure past the deadline is reported as the timeout it is: the
        // proximate error (a SQLITE_INTERRUPT, or the evaluator's own check)
        // is just the mechanism that stopped the query.
        Err(_) if deadline.is_some_and(|dl| dl.passed()) => Err(Error::QueryTimeout {
            elapsed_ms: started.elapsed_ms(),
            limit_ms: deadline
                .map(|dl| dl.millis_from(&started))
                .unwrap_or_default(),
        }),
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
            let triples = construct::eval_construct(store, &template, &rows)?;
            Ok(QueryResult::Graph(triples))
        }
        Query::Describe {
            pattern, dataset, ..
        } => {
            let ctx = apply_dataset(store, dataset.as_ref(), ctx)?;
            let (rows, _) = pattern::eval_pattern(store, &pattern, &ctx)?;
            let triples = construct::eval_describe(store, &rows)?;
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
    // quipu #69: an IRI that NAMES A DATASET expands to its members here, in
    // the one resolve closure. That placement is the whole design: everything
    // downstream — evaluation, the #67 label fold, the #68 floor check — reads
    // the expanded member set without knowing datasets exist, so none of them
    // can disagree about what the query reads.
    //
    // Checked before the plain graph lookup because a dataset name is a
    // deliberate registration; interned as an ordinary term it would otherwise
    // resolve to a graph id that matches nothing. With no datasets registered
    // the table is empty and this is exactly today's behaviour.
    let resolve = |nodes: &[spargebra::term::NamedNode]| -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        for nn in nodes {
            let iri = nn.as_str();
            if store.is_dataset(iri)? {
                ids.extend(store.dataset_member_ids(iri)?);
            } else {
                ids.extend(store.lookup_all(iri)?);
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
