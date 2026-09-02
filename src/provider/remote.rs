//! `RemoteProvider` — a remote quipu-server as a federation member (quipu #47).
//!
//! Split from `provider/mod.rs` so the `remote`-gated HTTP half lives apart
//! from the trait and the local/federated providers (quipu-tkh).

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::sparql::QueryResult;
use crate::store::Store;
use crate::types::Value;

use super::{FederatedProvider, GraphProvider, LocalProvider, ProviderStatus};

/// A remote Quipu instance, reached over its REST API (quipu #47).
///
/// Behind the `remote` feature: this is the only `ureq` user in the crate, and
/// `ureq` is a blocking-socket HTTP client that does not build for wasm32.
///
/// This is the half of federation that `docs/book/src/architecture/federation.md`
/// documented and nothing implemented: `FederatedProvider` could only aggregate
/// LOCAL providers, so the headline — one query across a local store and remote
/// instances — was unfulfilled, and `[[quipu.federation.remotes]]` was parsed
/// and consumed nowhere.
///
/// ## What a remote row can and cannot carry
///
/// `value_to_json` renders `Value::Ref(id)` as a **bare IRI string**, which is
/// indistinguishable on the wire from a `Value::Str` holding the same text. A
/// term id is local to the store that minted it, so it could not travel anyway.
///
/// Remote values therefore arrive as `Value::Str`, and that is a real semantic
/// difference rather than an implementation shortcut: a federated result can
/// tell you an IRI, not that the producer held it as a reference. Documented
/// here because the alternative — interning remote IRIs into the local store to
/// fabricate `Ref`s — would silently grow the local term table on every read.
///
/// `Lang` and `Typed` DO survive, because they serialize as objects.
#[cfg(feature = "remote")]
pub struct RemoteProvider {
    name: String,
    url: String,
    timeout: std::time::Duration,
    auth_token: Option<String>,
    /// The label this remote's rows carry — DECLARED by the local operator
    /// (quipu-fd1), never read from the remote itself: a remote asserting its
    /// own trustworthiness would defeat the boundary.
    label: super::DeclaredLabel,
}

#[cfg(feature = "remote")]
impl RemoteProvider {
    /// A remote at `url` (e.g. `http://quipu.example:3030`), labelled `name`.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into().trim_end_matches('/').to_string(),
            timeout: std::time::Duration::from_secs(30),
            auth_token: None,
            label: super::DeclaredLabel::default(),
        }
    }

    /// Declare the label this remote's rows carry (the local operator's
    /// declaration from `[[quipu.federation.remotes]]`).
    #[must_use]
    pub fn with_label(mut self, label: super::DeclaredLabel) -> Self {
        self.label = label;
        self
    }

    /// Override the per-request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Authenticate to the remote: sent as `Authorization: Bearer …` on every
    /// request — for a remote running with `server.auth_token` set.
    #[must_use]
    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Attach the bearer header when a token is configured.
    fn authed(&self, req: ureq::Request) -> ureq::Request {
        match &self.auth_token {
            Some(t) => req.set("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    fn post(&self, path: &str, body: &JsonValue) -> Result<JsonValue> {
        let resp = self
            .authed(ureq::post(&format!("{}{path}", self.url)))
            .set("Content-Type", "application/json")
            .timeout(self.timeout)
            .send_string(&body.to_string())
            .map_err(|e| {
                Error::InvalidValue(format!("remote '{}' at {}{path}: {e}", self.name, self.url))
            })?;
        // `into_string` + serde_json rather than ureq's `into_json`: the crate
        // is vendored with `default-features = false`, and parsing here needs no
        // feature change to a dependency the rest of the tree already relies on.
        let text = resp.into_string().map_err(|e| {
            Error::Serialization(format!(
                "remote '{}': unreadable response body: {e}",
                self.name
            ))
        })?;
        serde_json::from_str(&text).map_err(|e| {
            Error::Serialization(format!(
                "remote '{}': malformed JSON response: {e}",
                self.name
            ))
        })
    }
}

/// Evaluate a standards SPARQL `SERVICE` pattern through one configured remote.
///
/// The endpoint is never fetched directly. It must exactly match a locally
/// configured remote base URL or its `/query` endpoint, so query text cannot
/// turn Quipu into an SSRF proxy. Configuration also supplies authentication,
/// timeout and the provenance label stamped onto returned rows.
#[cfg(feature = "remote")]
pub fn query_configured_service(
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
                .filter_map(|(name, term)| {
                    let lexical = term.get("value")?.as_str()?;
                    let value = if let Some(lang) = term.get("xml:lang").and_then(JsonValue::as_str)
                    {
                        Value::Lang {
                            lexical: lexical.into(),
                            lang: lang.into(),
                        }
                    } else if let Some(datatype) = term.get("datatype").and_then(JsonValue::as_str)
                    {
                        Value::Typed {
                            lexical: lexical.into(),
                            datatype: datatype.into(),
                        }
                    } else {
                        Value::Str(lexical.into())
                    };
                    Some((name.clone(), value))
                })
                .collect::<crate::sparql::Bindings>()
        })
        .collect();
    Ok((
        QueryResult::Select { variables, rows },
        remote.name.clone(),
        label,
    ))
}

