use super::*;
use axum::routing::{get, post};
use serde_json::json;

async fn echo(request: axum::extract::Request) -> axum::Json<Value> {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1_000_000).await.unwrap();
    axum::Json(
        json!({"method":parts.method.as_str(),"uri":parts.uri.to_string(),
        "type":parts.headers["content-type"].to_str().unwrap(),
        "body":String::from_utf8(bytes.to_vec()).unwrap(),
        "authorization":parts.headers.get("authorization").map(|v|v.to_str().unwrap()),
        "task":parts.headers.get("x-quipu-task").map(|v|v.to_str().unwrap())}),
    )
}
fn payload(result: CallToolResult) -> Value {
    let content = result.content.into_iter().next().unwrap();
    serde_json::from_str(&content.as_text().unwrap().text).unwrap()
}

#[tokio::test]
async fn rule_answer_states_native_mcp_dispatch_preserves_typed_answers_and_read_annotation() {
    use axum::extract::State;
    let mut store = crate::Store::open_in_memory().unwrap();
    store
        .load_shapes(
            "rule-answers",
            include_str!("../../shapes/rule-answers.ttl"),
            "2026-01-01",
        )
        .unwrap();
    crate::tool_knot(&mut store, &json!({"turtle":
        "<urn:supported> <urn:positive> <urn:target> . <urn:refuted> <urn:negative> <urn:target> . <urn:both> <urn:positive> <urn:target> ; <urn:negative> <urn:target> ."
    })).unwrap();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(store));
    async fn handler(
        State(store): State<std::sync::Arc<std::sync::Mutex<crate::Store>>>,
        axum::Json(input): axum::Json<Value>,
    ) -> axum::Json<Value> {
        axum::Json(crate::tool_rule_query(&store.lock().unwrap(), &input).unwrap())
    }
    let server = McpServer::new(
        Router::new()
            .route("/rule_query", post(handler))
            .with_state(shared.clone()),
    );
    let definition = server
        .tools
        .iter()
        .find(|tool| tool.name == "quipu_rule_query")
        .unwrap();
    let definition = serde_json::to_value(definition).unwrap();
    assert_eq!(definition["annotations"]["readOnlyHint"], true);
    assert_eq!(definition["annotations"]["destructiveHint"], false);
    for subject in ["supported", "refuted", "both", "neither"] {
        let input = json!({"subject":format!("urn:{subject}"),"positive_predicate":"urn:positive",
            "negative_predicate":"urn:negative","object":{"iri":"urn:target"},"judgment_policy":"urn:policy"});
        let expected = crate::tool_rule_query(&shared.lock().unwrap(), &input).unwrap();
        let actual = payload(
            server
                .dispatch("quipu_rule_query", input, &HeaderMap::new())
                .await
                .unwrap(),
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["action_authority"], "not_granted");
    }
}

#[tokio::test]
async fn numeric_get_filters_and_fragment_iris_reach_the_existing_route() {
    let server = McpServer::new(
        Router::new()
            .route("/events", get(echo))
            .route("/entity", get(echo)),
    );
    let result = payload(
        server
            .dispatch(
                "quipu_events",
                json!({"since":42,"limit":3,"consumer":"a&b"}),
                &HeaderMap::new(),
            )
            .await
            .unwrap(),
    );
    let uri: axum::http::Uri = result["uri"].as_str().unwrap().parse().unwrap();
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(fields["since"], "42");
    assert_eq!(fields["limit"], "3");
    assert_eq!(fields["consumer"], "a&b");
    let result = payload(
        server
            .dispatch(
                "quipu_entity",
                json!({"iri":"https://example.org/a#b?c=d&z","expanded":true}),
                &HeaderMap::new(),
            )
            .await
            .unwrap(),
    );
    let uri: axum::http::Uri = result["uri"].as_str().unwrap().parse().unwrap();
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(fields["iri"], "https://example.org/a#b?c=d&z");
    assert_eq!(fields["expanded"], "true");
}

