//! Bounded, structured HTTP usage metadata for request logs.
//!
//! This module deliberately records shapes and counts rather than request or
//! response bodies. Operators can answer how Quipu is used without copying
//! bearer credentials, arbitrary graph content, or high-cardinality query text
//! into the log index.

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Authorization result established by the auth middleware.
pub enum AuthOutcome {
    /// Inner middleware has not yet completed or supplied a result.
    Pending,
    /// The endpoint is a read and requires no credential.
    NotRequired,
    /// The endpoint is a write on a server configured without a token.
    OpenWrite,
    /// The current configured bearer authenticated the write.
    AuthenticatedCurrent,
    /// The temporary previous bearer authenticated during its grace window.
    AuthenticatedPrevious,
    /// An additive named credential authenticated the write.
    AuthenticatedNamed,
    /// A signed write (session attestation) authenticated the write.
    AuthenticatedAttested,
    /// The write lacked the configured bearer or supplied the wrong one.
    Unauthorized,
    /// Server read-only mode refused the write regardless of credentials.
    ReadOnly,
}

impl AuthOutcome {
    /// Stable, low-cardinality value written to the structured log.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::NotRequired => "not_required",
            Self::OpenWrite => "open_write",
            Self::AuthenticatedCurrent => "authenticated_current",
            Self::AuthenticatedPrevious => "authenticated_previous",
            Self::AuthenticatedNamed => "authenticated_named",
            Self::AuthenticatedAttested => "authenticated_attested",
            Self::Unauthorized => "unauthorized",
            Self::ReadOnly => "read_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Top-level SPARQL operation class, without recording query text.
pub enum QueryShape {
    /// SPARQL SELECT.
    Select,
    /// SPARQL ASK.
    Ask,
    /// SPARQL CONSTRUCT.
    Construct,
    /// SPARQL DESCRIBE.
    Describe,
    /// Text was present but no supported operation token was found.
    Unknown,
    /// The JSON request contained no string `query` field.
    Missing,
}

impl QueryShape {
    /// Stable, low-cardinality value written to the structured log.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Ask => "ask",
            Self::Construct => "construct",
            Self::Describe => "describe",
            Self::Unknown => "unknown",
            Self::Missing => "missing",
        }
    }
}

/// Metadata a handler attaches to its response for the outer log middleware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestUsage {
    /// Classified SPARQL operation.
    pub query_shape: QueryShape,
    /// Number of returned rows, triples, or ASK result values.
    pub result_size: usize,
}

/// Classify a SPARQL query by its first operation keyword.
#[must_use]
pub fn classify_query(query: Option<&str>) -> QueryShape {
    let Some(query) = query else {
        return QueryShape::Missing;
    };
    // PREFIX/BASE declarations may contain words such as `select` in their
    // labels or IRIs, so strip the prologue before reading the operation token.
    let mut rest = query.trim_start();
    loop {
        if rest.starts_with('#') {
            rest = rest
                .split_once('\n')
                .map_or("", |(_, tail)| tail)
                .trim_start();
            continue;
        }
        let upper = rest.to_ascii_uppercase();
        if upper.starts_with("PREFIX ") || upper.starts_with("BASE ") {
            rest = rest
                .find('>')
                .map_or("", |end| &rest[end + 1..])
                .trim_start();
            continue;
        }
        break;
    }
    let operation = rest
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match operation.as_str() {
        "select" => QueryShape::Select,
        "ask" => QueryShape::Ask,
        "construct" => QueryShape::Construct,
        "describe" => QueryShape::Describe,
        _ => QueryShape::Unknown,
    }
}

/// Extract the public result cardinality already present in Quipu's JSON shape.
#[must_use]
pub fn json_result_size(value: &Value) -> usize {
    value
        .get("count")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| value.get("rows").and_then(Value::as_array).map(Vec::len))
        .or_else(|| value.get("triples").and_then(Value::as_array).map(Vec::len))
        .or_else(|| value.get("result").and_then(Value::as_bool).map(|_| 1))
        .unwrap_or(0)
}

