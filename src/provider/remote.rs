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
        }
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
                },
                Err(e) => ProviderStatus {
                    name: self.name.clone(),
                    healthy: false,
                    fact_count: None,
                    message: Some(format!("malformed /stats response: {e}")),
                },
            },
            Err(e) => ProviderStatus {
                name: self.name.clone(),
                healthy: false,
                fact_count: None,
                message: Some(format!("unreachable: {e}")),
            },
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
#[cfg(feature = "remote")]
#[must_use]
pub fn federated_from_config<'a>(
    store: &'a Store,
    local_label: &str,
    federation: &crate::config::FederationConfig,
) -> FederatedProvider<'a> {
    let mut fed = FederatedProvider::new();
    fed.add(Box::new(LocalProvider::new(store, local_label)));
    for remote in &federation.remotes {
        // 5s default (design §3): long enough for a real query, short enough
        // that one dead peer does not dominate a federated call.
        let mut p = RemoteProvider::new(&remote.name, &remote.url).with_timeout(
            std::time::Duration::from_millis(remote.timeout_ms.unwrap_or(5000)),
        );
        if let Some(token) = &remote.auth_token {
            p = p.with_auth_token(token);
        }
        fed.add(Box::new(p));
    }
    fed
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
        let fed = federated_from_config(&store, "local", &cfg);
        let names: Vec<String> = fed.health_all().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec!["local", "a", "b"],
            "local leads, then each configured remote — the dead config is retired"
        );
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
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
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

    #[test]
    fn a_configured_auth_token_reaches_the_remote_as_a_bearer_header() {
        // Through `federated_from_config`, not a hand-built provider: the test
        // proves the CONFIG field is wired, not merely that the builder works.
        let (url, server) = serve_once(r#"{"variables":["s"],"rows":[]}"#);
        let store = Store::open_in_memory().unwrap();
        let cfg = crate::config::FederationConfig {
            remotes: vec![crate::config::RemoteEndpoint {
                name: "authed".into(),
                url,
                auth_token: Some("sekrit".into()),
                timeout_ms: Some(2000),
            }],
        };
        let fed = federated_from_config(&store, "local", &cfg);
        let fq = fed.query_all("SELECT ?s WHERE { ?s ?p ?o }");
        assert!(fq.complete, "{:?}", fq.providers);
        let req = server.join().unwrap();
        assert!(
            req.contains("Authorization: Bearer sekrit"),
            "bearer header missing from request: {req}"
        );
    }
}
