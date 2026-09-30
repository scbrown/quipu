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
    base::{AppError, blocking},
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
    let update = match content_type {
        "application/sparql-update" => std::str::from_utf8(&body)
            .map_err(|e| {
                quipu::Error::InvalidValue(format!("SPARQL update body is not UTF-8: {e}"))
            })?
            .to_string(),
        "application/x-www-form-urlencoded" => {
            let fields: Vec<_> = url::form_urlencoded::parse(&body).collect();
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
    blocking(move || apply_update(&store, &format!("BASE <{base}>\n{update}"))).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
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

/// What one update did: the dataset path and the datums it transacted.
///
/// Only the tests read it back; production uses the counters above.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct Applied {
    pub(super) path: UpdatePath,
    pub(super) changes: Vec<(i64, Vec<quipu::store::Datum>)>,
}

fn apply_update(shared: &SharedStore, update: &str) -> Result<(), AppError> {
    apply_update_as(shared, update, false).map(|_| ())
}

/// Evaluate `update` with Oxigraph and transact the before/after diff.
///
/// The dataset is the slice [`update_slice::plan`] names, or a copy of the
/// whole store when it cannot name one (or `force_full`, which tests use to
/// compare both paths). Planning parses only, so it runs before the lock.
pub(super) fn apply_update_as(
    shared: &SharedStore,
    update: &str,
    force_full: bool,
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
        Plan::Sliced(touched) => {
            load_slice(&store, &ox, &graphs, touched)?;
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
    let batches: Vec<_> = changes.into_iter().collect();
    store.transact_graph_batches(&batches, &now, Some("sparql-update"), Some("sparql-update"))?;
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
