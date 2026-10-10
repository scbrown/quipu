//! SPARQL 1.1 Graph Store HTTP Protocol adapter.

use axum::{
    body::Bytes,
    extract::{Extension, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use oxrdf::NamedNode;
use serde::Deserialize;

use super::{SharedStore, base::blocking, tools::finish_deferred_embed};

pub(crate) fn routes() -> axum::Router<SharedStore> {
    axum::Router::new().route(
        "/rdf-graph-store",
        axum::routing::get(graph_store_get)
            .head(graph_store_get)
            .put(graph_store_put)
            .post(graph_store_post)
            .delete(graph_store_delete),
    )
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct GraphSelector {
    graph: Option<String>,
    default: Option<String>,
    transport: Option<String>,
}

impl GraphSelector {
    #[allow(clippy::result_large_err)]
    fn target(&self) -> Result<Option<&str>, Response> {
        match (self.graph.as_deref(), self.default.is_some()) {
            (Some(_), true) | (None, false) => Err((
                StatusCode::BAD_REQUEST,
                "select exactly one of ?graph=<absolute-IRI> or ?default",
            )
                .into_response()),
            (Some(iri), false) if NamedNode::new(iri).is_err() => {
                Err((StatusCode::BAD_REQUEST, "graph must be an absolute IRI").into_response())
            }
            (Some(iri), false) => Ok(Some(iri)),
            (None, true) => Ok(None),
        }
    }
}

#[allow(clippy::result_large_err)]
fn response_format(headers: &HeaderMap) -> Result<(oxrdfio::RdfFormat, &'static str), Response> {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/turtle");
    if accept.contains("text/turtle") || accept.contains("*/*") {
        Ok((oxrdfio::RdfFormat::Turtle, "text/turtle"))
    } else if accept.contains("application/n-triples") {
        Ok((oxrdfio::RdfFormat::NTriples, "application/n-triples"))
    } else {
        Err((
            StatusCode::NOT_ACCEPTABLE,
            "supported RDF responses: text/turtle, application/n-triples",
        )
            .into_response())
    }
}

#[allow(clippy::result_large_err)]
fn request_format(headers: &HeaderMap) -> Result<oxrdfio::RdfFormat, Response> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.starts_with("text/turtle") {
        Ok(oxrdfio::RdfFormat::Turtle)
    } else if content_type.starts_with("application/n-triples") {
        Ok(oxrdfio::RdfFormat::NTriples)
    } else {
        Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "supported RDF payloads: text/turtle, application/n-triples",
        )
            .into_response())
    }
}

