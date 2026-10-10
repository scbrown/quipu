//! Optional complete named-graph compact RDF transfer. No census-gate waiver.
use super::{SharedStore, base::blocking};
use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

pub(super) async fn get(store: SharedStore, graph: String) -> Response {
    let graph_sha = format!("{:x}", Sha256::digest(graph.as_bytes()));
    match blocking(move || {
        let store = store.read();
        Ok(quipu::export_compact_graph(&store, &graph)?)
    })
    .await
    {
        Ok(export) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE.as_str(), "text/turtle".to_owned()),
                ("x-quipu-rdf-transport", "compact-v1".to_owned()),
                ("x-quipu-graph-sha256", graph_sha),
                ("x-quipu-body-sha256", export.sha256),
                ("x-quipu-triples", export.triples.to_string()),
                ("x-quipu-actions", export.actions.to_string()),
            ],
            export.bytes,
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}
