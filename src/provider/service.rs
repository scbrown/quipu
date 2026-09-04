//! Standards-compatible SPARQL `SERVICE` evaluation over configured remotes.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::sparql::QueryResult;
use crate::store::Store;
use crate::types::Value;

fn decode_binding(store: &Store, term: &JsonValue) -> Result<Value> {
    let kind = term
        .get("type")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::Serialization("SPARQL result binding has no string type".into()))?;
    let lexical = term
        .get("value")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::Serialization("SPARQL result binding has no string value".into()))?;
    match kind {
        "uri" => {
            let node = oxrdf::NamedNode::new(lexical)
                .map_err(|e| Error::Serialization(format!("invalid SERVICE result IRI: {e}")))?;
            Ok(Value::Ref(store.query_ref(node.as_str())?))
        }
        "bnode" => {
            let node = oxrdf::BlankNode::new(lexical).map_err(|e| {
                Error::Serialization(format!("invalid SERVICE result blank node: {e}"))
            })?;
            Ok(Value::Ref(store.query_ref(&format!(
                "{}{}",
                crate::rdf::BLANK_PREFIX,
                node.as_str()
            ))?))
        }
        "literal" | "typed-literal" => {
            let literal = if let Some(lang) = term.get("xml:lang").and_then(JsonValue::as_str) {
                oxrdf::Literal::new_language_tagged_literal(lexical, lang).map_err(|e| {
                    Error::Serialization(format!("invalid SERVICE result language tag: {e}"))
                })?
            } else if let Some(datatype) = term.get("datatype").and_then(JsonValue::as_str) {
                let datatype = oxrdf::NamedNode::new(datatype).map_err(|e| {
                    Error::Serialization(format!("invalid SERVICE result datatype IRI: {e}"))
                })?;
                oxrdf::Literal::new_typed_literal(lexical, datatype)
            } else {
                oxrdf::Literal::new_simple_literal(lexical)
            };
            crate::rdf::term_to_value(store, &oxrdf::Term::Literal(literal))
        }
        other => Err(Error::Serialization(format!(
            "unsupported SPARQL result binding type '{other}'"
        ))),
    }
}