pub(crate) async fn graph_store_get(
    State(store): State<SharedStore>,
    Query(selector): Query<GraphSelector>,
    headers: HeaderMap,
) -> Response {
    let graph = match selector.target() {
        Ok(v) => v.map(str::to_owned),
        Err(r) => return r,
    };
    if let Some(transport) = selector.transport.as_deref() {
        if transport != "compact-v1" {
            return (StatusCode::BAD_REQUEST, "unknown RDF transport").into_response();
        }
        let Some(graph) = graph else {
            return (
                StatusCode::BAD_REQUEST,
                "compact transport requires a named graph",
            )
                .into_response();
        };
        return super::graph_store_compact::get(store, graph).await;
    }
    let (format, content_type) = match response_format(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match blocking(move || {
        let store = store.read();
        if let Some(iri) = graph.as_deref() {
            let exists = store
                .lookup(iri)?
                .and_then(|g| store.graph_class(g).ok().flatten())
                .is_some();
            if !exists {
                return Ok(None);
            }
        }
        let (bytes, _) = quipu::export_rdf_subset(&store, format, graph.as_deref())?;
        Ok(Some(bytes))
    })
    .await
    {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type)],
            bytes,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn write_graph(
    store: SharedStore,
    selector: GraphSelector,
    headers: HeaderMap,
    body: Bytes,
    principal: Option<Extension<quipu::http_auth::AuthenticatedPrincipal>>,
    replace: bool,
) -> Response {
    let graph = match selector.target() {
        Ok(v) => v.map(str::to_owned),
        Err(r) => return r,
    };
    let format = match request_format(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let actor = principal.map(|Extension(p)| p.as_str().to_owned());
    let writer = store.clone();
    match blocking(move || {
        let mut store = writer.lock();
        let existed = graph.as_deref().is_none_or(|iri| {
            store
                .lookup(iri)
                .ok()
                .flatten()
                .and_then(|g| store.graph_class(g).ok().flatten())
                .is_some()
        });
        let result = if replace {
            quipu::replace_rdf_graph(
                &mut store,
                body.as_ref(),
                format,
                graph.as_deref(),
                &quipu::time::now_iso(),
                actor.as_deref(),
                Some("graph-store-put"),
                graph.as_deref(),
            )
        } else {
            let g = match graph.as_deref() {
                None => 0,
                Some(iri) => store.graph_create(iri)?,
            };
            quipu::rdf::ingest_rdf_to_graph(
                &mut store,
                body.as_ref(),
                format,
                graph.as_deref(),
                &quipu::time::now_iso(),
                actor.as_deref(),
                Some("graph-store-post"),
                g,
            )
        }?;
        let deferred = store.take_deferred_embed();
        Ok((existed, result, deferred))
    })
    .await
    {
        Ok((existed, _, deferred)) => {
            if let Some(work) = deferred
                && let Err(e) = finish_deferred_embed(&store, &work)
            {
                return e.into_response();
            }
            if existed {
                StatusCode::NO_CONTENT.into_response()
            } else {
                StatusCode::CREATED.into_response()
            }
        }
        Err(e) => e.into_response(),
    }
}

pub(crate) async fn graph_store_put(
    State(store): State<SharedStore>,
    Query(selector): Query<GraphSelector>,
    headers: HeaderMap,
    principal: Option<Extension<quipu::http_auth::AuthenticatedPrincipal>>,
    body: Bytes,
) -> Response {
    write_graph(store, selector, headers, body, principal, true).await
}

pub(crate) async fn graph_store_post(
    State(store): State<SharedStore>,
    Query(selector): Query<GraphSelector>,
    headers: HeaderMap,
    principal: Option<Extension<quipu::http_auth::AuthenticatedPrincipal>>,
    body: Bytes,
) -> Response {
    write_graph(store, selector, headers, body, principal, false).await
}

pub(crate) async fn graph_store_delete(
    State(store): State<SharedStore>,
    Query(selector): Query<GraphSelector>,
    principal: Option<Extension<quipu::http_auth::AuthenticatedPrincipal>>,
) -> Response {
    let graph = match selector.target() {
        Ok(v) => v.map(str::to_owned),
        Err(r) => return r,
    };
    let actor = principal.map(|Extension(p)| p.as_str().to_owned());
    match blocking(move || {
        let mut store = store.lock();
        if let Some(iri) = graph.as_deref() {
            let exists = store
                .lookup(iri)?
                .and_then(|g| store.graph_class(g).ok().flatten())
                .is_some();
            if !exists {
                return Ok(false);
            }
        }
        quipu::delete_rdf_graph(
            &mut store,
            &quipu::time::now_iso(),
            actor.as_deref(),
            graph.as_deref(),
        )?;
        Ok(true)
    })
    .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        extract::{Query, State},
        http::{HeaderMap, StatusCode, header},
    };

    use super::super::StoreHandle;
    use super::*;

    fn named() -> GraphSelector {
        GraphSelector {
            graph: Some("http://example.org/graphs/interop".into()),
            default: None,
            transport: None,
        }
    }

    fn content_type(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, value.parse().unwrap());
        headers
    }

    #[tokio::test]
    async fn protocol_methods_round_trip_named_and_default_graphs() {
        let shared: SharedStore = Arc::new(StoreHandle::writer_only(
            quipu::Store::open_in_memory().unwrap(),
        ));
        let one = Bytes::from_static(b"<http://example.org/a> <http://example.org/p> \"one\" .");
        let two = Bytes::from_static(b"<http://example.org/b> <http://example.org/p> \"two\" .");

        let put = graph_store_put(
            State(shared.clone()),
            Query(named()),
            content_type("application/n-triples"),
            None,
            one,
        )
        .await;
        assert_eq!(put.status(), StatusCode::CREATED);

        let post = graph_store_post(
            State(shared.clone()),
            Query(named()),
            content_type("application/n-triples"),
            None,
            two,
        )
        .await;
        assert_eq!(post.status(), StatusCode::NO_CONTENT);

        let mut accept = HeaderMap::new();
        accept.insert(header::ACCEPT, "application/n-triples".parse().unwrap());
        let get = graph_store_get(State(shared.clone()), Query(named()), accept).await;
        assert_eq!(get.status(), StatusCode::OK);
        let body = axum::body::to_bytes(get.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("/a>") && body.contains("/b>"), "{body}");

        let replace = graph_store_put(
            State(shared.clone()),
            Query(named()),
            content_type("application/n-triples"),
            None,
            Bytes::from_static(b"<http://example.org/c> <http://example.org/p> \"three\" ."),
        )
        .await;
        assert_eq!(replace.status(), StatusCode::NO_CONTENT);

        let delete = graph_store_delete(State(shared.clone()), Query(named()), None).await;
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);
        let gone = graph_store_get(State(shared.clone()), Query(named()), HeaderMap::new()).await;
        assert_eq!(gone.status(), StatusCode::NOT_FOUND);

        let default = GraphSelector {
            graph: None,
            transport: None,
            default: Some(String::new()),
        };
        let root = graph_store_put(
            State(shared.clone()),
            Query(default),
            content_type("application/n-triples"),
            None,
            Bytes::from_static(b"<http://example.org/root> <http://example.org/p> \"ok\" ."),
        )
        .await;
        assert_eq!(root.status(), StatusCode::NO_CONTENT);

        // HEAD is registered to the same representation handler; Axum strips
        // its body at routing time. Method-sensitive auth is tested in
        // http_auth::tests::graph_store_classification_is_method_sensitive.
        assert!(!quipu::http_auth::is_write_request(
            "/rdf-graph-store",
            "HEAD"
        ));
    }
    #[tokio::test]
    async fn compact_transport_is_explicit_named_complete_and_bound() {
        use sha2::{Digest, Sha256};
        let shared: SharedStore = Arc::new(StoreHandle::writer_only(
            quipu::Store::open_in_memory().unwrap(),
        ));
        let source = b"<http://example.org/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Action> .\n<http://example.org/a> <https://schema.org/version> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
        let put = graph_store_put(
            State(shared.clone()),
            Query(named()),
            content_type("application/n-triples"),
            None,
            Bytes::from_static(source),
        )
        .await;
        assert_eq!(put.status(), StatusCode::CREATED);
        let mut selector = named();
        selector.transport = Some("compact-v1".into());
        let response =
            graph_store_get(State(shared.clone()), Query(selector), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-quipu-rdf-transport"], "compact-v1");
        assert_eq!(response.headers()["x-quipu-triples"], "2");
        assert_eq!(response.headers()["x-quipu-actions"], "1");
        let expected_graph = format!("{:x}", Sha256::digest(b"http://example.org/graphs/interop"));
        assert_eq!(response.headers()["x-quipu-graph-sha256"], expected_graph);
        let hash = response.headers()["x-quipu-body-sha256"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), quipu::COMPACT_RDF_LIMIT)
            .await
            .unwrap();
        assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)));
        let parsed: Vec<_> = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle)
            .for_reader(bytes.as_ref())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].is_ok());
        println!(
            "COMPACT_FIXTURE {}",
            serde_json::json!({
                "graph": "http://example.org/graphs/interop",
                "body": String::from_utf8(bytes.to_vec()).unwrap(),
                "sha256": hash, "triples": 2, "actions": 1
            })
        );
        let root = GraphSelector {
            graph: None,
            default: Some(String::new()),
            transport: Some("compact-v1".into()),
        };
        assert_eq!(
            graph_store_get(State(shared.clone()), Query(root), HeaderMap::new())
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let mut wrong = named();
        wrong.transport = Some("unknown".into());
        assert_eq!(
            graph_store_get(State(shared), Query(wrong), HeaderMap::new())
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