/// Invert [`crate::mcp::value_to_json`] as far as the wire allows.
///
/// An IRI and a string literal are the same JSON string, so both become
/// [`Value::Str`] — see [`RemoteProvider`] for why that is a stated limit and
/// not a bug to fix here.
#[cfg(feature = "remote")]
fn json_to_value(v: &JsonValue) -> Value {
    match v {
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Float(n.as_f64().unwrap_or(0.0)), Value::Int),
        JsonValue::Object(o) => {
            let lexical = o.get("value").and_then(|x| x.as_str()).unwrap_or("");
            if let Some(lang) = o.get("lang").and_then(|x| x.as_str()) {
                Value::Lang {
                    lexical: lexical.to_string(),
                    lang: lang.to_string(),
                }
            } else if let Some(dt) = o.get("datatype").and_then(|x| x.as_str()) {
                Value::Typed {
                    lexical: lexical.to_string(),
                    datatype: dt.to_string(),
                }
            } else {
                Value::Str(v.to_string())
            }
        }
        JsonValue::String(s) => Value::Str(s.clone()),
        other => Value::Str(other.to_string()),
    }
}

/// Rebuild a [`QueryResult`] from a `/query` response body.
///
/// Shape-directed, matching what `tool_query` emits: `rows`+`variables` for a
/// SELECT, `result` for an ASK, `triples` for CONSTRUCT/DESCRIBE.
#[cfg(feature = "remote")]
fn parse_query_response(body: &JsonValue) -> Result<QueryResult> {
    if let Some(rows) = body.get("rows").and_then(|v| v.as_array()) {
        let variables: Vec<String> = body
            .get("variables")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let parsed = rows
            .iter()
            .filter_map(|r| r.as_object())
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), json_to_value(v)))
                    .collect::<crate::sparql::Bindings>()
            })
            .collect();
        return Ok(QueryResult::Select {
            variables,
            rows: parsed,
        });
    }
    if let Some(b) = body.get("result").and_then(serde_json::Value::as_bool) {
        return Ok(QueryResult::Ask(b));
    }
    if let Some(triples) = body.get("triples").and_then(|v| v.as_array()) {
        let parsed = triples
            .iter()
            .filter_map(|t| {
                Some(crate::sparql::Triple {
                    subject: t.get("subject")?.as_str()?.to_string(),
                    predicate: t.get("predicate")?.as_str()?.to_string(),
                    object: json_to_value(t.get("object")?),
                })
            })
            .collect();
        return Ok(QueryResult::Graph(parsed));
    }
    Err(Error::Serialization(
        "remote response has no 'rows', 'result' or 'triples'".into(),
    ))
}

#[cfg(feature = "remote")]
impl GraphProvider for RemoteProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn query(&self, sparql: &str) -> Result<QueryResult> {
        parse_query_response(&self.post("/query", &serde_json::json!({ "query": sparql }))?)
    }

    fn entities(&self, type_filter: Option<&str>, limit: usize) -> Result<JsonValue> {
        self.post(
            "/cord",
            &serde_json::json!({ "type": type_filter, "limit": limit }),
        )
    }

    /// Health check that **never errors**.
    ///
    /// An unreachable or slow remote must degrade to `healthy: false`, never to
    /// an `Err` — `health_all` is called to decide whether a remote is usable,
    /// and a federated query that ABORTS because one member is down is exactly
    /// the failure federation exists to avoid. The reason lands in `message`
    /// rather than being discarded.
    fn health(&self) -> ProviderStatus {
        // The declared label rides every status, healthy or not: it is the
        // operator's configuration, not a liveness observation.
        let label = self.declared_label().cloned();
        match self
            .authed(ureq::get(&format!("{}/stats", self.url)))
            .timeout(self.timeout)
            .call()
        {
            Ok(resp) => match resp
                .into_string()
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<JsonValue>(&t).map_err(|e| e.to_string()))
            {
                Ok(body) => ProviderStatus {
                    name: self.name.clone(),
                    healthy: true,
                    fact_count: body
                        .get("facts")
                        .or_else(|| body.get("fact_count"))
                        .and_then(serde_json::Value::as_u64),
                    message: None,
                    label,
                },
                Err(e) => ProviderStatus {
                    name: self.name.clone(),
                    healthy: false,
                    fact_count: None,
                    message: Some(format!("malformed /stats response: {e}")),
                    label,
                },
            },
            Err(e) => ProviderStatus {
                name: self.name.clone(),
                healthy: false,
                fact_count: None,
                message: Some(format!("unreachable: {e}")),
                label,
            },
        }
    }

    fn declared_label(&self) -> Option<&super::DeclaredLabel> {
        if self.label.is_empty() {
            None
        } else {
            Some(&self.label)
        }
    }
}

