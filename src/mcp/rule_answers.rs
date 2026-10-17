//! The same typed answer as CLI/HTTP; MCP arguments are already JSON objects.

/// Read current ROOT support and admitted judgments, granting no action authority.
pub fn tool_rule_query(
    store: &crate::Store,
    input: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let request: crate::rule_answers::Request = serde_json::from_value(input.clone())
        .map_err(|error| crate::Error::InvalidValue(format!("rule-query request: {error}")))?;
    let answer = crate::rule_answers::query(store, &request)?;
    serde_json::to_value(answer).map_err(|error| crate::Error::Serialization(error.to_string()))
}
