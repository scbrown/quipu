//! Every SPARQL entry point is bounded and parses on a deep stack
//! (aegis-rq1afp, aegis-xcvb5z).
//!
//! A regression here does not fail one test: it aborts the test binary.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
};
use quipu::Store;
use quipu::sparql_structure::{STRUCTURE_LIMIT, structural_cost};
use serde_json::json;

use super::{SharedStore, StoreHandle};

fn fresh() -> SharedStore {
    Arc::new(StoreHandle::writer_only(Store::open_in_memory().unwrap()))
}

async fn query(store: &SharedStore, text: &str) -> StatusCode {
    let Ok(response) = super::query_endpoint::query(
        State(store.clone()),
        HeaderMap::new(),
        axum::Json(json!({ "query": text })),
    )
    .await
    else {
        return StatusCode::BAD_REQUEST;
    };
    response.status()
}

async fn update(store: &SharedStore, text: String) -> StatusCode {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/sparql-update"),
    );
    let Ok(response) = super::update::update_post(
        State(store.clone()),
        headers,
        OriginalUri(Uri::from_static("/update")),
        Bytes::from(text),
    )
    .await
    else {
        return StatusCode::BAD_REQUEST;
    };
    response.status()
}

fn select(body: &str) -> String {
    format!("SELECT * WHERE {{ ?s ?p ?o {body} }}")
}

/// A query shape, by size `n`.
type Shape = Box<dyn Fn(usize) -> String>;

/// Query shapes that recurse in the parser, by size `n`.
fn shapes() -> Vec<(&'static str, Shape)> {
    vec![
        (
            "nested parentheses",
            Box::new(|n| select(&format!("FILTER({}1{} = 1)", "(".repeat(n), ")".repeat(n)))),
        ),
        (
            "parentheses after less-than",
            Box::new(|n| {
                select(&format!(
                    "FILTER(?o<{}1{}>?o)",
                    "(".repeat(n),
                    ")".repeat(n)
                ))
            }),
        ),
        (
            "addition chain",
            Box::new(|n| select(&format!("FILTER({} = 1)", vec!["1"; n + 1].join("+")))),
        ),
        (
            "logical-or chain",
            Box::new(|n| select(&format!("FILTER({})", vec!["?o"; n + 1].join("||")))),
        ),
        (
            "path alternative chain",
            Box::new(|n| {
                format!(
                    "SELECT * WHERE {{ ?s {} ?o }}",
                    vec!["<urn:p>"; n + 1].join("|")
                )
            }),
        ),
        (
            "negation chain",
            Box::new(|n| select(&format!("FILTER({}?o)", "!".repeat(n)))),
        ),
        (
            "inverse path chain",
            Box::new(|n| format!("SELECT * WHERE {{ ?s {}<urn:p> ?o }}", "^".repeat(n))),
        ),
        (
            "nested NOT EXISTS",
            Box::new(|n| {
                let mut inner = "?s ?p ?o".to_string();
                for _ in 0..n {
                    inner = format!("?s ?p ?o FILTER NOT EXISTS {{ {inner} }}");
                }
                format!("SELECT * WHERE {{ {inner} }}")
            }),
        ),
        (
            "flat UNION chain",
            Box::new(|n| {
                format!(
                    "SELECT * WHERE {{ {} }}",
                    vec!["{ ?s ?p ?o }"; n + 1].join(" UNION ")
                )
            }),
        ),
        (
            "operator chain inside a long IRI-shaped region",
            Box::new(|n| select(&format!("FILTER(1<{}>0)", vec!["1"; n + 1].join("-")))),
        ),
    ]
}

/// The largest `n` whose cost is within the limit.
fn at_limit(shape: &dyn Fn(usize) -> String) -> String {
    let (mut lo, mut hi) = (1usize, 1 << 16);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if structural_cost(&shape(mid)) <= STRUCTURE_LIMIT {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    shape(lo)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_shape_at_the_limit_answers_and_beyond_it_is_refused_and_the_server_lives() {
    let store = fresh();
    for (name, shape) in shapes() {
        let ok = at_limit(shape.as_ref());
        assert!(structural_cost(&ok) <= STRUCTURE_LIMIT, "{name}");
        let status = query(&store, &ok).await;
        assert_ne!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{name} at the limit"
        );
        // Far beyond: 5x the limit in this shape.
        let big = shape(STRUCTURE_LIMIT * 5);
        assert!(
            structural_cost(&big) > STRUCTURE_LIMIT,
            "{name}: counter blind to it"
        );
        assert_eq!(
            query(&store, &big).await,
            StatusCode::BAD_REQUEST,
            "{name} beyond the limit"
        );
        // Still serving.
        assert_eq!(
            query(&store, "SELECT * WHERE { ?s ?p ?o }").await,
            StatusCode::OK,
            "{name}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_beyond_the_limit_is_refused_and_one_at_it_runs() {
    let store = fresh();
    let deep = |n: usize| {
        let mut inner = "?s ?p ?o".to_string();
        for _ in 0..n {
            inner = format!("?s ?p ?o FILTER NOT EXISTS {{ {inner} }}");
        }
        format!("DELETE {{ ?s ?p ?o }} WHERE {{ {inner} }}")
    };
    assert_eq!(update(&store, at_limit(&deep)).await, StatusCode::OK);
    assert_eq!(update(&store, deep(5000)).await, StatusCode::BAD_REQUEST);
    assert_eq!(
        update(&store, "INSERT DATA { <urn:a> <urn:b> <urn:c> }".into()).await,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seeds_shaped_batches_are_accepted() {
    // wu's acceptance arm: sd's default batch must stay 200.
    let store = fresh();
    let guards: Vec<String> = (0..500)
        .map(|i| {
            format!(
                "FILTER NOT EXISTS {{ GRAPH <urn:board> {{ <urn:seed:{i}> <urn:rev> ?r{i} }} }}"
            )
        })
        .collect();
    let inserts: Vec<String> = (0..500)
        .map(|i| format!("<urn:seed:{i}> <urn:rev> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> ; <urn:title> \"seed (#{i}) - a-b\"@en-US ."))
        .collect();
    let batch = format!(
        "INSERT {{ GRAPH <urn:board> {{ {} }} }} WHERE {{ {} }}",
        inserts.join(" "),
        guards.join(" ")
    );
    assert!(
        structural_cost(&batch) <= STRUCTURE_LIMIT,
        "cost {}",
        structural_cost(&batch)
    );
    assert_eq!(update(&store, batch).await, StatusCode::OK);
    let unions = format!(
        "SELECT * WHERE {{ {} }}",
        (0..500)
            .map(|i| format!("{{ <urn:seed:{i}> ?p ?o }}"))
            .collect::<Vec<_>>()
            .join(" UNION ")
    );
    assert!(structural_cost(&unions) <= STRUCTURE_LIMIT);
    assert_eq!(query(&store, &unions).await, StatusCode::OK);
}
