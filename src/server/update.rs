//! SPARQL 1.1 Update protocol adapter.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use oxigraph::{
    model::{GraphName, NamedNode, NamedOrBlankNode, Quad},
    store::Store as OxStore,
};

use super::{
    SharedStore,
    base::{AppError, blocking_deep},
    update_slice::{self, Plan, Subjects},
};

pub(crate) async fn update_post(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map_or("", str::trim);
    let mut form_fields: Vec<(String, String)> = Vec::new();
    let update = match content_type {
        "application/sparql-update" => std::str::from_utf8(&body)
            .map_err(|e| {
                quipu::Error::InvalidValue(format!("SPARQL update body is not UTF-8: {e}"))
            })?
            .to_string(),
        "application/x-www-form-urlencoded" => {
            let fields: Vec<_> = url::form_urlencoded::parse(&body).collect();
            form_fields = fields
                .iter()
                .filter(|(k, _)| k == "actor" || k == "source")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let updates: Vec<_> = fields
                .iter()
                .filter_map(|(k, v)| (k == "update").then_some(v.as_ref()))
                .collect();
            if updates.len() != 1 {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    "form body must contain exactly one update parameter",
                )
                    .into_response());
            }
            updates[0].to_string()
        }
        _ => return Ok((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/sparql-update or application/x-www-form-urlencoded",
        )
            .into_response()),
    };
    // Before ANY parser sees it: a deep or long-chained update overflows the
    // recursive parser and aborts the process (aegis-rq1afp).
    if let Err(e) = quipu::sparql_structure::check(&update) {
        return Ok((StatusCode::BAD_REQUEST, e.to_string()).into_response());
    }
    let parameters: Vec<_> = uri
        .query()
        .map(|query| url::form_urlencoded::parse(query.as_bytes()).collect())
        .unwrap_or_default();
    if parameters
        .iter()
        .any(|(name, _)| name == "default-graph-uri" || name == "named-graph-uri")
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            "query dataset parameters are invalid for SPARQL Update",
        )
            .into_response());
    }
    let attribution = match Attribution::from_fields(
        parameters
            .iter()
            .map(|(k, v)| (k.as_ref(), v.as_ref()))
            .chain(form_fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))),
    ) {
        Ok(a) => a,
        Err(message) => return Ok((StatusCode::BAD_REQUEST, message).into_response()),
    };
    let mut using = String::new();
    for (name, value) in &parameters {
        match name.as_ref() {
            "using-graph-uri" => using.push_str(&format!(" USING <{value}>")),
            "using-named-graph-uri" => using.push_str(&format!(" USING NAMED <{value}>")),
            _ => {}
        }
    }
    if !using.is_empty()
        && update
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| word.eq_ignore_ascii_case("using") || word.eq_ignore_ascii_case("with"))
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            "protocol using-graph-uri conflicts with an update USING clause",
        )
            .into_response());
    }
    let update = if using.is_empty() {
        update
    } else {
        let position = update.to_ascii_uppercase().rfind("WHERE").ok_or_else(|| {
            quipu::Error::InvalidValue(
                "using-graph-uri requires a DELETE/INSERT WHERE operation".into(),
            )
        })?;
        format!("{}{} {}", &update[..position], using, &update[position..])
    };
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let base = format!("http://{host}{}", uri.path());
    let report = blocking_deep(move || {
        apply_update_reported(&store, &format!("BASE <{base}>\n{update}"), &attribution)
    })
    .await?;
    Ok(axum::Json(report).into_response())
}

static UPDATES_SLICED: AtomicU64 = AtomicU64::new(0);
static UPDATES_FULL: AtomicU64 = AtomicU64::new(0);

/// How many `/update` requests evaluated over a slice versus the whole store
/// (aegis-jm1lcl). A rising `full` count names updates still paying O(store).
pub(crate) fn render_update_paths(out: &mut String) {
    out.push_str(
        "# HELP quipu_sparql_update_evaluations_total SPARQL updates evaluated, by dataset path (sliced or full store copy).\n\
         # TYPE quipu_sparql_update_evaluations_total counter\n",
    );
    for (path, counter) in [("sliced", &UPDATES_SLICED), ("full", &UPDATES_FULL)] {
        let _ = writeln!(
            out,
            "quipu_sparql_update_evaluations_total{{path=\"{path}\"}} {}",
            counter.load(Ordering::Relaxed)
        );
    }
}

