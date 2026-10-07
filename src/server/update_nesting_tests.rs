//! `/update` survives deep and long-chained SPARQL (aegis-rq1afp).
//!
//! The recursive parser overflowed the blocking pool's stack and ABORTED the
//! process at 500 nested `FILTER NOT EXISTS`. A regression here does not fail
//! one test: it kills the test binary, which is the point.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
};
use quipu::Store;
use quipu::sparql_structure::{STRUCTURE_LIMIT, structural_cost};

use super::super::{SharedStore, StoreHandle};
use super::update_post;

fn fresh() -> SharedStore {
    Arc::new(StoreHandle::writer_only(Store::open_in_memory().unwrap()))
}

/// `levels` nested `FILTER NOT EXISTS` groups: the shape that overflowed first.
fn nested(levels: usize) -> String {
    let mut inner = "?s ?p ?o".to_string();
    for _ in 0..levels {
        inner = format!("?s ?p ?o FILTER NOT EXISTS {{ {inner} }}");
    }
    format!("DELETE {{ ?s ?p ?o }} WHERE {{ {inner} }}")
}

async fn post(store: &SharedStore, update: String) -> (StatusCode, String) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/sparql-update"),
    );
    let Ok(response) = update_post(
        State(store.clone()),
        headers,
        OriginalUri(Uri::from_static("/update")),
        Bytes::from(update),
    )
    .await
    else {
        panic!("update_post returned an error");
    };
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// The deepest request of each shape the limit admits.
fn at_limit_shapes() -> Vec<(&'static str, String)> {
    let filler = |body: String| format!("DELETE {{ ?s ?p ?o }} WHERE {{ {body} }}");
    // `nested` costs 3 per level ({, FILTER, EXISTS) plus the two outer braces.
    let nested_levels = (STRUCTURE_LIMIT - 2) / 3;
    let flat_union = vec!["{ ?s ?p ?o }"; (STRUCTURE_LIMIT - 1) / 2].join(" UNION ");
    let flat_guards = vec!["FILTER NOT EXISTS { ?s ?p ?o }"; (STRUCTURE_LIMIT - 2) / 3].join(" ");
    let mut nest_union = "{ ?s ?p ?o }".to_string();
    for _ in 0..(STRUCTURE_LIMIT - 2) / 3 {
        nest_union = format!("{{ {nest_union} UNION {{ ?s ?p ?o }} }}");
    }
    let parens = format!(
        "{}1{}",
        "(".repeat(STRUCTURE_LIMIT - 4),
        ")".repeat(STRUCTURE_LIMIT - 4)
    );
    vec![
        ("nested FILTER NOT EXISTS", nested(nested_levels)),
        ("flat UNION chain", filler(flat_union)),
        (
            "sibling FILTER NOT EXISTS",
            filler(format!("?s ?p ?o {flat_guards}")),
        ),
        (
            "nested UNION",
            format!("DELETE {{ ?s ?p ?o }} WHERE {nest_union}"),
        ),
        (
            "nested parentheses",
            filler(format!("?s ?p ?o FILTER({parens} = 1)")),
        ),
    ]
}

/// Runs in the test (DEBUG) profile on purpose: its frames are ~25x larger
/// than release, so a shape that fits here fits the deployed build.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_shape_at_the_limit_runs_on_a_stack_that_holds_it() {
    let store = fresh();
    for (shape, update) in at_limit_shapes() {
        let cost = structural_cost(&update);
        assert!(cost <= STRUCTURE_LIMIT, "{shape}: cost {cost}");
        assert!(
            cost + 8 > STRUCTURE_LIMIT,
            "{shape}: cost {cost} is not AT the limit"
        );
        let (status, body) = post(&store, update).await;
        assert_eq!(status, StatusCode::OK, "{shape}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_over_the_limit_is_refused_before_parsing_and_the_server_lives() {
    let store = fresh();
    let (status, body) = post(&store, nested(5000)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("nests too deeply"), "{body}");
    // A long FLAT chain, depth 1 in braces, overflowed too: it must be refused,
    // not judged by brace depth alone.
    let chain = format!(
        "DELETE {{ ?s ?p ?o }} WHERE {{ {} }}",
        vec!["{ ?s ?p ?o }"; 10_000].join(" UNION ")
    );
    let (status, body) = post(&store, chain).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Still serving.
    let (status, body) = post(&store, "INSERT DATA { <urn:a> <urn:b> <urn:c> }".into()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