#[tokio::test]
async fn update_form_preserves_exact_sparql_and_provenance_and_request_identity() {
    let server = McpServer::new(Router::new().route("/update", post(echo)));
    let update = "INSERT DATA { GRAPH <urn:test:g> { <urn:s> <urn:p> \"a+b&c=é\" } }";
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer fixture-only".parse().unwrap());
    headers.insert("x-quipu-task", "fixture-task".parse().unwrap());
    let result = payload(
        server
            .dispatch(
                "quipu_update",
                json!({"update":update,"actor":"urn:declared:actor","source":"fixture+source"}),
                &headers,
            )
            .await
            .unwrap(),
    );
    assert_eq!(result["type"], "application/x-www-form-urlencoded");
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(result["body"].as_str().unwrap().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(fields["update"], update);
    assert_eq!(fields["actor"], "urn:declared:actor");
    assert_eq!(fields["source"], "fixture+source");
    assert_eq!(result["authorization"], "Bearer fixture-only");
    assert_eq!(result["task"], "fixture-task");
}

#[tokio::test]
async fn wrong_scope_type_unknown_fields_and_missing_scope_refuse_before_dispatch() {
    let server = McpServer::new(
        Router::new()
            .route("/share", post(echo))
            .route("/reason", post(echo)),
    );
    for (name, input) in [
        ("quipu_share", json!({})),
        ("quipu_share", json!({"scope":{"kind":"graph","value":45}})),
        (
            "quipu_share",
            json!({"scope":{"kind":"root"},"destination":"internal"}),
        ),
        ("quipu_reason", json!({"graph":false})),
        ("quipu_reason", json!({"graph_iri":"urn:g"})),
    ] {
        assert!(
            server
                .dispatch(name, input, &HeaderMap::new())
                .await
                .is_err()
        );
    }
    let result = payload(
        server
            .dispatch(
                "quipu_share",
                json!({"scope":{"kind":"graph","value":"urn:g"}}),
                &HeaderMap::new(),
            )
            .await
            .unwrap(),
    );
    assert!(result["body"].as_str().unwrap().contains("urn:g"));
}

#[test]
fn write_annotations_cover_new_workflows_and_reads_stay_read_only() {
    let server = McpServer::new(Router::new());
    for (name, write) in [
        ("quipu_share", false),
        ("quipu_import", true),
        ("quipu_import_promote", true),
        ("quipu_update", true),
        ("quipu_reason", true),
        ("quipu_events", false),
        ("quipu_events_commit", true),
        ("quipu_changes", false),
        ("quipu_subscriptions", true),
        ("quipu_embed_backfill", true),
        ("quipu_knot_stage", true),
        ("quipu_knot_promote", true),
    ] {
        let tool = server.tools.iter().find(|t| t.name == name).unwrap();
        let definition = serde_json::to_value(tool).unwrap();
        assert_eq!(definition["annotations"]["readOnlyHint"], !write, "{name}");
    }
}

#[tokio::test]
async fn search_mcp_hides_diagnostic_neighbors_and_preserves_admitted_positive() {
    for accepted in [false, true] {
        let app = Router::new().route("/search", post(move || async move {
            axum::Json(json!({"results":[{"entity":"urn:wrong","text":"must not be injected","similarity":0.99}],
                "count":1,"scoped":true,"ranking":"semantic","insufficient_context":!accepted,
                "context":{"contract":"quipu-context/v1","outcome":if accepted {"CONTEXT"} else {"NO CONTEXT"},"similarity_floor":0.52},
                "context_results":if accepted {json!([{"entity":"urn:good","text":"admitted positive","similarity":0.8,"source":"knowledge","valid_from":"2026-01-01"}])} else {json!([])}}))
        }));
        let out = payload(
            McpServer::new(app)
                .dispatch(
                    "quipu_search",
                    json!({"query":"subject"}),
                    &HeaderMap::new(),
                )
                .await
                .unwrap(),
        );
        assert!(!out.to_string().contains("urn:wrong"));
        assert_eq!(out["count"], usize::from(accepted));
        assert_eq!(out["scoped"], true);
        assert_eq!(out["ranking"], "semantic");
        assert_eq!(
            out["context"]["outcome"],
            if accepted { "CONTEXT" } else { "NO CONTEXT" }
        );
        if accepted {
            assert_eq!(out["results"][0]["valid_from"], "2026-01-01");
        }
    }
}

#[tokio::test]
async fn search_mcp_unknown_contract_is_an_error_not_an_empty_success() {
    let app = Router::new().route(
        "/search",
        post(|| async { axum::Json(json!({"results":[{"text":"legacy neighbor"}]})) }),
    );
    let out = McpServer::new(app)
        .dispatch(
            "quipu_search",
            json!({"query":"subject"}),
            &HeaderMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(out.is_error, Some(true));
    let text = &out.content[0].as_text().unwrap().text;
    assert!(text.contains("UNKNOWN"));
    assert!(!text.contains("legacy neighbor"));
}