/// Serialize one request lifecycle event as a single JSON log line.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn structured_request_log(
    event: &str,
    request_id: u64,
    client: &str,
    task: &str,
    method: &str,
    path: &str,
    endpoint: &str,
    status: Option<u16>,
    duration_ms: Option<u128>,
    auth: AuthOutcome,
    usage: Option<RequestUsage>,
) -> String {
    let mut record = json!({
        "timestamp": crate::time::now_iso(),
        "event": event,
        "request_id": request_id,
        "client": client,
        "task": task,
        "method": method,
        "path": path,
        "endpoint": endpoint,
        "auth_outcome": auth.as_str(),
    });
    if let Some(status) = status {
        record["status"] = json!(status);
    }
    if let Some(duration_ms) = duration_ms {
        record["duration_ms"] = json!(duration_ms);
    }
    if let Some(usage) = usage {
        record["query_shape"] = json!(usage.query_shape.as_str());
        record["result_size"] = json!(usage.result_size);
    }
    record.to_string()
}

/// Add bounded caller-declared host/agent metadata, never authenticated identity.
/// These values belong in logs, not high-cardinality metric labels.
#[must_use]
pub fn with_declared_attribution(log: String, host: Option<&str>, agent: Option<&str>) -> String {
    let clean = |value: Option<&str>| {
        value
            .map(|s| {
                s.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c))
                    .take(64)
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
    };
    let (host, agent) = (clean(host), clean(agent));
    if host.is_none() && agent.is_none() {
        return log;
    }
    let Ok(mut record) = serde_json::from_str::<Value>(&log) else {
        return log;
    };
    if !record.is_object() {
        return log;
    }
    if let Some(host) = host {
        record["client_host"] = json!(host);
    }
    if let Some(agent) = agent {
        record["agent"] = json!(agent);
    }
    record["attribution_source"] = json!("declared_headers");
    record.to_string()
}