/// Which dataset an update was evaluated over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdatePath {
    /// Only the facts the update can match or write (see [`update_slice`]).
    Sliced,
    /// A copy of every current fact in every dataset graph.
    Full,
}

/// What one update did: the dataset path, the datums it transacted, and the
/// transaction each graph's batch committed as (`(graph, tx)`, batch order).
pub(super) struct Applied {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) path: UpdatePath,
    pub(super) changes: Vec<(i64, Vec<quipu::store::Datum>)>,
    pub(super) txs: Vec<(i64, i64)>,
}

impl Applied {
    /// The `/update` response body (aegis-xajsgn). SPARQL 1.1 Protocol leaves a
    /// successful update's body to the implementation; quipu reports what it
    /// committed so a conditional `DELETE/INSERT ... WHERE` caller can tell
    /// whether its precondition matched without a racy read-back:
    /// `asserted == 0 && retracted == 0` means the WHERE matched nothing (or the
    /// update was a no-op) and `tx` is null. With changes in several graphs each
    /// graph commits its own transaction; `tx` is the last of them.
    pub(super) fn report(&self, store: &quipu::Store) -> Result<serde_json::Value, AppError> {
        let mut graphs = Vec::with_capacity(self.changes.len());
        let (mut asserted, mut retracted) = (0usize, 0usize);
        for ((graph, datums), (_, tx)) in self.changes.iter().zip(&self.txs) {
            let a = datums.iter().filter(|d| d.op == quipu::Op::Assert).count();
            let r = datums.len() - a;
            asserted += a;
            retracted += r;
            let iri = if *graph == 0 {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(store.resolve(*graph)?)
            };
            graphs.push(serde_json::json!({
                "graph": iri, "tx": tx, "asserted": a, "retracted": r,
            }));
        }
        Ok(serde_json::json!({
            "tx": self.txs.last().map(|(_, tx)| *tx),
            "asserted": asserted,
            "retracted": retracted,
            "graphs": graphs,
        }))
    }
}

/// Who a write says it is from (`actor`) and through what (`source`), as the
/// caller declares them (aegis-7vlk7j). Declared, like `/knot`'s fields: the
/// VERIFIED caller is recorded separately as the transaction's `authenticated`
/// principal, so a declared actor attributes a write without impersonating
/// anyone. An undeclared actor is recorded as unknown (null), never as the
/// endpoint's name; `source` defaults to `sparql-update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attribution {
    pub(super) actor: Option<String>,
    pub(super) source: String,
}

impl Default for Attribution {
    fn default() -> Self {
        Self {
            actor: None,
            source: "sparql-update".to_owned(),
        }
    }
}

impl Attribution {
    const MAX_LEN: usize = 256;

    /// From the protocol query parameters and form fields; `actor` and `source`
    /// may each appear once, non-empty, at most 256 chars, no control chars.
    pub(super) fn from_fields<'a>(
        fields: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let mut out = Self::default();
        let (mut actor, mut source) = (None, None);
        for (name, value) in fields {
            let slot = match name {
                "actor" => &mut actor,
                "source" => &mut source,
                _ => continue,
            };
            if slot.is_some() {
                return Err(format!("{name} may be given once"));
            }
            if value.is_empty()
                || value.chars().count() > Self::MAX_LEN
                || value.chars().any(char::is_control)
            {
                return Err(format!(
                    "{name} must be 1..={} characters with no control characters",
                    Self::MAX_LEN
                ));
            }
            *slot = Some(value.to_owned());
        }
        out.actor = actor;
        if let Some(source) = source {
            out.source = source;
        }
        Ok(out)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn apply_update(shared: &SharedStore, update: &str) -> Result<serde_json::Value, AppError> {
    apply_update_reported(shared, update, &Attribution::default())
}

fn apply_update_reported(
    shared: &SharedStore,
    update: &str,
    attribution: &Attribution,
) -> Result<serde_json::Value, AppError> {
    let applied = apply_update_attributed(shared, update, false, attribution)?;
    let store = shared.lock();
    applied.report(&store)
}

/// Evaluate `update` with Oxigraph and transact the before/after diff.
///
/// The dataset is the slice [`update_slice::plan`] names, or a copy of the
/// whole store when it cannot name one (or `force_full`, which tests use to
/// compare both paths). Planning parses only, so it runs before the lock.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn apply_update_as(
    shared: &SharedStore,
    update: &str,
    force_full: bool,
) -> Result<Applied, AppError> {
    apply_update_attributed(shared, update, force_full, &Attribution::default())
}