/// Build a [`FederatedProvider`] from configured remotes (quipu #47).
///
/// This is what retires `[[quipu.federation.remotes]]` as dead config: it was
/// parsed and exported and consumed by NOTHING, i.e. a settable switch wired to
/// nothing at all — the same class `config.rs`'s unwired-field guard exists to
/// catch, and it sat on that guard's allowlist until now.
///
/// The local store is added first so it leads the merged results, matching the
/// order `query_all` reports.
///
/// # Errors
/// A malformed label declaration on a remote (a partial trust triple, an
/// unparseable freshness — quipu-fd1). Refused rather than silently dropped:
/// a typo'd declaration that vanished would leave the operator believing a
/// label flows when none does.
#[cfg(feature = "remote")]
pub fn federated_from_config<'a>(
    store: &'a Store,
    local_label: &str,
    federation: &crate::config::FederationConfig,
) -> Result<FederatedProvider<'a>> {
    let mut fed = FederatedProvider::new();
    fed.add(Box::new(LocalProvider::new(store, local_label)));
    for remote in &federation.remotes {
        // 5s default (design §3): long enough for a real query, short enough
        // that one dead peer does not dominate a federated call.
        let mut p = RemoteProvider::new(&remote.name, &remote.url)
            .with_timeout(std::time::Duration::from_millis(
                remote.timeout_ms.unwrap_or(5000),
            ))
            .with_label(remote.declared_label()?);
        if let Some(token) = &remote.auth_token {
            p = p.with_auth_token(token);
        }
        fed.add(Box::new(p));
    }
    Ok(fed)
}

#[cfg(all(test, feature = "remote"))]
mod remote_tests {
    use super::*;

    /// A port nothing listens on. Connection-refused is the *cheap* unreachable
    /// case and needs no mock server.
    const DEAD: &str = "http://127.0.0.1:1";

    #[test]
    fn health_on_an_unreachable_remote_is_false_and_never_an_error() {
        // The property the whole design rests on: `health` returns a STATUS, not
        // a Result. A federated query that aborted because one member was down
        // is the failure federation exists to avoid.
        let p =
            RemoteProvider::new("dead", DEAD).with_timeout(std::time::Duration::from_millis(200));
        let status = p.health();
        assert!(!status.healthy, "unreachable must report unhealthy");
        assert_eq!(status.name, "dead");
        assert!(
            status
                .message
                .as_deref()
                .is_some_and(|m| m.contains("unreachable")),
            "the reason is carried, not discarded: {:?}",
            status.message
        );
    }

    #[test]
    fn a_query_to_an_unreachable_remote_errors_rather_than_returning_empty() {
        // The OPPOSITE choice from `health`, deliberately. An empty result set
        // from a down remote is indistinguishable from "the remote has no
        // matching data" — that is a wrong answer wearing a right answer's
        // clothes. Health degrades; a query does not.
        let p =
            RemoteProvider::new("dead", DEAD).with_timeout(std::time::Duration::from_millis(200));
        assert!(p.query("SELECT ?s WHERE { ?s ?p ?o }").is_err());
    }

    #[test]
    fn a_select_response_round_trips_into_a_query_result() {
        let body = serde_json::json!({
            "variables": ["s", "n"],
            "rows": [{"s": "http://example.org/a", "n": 42}],
            "count": 1,
        });
        let QueryResult::Select { variables, rows } = parse_query_response(&body).unwrap() else {
            panic!("expected Select");
        };
        assert_eq!(variables, vec!["s", "n"]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("n"), Some(&Value::Int(42)));
        assert_eq!(
            rows[0].get("s"),
            Some(&Value::Str("http://example.org/a".into())),
            "an IRI arrives as Str — a term id is local and cannot travel"
        );
    }