/// Add the receiver-observed network hop without treating it as writer identity.
/// Forwarded headers are deliberately not inputs. Internal/stdio dispatch has
/// no socket peer; a reverse proxy's peer identifies the proxy, not its client.
#[must_use]
pub fn with_request_context(
    log: String,
    peer: Option<std::net::SocketAddr>,
    provenance: Option<&crate::write_provenance::RequestProvenance>,
) -> String {
    let Ok(mut record) = serde_json::from_str::<Value>(&log) else {
        return log;
    };
    if !record.is_object() {
        return log;
    }
    record["transport_peer"] = json!(peer.map(|p| p.to_string()));
    record["transport_peer_source"] = json!(if peer.is_some() {
        "socket"
    } else {
        "unavailable"
    });
    if let Some(provenance) = provenance {
        // Request declarations include refusals; this is not commit coverage.
        record["request_provenance"] = json!(provenance.completeness.as_str());
        record["request_provenance_missing"] = json!(provenance.missing);
    }
    record.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_observation_does_not_authenticate_declared_identity() {
        let log =
            with_declared_attribution("{}".into(), Some("claimed-host"), Some("claimed-agent"));
        let peer = "127.0.0.1:12345".parse().unwrap();
        let record: Value =
            serde_json::from_str(&with_request_context(log, Some(peer), None)).unwrap();
        assert_eq!(record["client_host"], "claimed-host");
        assert_eq!(record["agent"], "claimed-agent");
        assert_eq!(record["attribution_source"], "declared_headers");
        assert_eq!(record["transport_peer"], "127.0.0.1:12345");
        assert_eq!(record["transport_peer_source"], "socket");
        assert!(record.get("authorization").is_none());
    }

    #[test]
    fn missing_transport_context_is_explicit_without_guessing_a_peer() {
        let record: Value =
            serde_json::from_str(&with_request_context("{}".into(), None, None)).unwrap();
        assert!(record["transport_peer"].is_null());
        assert_eq!(record["transport_peer_source"], "unavailable");
        assert!(record.get("agent").is_none());
        assert_eq!(with_request_context("[]".into(), None, None), "[]");
    }

    #[test]
    fn missing_field_names_do_not_expose_values_or_claim_a_commit() {
        let provenance = crate::write_provenance::RequestProvenance::classify(
            "seeds",
            "/update",
            [Some("a"), Some("codex"), Some("h"), Some("s"), None],
        );
        let record: Value =
            serde_json::from_str(&with_request_context("{}".into(), None, Some(&provenance)))
                .unwrap();
        assert_eq!(record["request_provenance"], "partial");
        assert_eq!(record["request_provenance_missing"], json!(["model"]));
        assert!(record.get("model").is_none());
        assert!(record.get("session").is_none());
        assert!(record.get("tx_id").is_none());
    }

    #[test]
    fn declared_attribution_is_bounded_optional_and_distinguished_from_identity() {
        assert_eq!(with_declared_attribution("{}".into(), None, None), "{}");
        let log = with_declared_attribution("{}".into(), Some("runner-a"), Some(&"x".repeat(200)));
        let record: Value = serde_json::from_str(&log).unwrap();
        assert_eq!(record["client_host"], "runner-a");
        assert_eq!(record["agent"].as_str().unwrap().len(), 64);
        assert_eq!(record["attribution_source"], "declared_headers");
        assert!(record.get("authorization").is_none());
        assert_eq!(
            with_declared_attribution("[]".into(), Some("runner-a"), None),
            "[]"
        );
    }

    #[test]
    fn query_shape_skips_prefix_declarations() {
        assert_eq!(
            classify_query(Some(
                "PREFIX ex: <https://example.test/> SELECT ?s WHERE { ?s ?p ?o }"
            )),
            QueryShape::Select
        );
        assert_eq!(
            classify_query(Some(
                "PREFIX select: <https://example.test/select> ASK { ?s ?p ?o }"
            )),
            QueryShape::Ask
        );
        assert_eq!(classify_query(Some("ASK { ?s ?p ?o }")), QueryShape::Ask);
        assert_eq!(classify_query(None), QueryShape::Missing);
    }

    #[test]
    fn structured_log_is_json_and_carries_bounded_usage_fields() {
        let line = structured_request_log(
            "request_complete",
            7,
            "query-first",
            "aegis-94thg",
            "POST",
            "/query",
            "/query",
            Some(200),
            Some(12),
            AuthOutcome::NotRequired,
            Some(RequestUsage {
                query_shape: QueryShape::Select,
                result_size: 3,
            }),
        );
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["client"], "query-first");
        assert_eq!(value["task"], "aegis-94thg");
        assert_eq!(value["query_shape"], "select");
        assert_eq!(value["result_size"], 3);
        assert_eq!(value["auth_outcome"], "not_required");
        assert!(value.get("authorization").is_none());
    }

    #[test]
    fn structured_log_distinguishes_bearer_generation_without_secret_material() {
        for (outcome, expected) in [
            (AuthOutcome::AuthenticatedCurrent, "authenticated_current"),
            (AuthOutcome::AuthenticatedPrevious, "authenticated_previous"),
        ] {
            let line = structured_request_log(
                "request_complete",
                8,
                "agent-adhoc",
                "aegis-ko8eck",
                "POST",
                "/episode",
                "/episode",
                Some(200),
                Some(1),
                outcome,
                None,
            );
            let value: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(value["auth_outcome"], expected);
            assert!(!line.contains("secret"));
            assert!(value.get("authorization").is_none());
        }
    }

    #[test]
    fn result_size_handles_each_public_json_shape() {
        assert_eq!(json_result_size(&json!({"count": 4, "rows": []})), 4);
        assert_eq!(json_result_size(&json!({"rows": [{}, {}]})), 2);
        assert_eq!(json_result_size(&json!({"triples": [{}, {}, {}]})), 3);
        assert_eq!(json_result_size(&json!({"result": false})), 1);
    }
}