/// [`apply_update_as`] with the caller's declared [`Attribution`].
pub(super) fn apply_update_attributed(
    shared: &SharedStore,
    update: &str,
    force_full: bool,
    attribution: &Attribution,
) -> Result<Applied, AppError> {
    let plan = if force_full {
        Plan::Full("forced")
    } else {
        update_slice::plan(update)
    };
    let mut store = shared.lock();
    let ox = OxStore::new().map_err(|e| quipu::Error::Store(e.to_string()))?;
    let mut graph_ids = HashMap::new();
    graph_ids.insert(GraphName::DefaultGraph, 0);
    let mut graphs = vec![(0, GraphName::DefaultGraph)];
    for graph_id in store.all_named_graph_ids()? {
        let iri = store.resolve(graph_id)?;
        let name = GraphName::NamedNode(
            NamedNode::new(iri).map_err(|e| quipu::Error::InvalidValue(e.to_string()))?,
        );
        graph_ids.insert(name.clone(), graph_id);
        graphs.push((graph_id, name));
    }
    let path = match &plan {
        Plan::Full(_) => {
            for (graph_id, graph) in &graphs {
                for fact in store.current_facts_in_graph(*graph_id)? {
                    insert_fact(&store, &ox, fact.entity, fact.attribute, &fact.value, graph)?;
                }
            }
            UPDATES_FULL.fetch_add(1, Ordering::Relaxed);
            UpdatePath::Full
        }
        Plan::Sliced(touched, whole) => {
            load_slice(&store, &ox, &graphs, touched, whole)?;
            UPDATES_SLICED.fetch_add(1, Ordering::Relaxed);
            UpdatePath::Sliced
        }
    };
    let before: HashSet<Quad> = ox
        .iter()
        .collect::<Result<_, _>>()
        .map_err(|e| quipu::Error::Store(e.to_string()))?;
    ox.update(update)
        .map_err(|e| quipu::Error::InvalidValue(format!("SPARQL update error: {e}")))?;
    let after: HashSet<Quad> = ox
        .iter()
        .collect::<Result<_, _>>()
        .map_err(|e| quipu::Error::Store(e.to_string()))?;
    let now = quipu::time::now_iso();
    let mut changes: HashMap<i64, Vec<quipu::store::Datum>> = HashMap::new();
    for quad in before.difference(&after) {
        let graph_id = graph_id(&store, &mut graph_ids, &quad.graph_name)?;
        changes
            .entry(graph_id)
            .or_default()
            .push(quipu::store::Datum {
                entity: quipu::rdf::intern_subject(&store, &quad.subject)?,
                attribute: store.intern(quad.predicate.as_str())?,
                value: quipu::rdf::term_to_value(&store, &quad.object)?,
                valid_from: now.clone(),
                valid_to: Some(now.clone()),
                op: quipu::Op::Retract,
            });
    }
    for quad in after.difference(&before) {
        let graph_id = graph_id(&store, &mut graph_ids, &quad.graph_name)?;
        changes
            .entry(graph_id)
            .or_default()
            .push(quipu::store::Datum {
                entity: quipu::rdf::intern_subject(&store, &quad.subject)?,
                attribute: store.intern(quad.predicate.as_str())?,
                value: quipu::rdf::term_to_value(&store, &quad.object)?,
                valid_from: now.clone(),
                valid_to: None,
                op: quipu::Op::Assert,
            });
    }
    let mut batches: Vec<_> = changes.into_iter().collect();
    // Deterministic commit (and report) order across graphs; ROOT first.
    batches.sort_unstable_by_key(|(graph, _)| *graph);
    let txs = store.transact_graph_batches_tx(
        &batches,
        &now,
        attribution.actor.as_deref(),
        Some(&attribution.source),
    )?;
    // Register every named graph this update asserted into, so the next
    // update's dataset includes it (aegis-e9o5ci). After the commit, so a
    // refused write leaves no empty registry row behind.
    for (graph_id, datums) in &batches {
        if datums.iter().any(|d| d.op == quipu::Op::Assert) {
            store.graph_ensure_registered(*graph_id)?;
        }
    }
    Ok(Applied {
        path,
        changes: batches,
        txs,
    })
}

