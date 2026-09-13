//! Report undeclared top-level request keys without changing operation semantics.

use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub(crate) const HTTP_ONLY_FIELDS: &[(&str, &[&str])] = &[
    (
        "quipu_subscriptions",
        &[
            "action",
            "consumer_id",
            "webhook_url",
            "types",
            "mode",
            "sparql_ask",
            "batch_size",
            "batch_window_s",
        ],
    ),
    ("quipu_graph_create", &["graph"]),
    (
        "quipu_graph_label",
        &[
            "graph",
            "timestamp",
            "trust",
            "freshness",
            "durability",
            "policy",
            "kind",
            "valid_to",
            "actor",
        ],
    ),
    ("quipu_reason", &["rules", "prefix", "graph", "timestamp"]),
];

fn schemas() -> &'static BTreeMap<String, BTreeSet<String>> {
    static SCHEMAS: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let mut schemas: BTreeMap<String, BTreeSet<String>> = quipu::tool_definitions()
            .into_iter()
            .map(|definition| {
                let name = definition["name"].as_str().expect("tool name").to_string();
                let keys = definition["inputSchema"]["properties"]
                    .as_object()
                    .expect("tool properties")
                    .keys()
                    .cloned()
                    .collect();
                (name, keys)
            })
            .collect();
        // HTTP-only handlers have no MCP definition to reuse. Keep their fields
        // explicit here and audit them against their handler reads in tests.
        for (name, fields) in HTTP_ONLY_FIELDS {
            schemas.insert((*name).into(), fields.iter().map(|s| (*s).into()).collect());
        }
        schemas
    })
}

pub(crate) fn ignored(tool: &str, input: &JsonValue) -> Vec<String> {
    let tool = tool.replace(' ', "");
    let function = tool.rsplit("::").next().unwrap_or(&tool).trim();
    let name = if function == "tool_graph_view" {
        "quipu_graph".to_string()
    } else {
        function
            .strip_prefix("tool_")
            .map_or_else(|| function.to_string(), |s| format!("quipu_{s}"))
    };
    let properties = schemas()
        .get(&name)
        .unwrap_or_else(|| panic!("REST tool {tool} must have a declared input schema ({name})"));
    let Some(object) = input.as_object() else {
        return Vec::new();
    };
    let mut fields: Vec<_> = object
        .keys()
        .filter(|key| {
            !properties.contains(*key)
                || (tool.contains("::graphiti::") && key.as_str() == "verbose")
        })
        .cloned()
        .collect();
    fields.sort();
    fields
}

pub(crate) fn annotate(tool: &str, input: &JsonValue, mut output: JsonValue) -> JsonValue {
    let ignored = ignored(tool, input);
    if !ignored.is_empty()
        && let Some(object) = output.as_object_mut()
    {
        object.insert("ignored_fields".to_string(), serde_json::json!(ignored));
    }
    output
}

/// Query-only transport controls are consumed by the HTTP adapter.
fn query_fields(input: &JsonValue) -> JsonValue {
    let mut fields = input.clone();
    if let Some(object) = fields.as_object_mut() {
        object.remove("federated");
        object.remove("_sparql_protocol");
    }
    fields
}

pub(crate) fn annotate_query(input: &JsonValue, output: JsonValue) -> JsonValue {
    annotate("quipu_query", &query_fields(input), output)
}

pub(crate) fn query_header(input: &JsonValue, headers: &mut axum::http::HeaderMap) {
    header("quipu_query", &query_fields(input), headers);
}

pub(crate) fn header(tool: &str, input: &JsonValue, headers: &mut axum::http::HeaderMap) {
    let fields = ignored(tool, input);
    if !fields.is_empty() {
        // ASCII JSON keeps arbitrary Unicode/control characters safe in an HTTP header.
        let json = serde_json::to_string(&fields).expect("field names serialize");
        let ascii: String = json
            .chars()
            .flat_map(|c| {
                if c.is_ascii() {
                    c.to_string().chars().collect::<Vec<_>>()
                } else {
                    c.encode_utf16(&mut [0; 2])
                        .iter()
                        .flat_map(|unit| format!("\\u{unit:04x}").chars().collect::<Vec<_>>())
                        .collect()
                }
            })
            .collect();
        if let Ok(value) = axum::http::HeaderValue::from_str(&ascii) {
            headers.insert("x-quipu-ignored-fields", value);
        }
    }
}

#[cfg(test)]
#[path = "input_fields_tests.rs"]
mod tests;