/// Evaluate a standards SPARQL `SERVICE` pattern through one configured remote.
///
/// The endpoint is never fetched directly. It must exactly match a locally
/// configured remote base URL or its `/query` endpoint, so query text cannot
/// turn Quipu into an SSRF proxy. Configuration also supplies authentication,
/// timeout and the provenance label stamped onto returned rows.
pub fn query_configured_service(
    store: &Store,
    federation: &crate::config::FederationConfig,
    endpoint: &str,
    sparql: &str,
    deadline_ms: Option<u64>,
) -> Result<(QueryResult, String, super::DeclaredLabel)> {
    let requested = endpoint.trim_end_matches('/');
    let remote = federation
        .remotes
        .iter()
        .find(|r| {
            let base = r.url.trim_end_matches('/');
            requested == base || requested == format!("{base}/query")
        })
        .ok_or_else(|| {
            Error::InvalidValue(format!(
                "SERVICE endpoint '{endpoint}' is not in [[quipu.federation.remotes]]"
            ))
        })?;
    let label = remote.declared_label()?;
    let configured_timeout = remote.timeout_ms.unwrap_or(5000);
    let effective_timeout = deadline_ms.map_or(configured_timeout, |remaining| {
        configured_timeout.min(remaining.max(1))
    });
    let mut request = ureq::post(requested)
        .set("Content-Type", "application/sparql-query")
        .set("Accept", "application/sparql-results+json")
        .timeout(std::time::Duration::from_millis(effective_timeout));
    if let Some(token) = &remote.auth_token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let started = std::time::Instant::now();
    let response = request.send_string(sparql).map_err(|e| {
        let elapsed = started.elapsed();
        if elapsed >= std::time::Duration::from_millis(effective_timeout) {
            Error::QueryTimeout {
                elapsed_ms: elapsed.as_millis(),
                limit_ms: u128::from(effective_timeout),
            }
        } else {
            Error::InvalidValue(format!(
                "SERVICE remote '{}' at {requested}: {e}",
                remote.name
            ))
        }
    })?;
    let text = response.into_string().map_err(|e| {
        Error::Serialization(format!(
            "SERVICE remote '{}': unreadable response body: {e}",
            remote.name
        ))
    })?;
    let body: JsonValue = serde_json::from_str(&text).map_err(|e| {
        Error::Serialization(format!(
            "SERVICE remote '{}': malformed SPARQL Results JSON: {e}",
            remote.name
        ))
    })?;
    let variables = body
        .pointer("/head/vars")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| Error::Serialization("SPARQL Results JSON has no head.vars".into()))?
        .iter()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect();
    let rows = body
        .pointer("/results/bindings")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| Error::Serialization("SPARQL Results JSON has no results.bindings".into()))?
        .iter()
        .filter_map(JsonValue::as_object)
        .map(|row| {
            row.iter()
                .map(|(name, term)| decode_binding(store, term).map(|value| (name.clone(), value)))
                .collect::<Result<crate::sparql::Bindings>>()
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        QueryResult::Select { variables, rows },
        remote.name.clone(),
        label,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    const DEAD: &str = "http://127.0.0.1:1";

    /// One-shot mock remote: accepts a single connection, captures the request
    /// head, answers with `body`, and hands the captured request back — the
    /// no-dependency mock the federation design's test plan calls for.
    fn serve_once(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = sock.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&bytes);
                let Some(head_end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let content_len = text[..head_end]
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= head_end + 4 + content_len {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&bytes).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).unwrap();
            req
        });
        (format!("http://{addr}"), handle)
    }

    fn service_input(endpoint: &str) -> serde_json::Value {
        serde_json::json!({
            "query": format!(
                "SELECT ?s WHERE {{ SERVICE <{endpoint}/query> {{ ?s <http://example.org/name> ?n }} }}"
            )
        })
    }

    #[test]
    fn standard_service_uses_only_configured_remote_and_stamps_provenance() {
        let (url, server) = serve_once(
            r#"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/a"}}]}}"#,
        );
        let store = Store::open_in_memory().unwrap();
        let cfg = crate::config::FederationConfig {
            remotes: vec![crate::config::RemoteEndpoint {
                freshness: Some("fresh".into()),
                ..crate::config::RemoteEndpoint::new("partner", &url)
            }],
        };
        let (result, truncated) = crate::mcp::query_result_with_federation(
            &store,
            &service_input(&url),
            Some(std::sync::Arc::new(cfg)),
        )
        .unwrap();
        let QueryResult::Select { variables, rows } = result else {
            panic!("SERVICE SELECT must remain SELECT")
        };
        assert!(!truncated);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["_provider"], Value::Str("partner".into()));
        assert_eq!(rows[0]["_freshness"], Value::Str("fresh".into()));
        assert!(variables.contains(&"_provider".into()));
        let Value::Ref(subject) = rows[0]["s"] else {
            panic!("SERVICE URI binding must remain an RDF reference")
        };
        assert_eq!(store.resolve(subject).unwrap(), "http://example.org/a");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /query "), "{request}");
        assert!(
            request.contains("Content-Type: application/sparql-query"),
            "{request}"
        );
        assert!(
            request.contains("Accept: application/sparql-results+json"),
            "{request}"
        );
        assert!(request.contains("SELECT * WHERE"), "{request}");
    }

    #[test]
    fn service_decoder_preserves_rdf_term_kinds() {
        let store = Store::open_in_memory().unwrap();
        let row = serde_json::json!({
            "iri": {"type": "uri", "value": "http://example.org/resource"},
            "blank": {"type": "bnode", "value": "result-node"},
            "plain": {"type": "literal", "value": "plain"},
            "language": {"type": "literal", "xml:lang": "en-GB", "value": "colour"},
            "typed": {"type": "literal", "datatype": "http://www.w3.org/2001/XMLSchema#date", "value": "2026-09-03"}
        });
        let decoded = row
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, term)| decode_binding(&store, term).map(|value| (name.clone(), value)))
            .collect::<Result<crate::sparql::Bindings>>()
            .unwrap();

        let Value::Ref(iri) = decoded["iri"] else {
            panic!("URI must decode to Ref")
        };
        assert_eq!(store.resolve(iri).unwrap(), "http://example.org/resource");
        let Value::Ref(blank) = decoded["blank"] else {
            panic!("blank node must decode to Ref")
        };
        assert_eq!(store.resolve(blank).unwrap(), "_:result-node");
        assert_eq!(decoded["plain"], Value::Str("plain".into()));
        assert_eq!(
            decoded["language"],
            Value::Lang {
                lexical: "colour".into(),
                lang: "en-gb".into()
            }
        );
        assert_eq!(
            decoded["typed"],
            Value::Typed {
                lexical: "2026-09-03".into(),
                datatype: "http://www.w3.org/2001/XMLSchema#date".into()
            }
        );
    }

    #[test]
    fn service_uri_decoder_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.db");
        drop(Store::open(path.to_str().unwrap()).unwrap());
        let store = Store::open_read_only(path.to_str().unwrap()).unwrap();
        let value = decode_binding(
            &store,
            &serde_json::json!({"type": "uri", "value": "http://example.org/remote-only"}),
        )
        .unwrap();
        let Value::Ref(id) = value else {
            panic!("URI must decode to Ref")
        };
        assert_eq!(store.resolve(id).unwrap(), "http://example.org/remote-only");
    }

    #[test]
    fn service_disallowed_destination_fails_closed_without_fetching() {
        let store = Store::open_in_memory().unwrap();
        let err = crate::mcp::query_result_with_federation(
            &store,
            &service_input("http://127.0.0.1:9"),
            Some(std::sync::Arc::new(
                crate::config::FederationConfig::default(),
            )),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("not in [[quipu.federation.remotes]]")
        );
    }

    #[test]
    fn service_timeout_is_enforced_by_the_configured_remote() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            std::thread::sleep(std::time::Duration::from_millis(150));
            let body = r#"{"variables":[],"rows":[]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(response.as_bytes());
        });
        let cfg = crate::config::FederationConfig {
            remotes: vec![crate::config::RemoteEndpoint {
                timeout_ms: Some(5000),
                ..crate::config::RemoteEndpoint::new("slow", &url)
            }],
        };
        let started = std::time::Instant::now();
        let mut store = Store::open_in_memory().unwrap();
        store.search_config_mut().query_timeout_ms = 20;
        let err = crate::mcp::query_result_with_federation(
            &store,
            &service_input(&url),
            Some(std::sync::Arc::new(cfg)),
        )
        .unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_millis(120));
        assert!(err.to_string().contains("query timeout"), "{err}");
        server.join().unwrap();
    }

    #[test]
    fn service_results_obey_the_service_row_ceiling() {
        let (url, server) = serve_once(
            r#"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"literal","value":"a"}},{"s":{"type":"literal","value":"b"}}]}}"#,
        );
        let mut store = Store::open_in_memory().unwrap();
        store.search_config_mut().max_sparql_rows = 1;
        let cfg = crate::config::FederationConfig {
            remotes: vec![crate::config::RemoteEndpoint::new("bounded", &url)],
        };
        let (result, truncated) = crate::mcp::query_result_with_federation(
            &store,
            &service_input(&url),
            Some(std::sync::Arc::new(cfg)),
        )
        .unwrap();
        assert!(truncated);
        assert_eq!(result.rows().len(), 1);
        server.join().unwrap();
    }

    #[test]
    fn installing_service_remotes_does_not_change_local_only_queries() {
        let mut store = Store::open_in_memory().unwrap();
        crate::mcp::tool_knot(
            &mut store,
            &serde_json::json!({"turtle": "<urn:a> <urn:p> <urn:o> ."}),
        )
        .unwrap();
        let input = serde_json::json!({"query": "SELECT ?s WHERE { ?s <urn:p> <urn:o> }"});
        let plain = crate::mcp::query_result(&store, &input).unwrap().0;
        let configured = crate::mcp::query_result_with_federation(
            &store,
            &input,
            Some(std::sync::Arc::new(crate::config::FederationConfig {
                remotes: vec![crate::config::RemoteEndpoint::new("dead", DEAD)],
            })),
        )
        .unwrap()
        .0;
        assert_eq!(plain.rows(), configured.rows());
        assert_eq!(plain.variables(), configured.variables());
    }
}