/// Copy the planned slice — each touched predicate, for all or only the named
/// subjects — out of every dataset graph with one indexed read per predicate
/// group. A predicate or subject never interned has no facts and is skipped.
fn load_slice(
    store: &quipu::Store,
    ox: &OxStore,
    graphs: &[(i64, GraphName)],
    touched: &std::collections::BTreeMap<String, Subjects>,
    whole: &std::collections::BTreeSet<String>,
) -> Result<(), AppError> {
    let dataset: HashMap<i64, &GraphName> = graphs.iter().map(|(id, name)| (*id, name)).collect();
    let mut every_subject = Vec::new();
    let mut reads = Vec::new();
    for (predicate, subjects) in touched {
        let attributes = store.lookup_all(predicate)?;
        if attributes.is_empty() {
            continue;
        }
        match subjects {
            Subjects::All => every_subject.extend(attributes),
            Subjects::Only(iris) => {
                let mut entities = Vec::new();
                for iri in iris {
                    entities.extend(store.lookup_all(iri)?);
                }
                reads.push((attributes, Some(entities)));
            }
        }
    }
    // Every current fact of each `whole` subject, one indexed read per entity
    // and dataset graph (a variable predicate on a constant subject).
    for iri in whole {
        for entity in store.lookup_all(iri)? {
            for (g, graph) in graphs {
                for fact in store.entity_facts_in_graph(entity, *g)? {
                    insert_fact(store, ox, fact.entity, fact.attribute, &fact.value, graph)?;
                }
            }
        }
    }
    reads.push((every_subject, None));
    for (attributes, entities) in &reads {
        for (g, entity, attribute, value) in
            store.current_graph_facts_for_attributes(attributes, entities.as_deref())?
        {
            if let Some(graph) = dataset.get(&g) {
                insert_fact(store, ox, entity, attribute, &value, graph)?;
            }
        }
    }
    Ok(())
}

fn insert_fact(
    store: &quipu::Store,
    ox: &OxStore,
    entity: i64,
    attribute: i64,
    value: &quipu::Value,
    graph: &GraphName,
) -> Result<(), AppError> {
    let subject_iri = store.resolve(entity)?;
    let subject = if let Some(id) = subject_iri.strip_prefix("_:") {
        NamedOrBlankNode::BlankNode(
            oxigraph::model::BlankNode::new(id)
                .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?,
        )
    } else {
        NamedOrBlankNode::NamedNode(
            NamedNode::new(subject_iri).map_err(|e| quipu::Error::InvalidValue(e.to_string()))?,
        )
    };
    let predicate = NamedNode::new(store.resolve(attribute)?)
        .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?;
    ox.insert(&Quad::new(
        subject,
        predicate,
        quipu::rdf::value_to_term(store, value)?,
        graph.clone(),
    ))
    .map_err(|e| quipu::Error::Store(e.to_string()))?;
    Ok(())
}

fn graph_id(
    store: &quipu::Store,
    ids: &mut HashMap<GraphName, i64>,
    graph: &GraphName,
) -> Result<i64, AppError> {
    if let Some(id) = ids.get(graph) {
        return Ok(*id);
    }
    let GraphName::NamedNode(name) = graph else {
        return Err(
            quipu::Error::InvalidValue("blank-node graph names are unsupported".into()).into(),
        );
    };
    let id = store.intern(name.as_str())?;
    ids.insert(graph.clone(), id);
    Ok(id)
}

#[cfg(test)]
#[path = "update_bench.rs"]
mod bench;
#[cfg(test)]
#[path = "update_graph_tests.rs"]
mod graph_tests;
#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "update_nesting_tests.rs"]
mod update_nesting_tests;

#[cfg(test)]
#[path = "update_report_tests.rs"]
mod update_report_tests;

#[cfg(test)]
#[path = "update_attribution_tests.rs"]
mod update_attribution_tests;
