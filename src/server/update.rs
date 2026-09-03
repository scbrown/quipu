//! SPARQL 1.1 Update protocol adapter.

use std::collections::{HashMap, HashSet};

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

fn apply_update(shared: &SharedStore, update: &str) -> Result<(), AppError> {
    let mut store = shared.lock();
    let ox = OxStore::new().map_err(|e| quipu::Error::Store(e.to_string()))?;
    let mut graph_ids = HashMap::new();
    graph_ids.insert(GraphName::DefaultGraph, 0);
    let mut ids = vec![0];
    ids.extend(store.all_named_graph_ids()?);
    for graph_id in ids {
        let graph = if graph_id == 0 {
            GraphName::DefaultGraph
        } else {
            let iri = store.resolve(graph_id)?;
            let name = GraphName::NamedNode(
                NamedNode::new(iri).map_err(|e| quipu::Error::InvalidValue(e.to_string()))?,
            );
            graph_ids.insert(name.clone(), graph_id);
            name
        };
        for fact in store.current_facts_in_graph(graph_id)? {
            let subject_iri = store.resolve(fact.entity)?;
            let subject = if let Some(id) = subject_iri.strip_prefix("_:") {
                NamedOrBlankNode::BlankNode(
                    oxigraph::model::BlankNode::new(id)
                        .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?,
                )
            } else {
                NamedOrBlankNode::NamedNode(
                    NamedNode::new(subject_iri)
                        .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?,
                )
            };
            let predicate = NamedNode::new(store.resolve(fact.attribute)?)
                .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?;
            ox.insert(&Quad::new(
                subject,
                predicate,
                quipu::rdf::value_to_term(&store, &fact.value)?,
                graph.clone(),
            ))
            .map_err(|e| quipu::Error::Store(e.to_string()))?;
        }
    }
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
