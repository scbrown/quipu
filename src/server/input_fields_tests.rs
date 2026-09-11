//! Regression checks at the HTTP handler boundary, with isolated stores.
use super::*;
use axum::{Json, extract::State, http::HeaderMap};
use serde_json::json;
use std::sync::Arc;

fn store() -> super::super::SharedStore {
    Arc::new(super::super::StoreHandle::writer_only(
        quipu::Store::open_in_memory().unwrap(),
    ))
}

#[test]
fn schemas_accept_declared_fields_and_report_sorted_unknowns() {
    for definition in quipu::tool_definitions() {
        let name = definition["name"].as_str().unwrap();
        let input = definition["inputSchema"]["properties"].clone();
        assert!(ignored(name, &input).is_empty(), "{name}");
        assert_eq!(annotate(name, &input, json!({})), json!({}));
        let mut extra = input;
        extra["zzz_unknown"] = json!(true);
        extra["aaa_unknown"] = json!(null);
        assert_eq!(
            annotate(name, &extra, json!({"result": 1})),
            json!({"result": 1, "ignored_fields": ["aaa_unknown", "zzz_unknown"]}),
            "{name}"
        );
    }
}

#[test]
fn every_macro_handler_resolves_a_schema() {
    let source = syn::parse_file(include_str!("tools.rs")).unwrap();
    let mut count = 0;
    for item in source.items {
        let syn::Item::Macro(item) = item else {
            continue;
        };
        if !["ro_handler", "rw_handler", "embed_handler"]
            .iter()
            .any(|name| item.mac.path.is_ident(name))
        {
            continue;
        }
        let tokens = item.mac.tokens.to_string();
        let tool = tokens
            .split_once(',')
            .unwrap()
            .1
            .trim()
            .trim_end_matches(',');
        if tool.contains("tool_load_ontology") && !cfg!(feature = "owl") {
            continue;
        }
        assert_eq!(ignored(tool, &json!({"__unknown": 1})), vec!["__unknown"]);
        count += 1;
    }
    assert!(count > 30, "macro coverage control: {count}");
}

#[test]
fn graphiti_adapter_reports_verbose_but_accepts_its_actual_fields() {
    let input = json!({"query":"sample", "group_ids":[], "max_results":2,
        "entity_type_filter":"https://example.org/Type", "verbose":true});
    assert_eq!(
        ignored("quipu :: mcp :: graphiti :: tool_search_nodes", &input),
        vec!["verbose"]
    );
    assert!(ignored("quipu::tool_search_nodes", &input).is_empty());
}

#[tokio::test]
async fn knot_unknown_dry_run_reports_warning_and_still_writes() {
    let shared = store();
    let response = super::super::publication::knot(
        State(shared.clone()),
        Json(json!({
            "turtle": "<https://example.org/item> <https://example.org/value> \"written\" .",
            "dry_run": true
        })),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(response["ignored_fields"], json!(["dry_run"]));
    let result = quipu::tool_query(
        &shared.read(),
        &json!({
            "query":"SELECT ?v WHERE { <https://example.org/item> <https://example.org/value> ?v }"
        }),
    )
    .unwrap();
    assert_eq!(
        result["count"], 1,
        "reporting must preserve the existing write"
    );
    let response = super::super::publication::knot(
        State(shared),
        Json(json!({
            "turtle": "<https://example.org/clean> <https://example.org/value> \"written\" ."
        })),
    )
    .await
    .unwrap()
    .0;
    assert!(response.get("ignored_fields").is_none());
}

#[tokio::test]
async fn handler_errors_are_preserved() {
    assert!(
        super::super::publication::knot(
            State(store()),
            Json(json!({
                "dry_run": true
            }))
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn macro_read_and_write_handlers_report_fields() {
    let shared = store();
    let response = super::super::tools::search_nodes(
        State(shared.clone()),
        Json(json!({
            "query":"nothing", "typo":true
        })),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(response["ignored_fields"], json!(["typo"]));
    let response = super::super::tools::datasets(
        State(shared),
        Json(json!({
            "action":"list", "typo":true
        })),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(response["ignored_fields"], json!(["typo"]));
}

#[tokio::test]
async fn query_json_reports_fields_and_standard_json_preserves_format() {
    let shared = store();
    let input = json!({"query":"SELECT ?s WHERE { ?s ?p ?o }", "dry_run":true});
    let response = super::super::query_endpoint::query(
        State(shared.clone()),
        HeaderMap::new(),
        Json(input.clone()),
    )
    .await
    .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: JsonValue = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["ignored_fields"], json!(["dry_run"]));
    let mut headers = HeaderMap::new();
    headers.insert("accept", "application/sparql-results+json".parse().unwrap());
    let response = super::super::query_endpoint::query(State(shared), headers, Json(input))
        .await
        .unwrap();
    assert_eq!(
        response.headers()["x-quipu-ignored-fields"],
        "[\"dry_run\"]"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: JsonValue = serde_json::from_slice(&body).unwrap();
    assert!(value.get("ignored_fields").is_none());
    assert!(value.get("head").is_some());
    assert!(value.get("results").is_some());
}

#[tokio::test]
async fn export_preserves_rdf_and_reports_unknown_fields_in_header() {
    let response = super::super::publication::export(
        State(store()),
        Json(json!({
            "format":"ntriples", "dry_run":true
        })),
    )
    .await
    .unwrap();
    assert_eq!(response.headers()["content-type"], "application/n-triples");
    assert_eq!(
        response.headers()["x-quipu-ignored-fields"],
        "[\"dry_run\"]"
    );
}

#[test]
fn headers_encode_arbitrary_field_names_and_omit_clean_requests() {
    let mut headers = HeaderMap::new();
    query_header(
        &json!({"query":"ASK {}", "federated":false, "_sparql_protocol":true}),
        &mut headers,
    );
    assert!(headers.get("x-quipu-ignored-fields").is_none());
    let input = json!({"é\n🦀": true});
    header("quipu_knot", &input, &mut headers);
    let names: Vec<String> =
        serde_json::from_str(headers["x-quipu-ignored-fields"].to_str().unwrap()).unwrap();
    assert_eq!(names, vec!["é\n🦀"]);
}