    #[test]
    fn lang_and_typed_literals_survive_the_wire_but_ref_cannot() {
        // Lang/Typed serialize as OBJECTS, so they round-trip. `Ref` serializes
        // as a bare string, so it cannot — a documented limit, asserted so it
        // stays documented.
        assert_eq!(
            json_to_value(&serde_json::json!({"value": "hello", "lang": "en"})),
            Value::Lang {
                lexical: "hello".into(),
                lang: "en".into()
            }
        );
        assert_eq!(
            json_to_value(&serde_json::json!({"value": "5", "datatype": "xsd:int"})),
            Value::Typed {
                lexical: "5".into(),
                datatype: "xsd:int".into()
            }
        );
        assert_eq!(
            json_to_value(&serde_json::json!("http://example.org/x")),
            Value::Str("http://example.org/x".into()),
            "indistinguishable from a string literal, by construction"
        );
    }

    #[test]
    fn ask_and_construct_responses_are_recognised_by_shape() {
        assert!(matches!(
            parse_query_response(&serde_json::json!({"result": true})).unwrap(),
            QueryResult::Ask(true)
        ));
        let g = parse_query_response(&serde_json::json!({
            "triples": [{"subject": "s", "predicate": "p", "object": "o"}]
        }))
        .unwrap();
        assert!(matches!(g, QueryResult::Graph(t) if t.len() == 1));
    }

    #[test]
    fn a_response_of_no_known_shape_is_refused() {
        // Never a silent empty result: a body we cannot interpret is an error,
        // not zero rows.
        assert!(parse_query_response(&serde_json::json!({"unexpected": 1})).is_err());
    }

    #[test]
    fn config_remotes_become_providers_beside_the_local_store() {
        let store = Store::open_in_memory().unwrap();
        let cfg = crate::config::FederationConfig {
            remotes: vec![
                crate::config::RemoteEndpoint::new("a", "http://one:3030"),
                crate::config::RemoteEndpoint::new("b", "http://two:3030/"),
            ],
        };
        let fed = federated_from_config(&store, "local", &cfg).unwrap();
        let names: Vec<String> = fed.health_all().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec!["local", "a", "b"],
            "local leads, then each configured remote — the dead config is retired"
        );
    }

    #[test]
    fn a_declared_label_reaches_the_status_and_a_partial_declaration_is_refused() {
        // quipu-fd1: the CONFIG declaration is wired through to the provider —
        // the status carries it even for an unreachable remote (it is the
        // operator's declaration, not a liveness observation) — and a partial
        // trust triple is refused at build time, never silently dropped.
        let store = Store::open_in_memory().unwrap();
        let mut cfg = crate::config::FederationConfig {
            remotes: vec![crate::config::RemoteEndpoint {
                trust: Some("urn:trust:partner".into()),
                trust_chain: Some("urn:chain:c".into()),
                trust_rank: Some(30),
                ..crate::config::RemoteEndpoint::new("p", DEAD)
            }],
        };
        let fed = federated_from_config(&store, "local", &cfg).unwrap();
        let statuses = fed.health_all();
        assert!(statuses[0].label.is_none(), "local: labels are per-graph");
        let declared = statuses[1].label.as_ref().expect("declared label rides");
        assert_eq!(
            declared.trust.as_ref().map(|t| (t.iri.as_str(), t.rank)),
            Some(("urn:trust:partner", 30))
        );

        cfg.remotes[0].trust_chain = None;
        let Err(err) = federated_from_config(&store, "local", &cfg) else {
            panic!("a partial trust declaration must be refused")
        };
        assert!(err.to_string().contains("trust_chain"), "{err}");
    }

    #[test]
    fn a_trailing_slash_in_the_configured_url_does_not_double_up() {
        let p = RemoteProvider::new("x", "http://host:3030/");
        assert_eq!(p.url, "http://host:3030", "normalised at construction");
    }

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

    #[test]
    fn a_configured_auth_token_reaches_the_remote_as_a_bearer_header() {
        // Through `federated_from_config`, not a hand-built provider: the test
        // proves the CONFIG field is wired, not merely that the builder works.
        let (url, server) = serve_once(r#"{"variables":["s"],"rows":[]}"#);
        let store = Store::open_in_memory().unwrap();
        let cfg = crate::config::FederationConfig {
            remotes: vec![crate::config::RemoteEndpoint {
                auth_token: Some("sekrit".into()),
                timeout_ms: Some(2000),
                ..crate::config::RemoteEndpoint::new("authed", url)
            }],
        };
        let fed = federated_from_config(&store, "local", &cfg).unwrap();
        let fq = fed.query_all("SELECT ?s WHERE { ?s ?p ?o }");
        assert!(fq.complete, "{:?}", fq.providers);
        let req = server.join().unwrap();
        assert!(
            req.contains("Authorization: Bearer sekrit"),
            "bearer header missing from request: {req}"
        );
    }
}
