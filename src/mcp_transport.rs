//! Native MCP transport. Every call enters the existing REST application, so
//! authentication, read-only guards, and transaction identity have one authority.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request},
};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt as _;

/// MCP dispatch paired with an already guarded REST router.
#[derive(Clone)]
pub struct McpServer {
    app: Router,
    tools: Arc<Vec<Tool>>,
    stdio_headers: HeaderMap,
}

/// Resolve only a name from the published manifest, never an arbitrary route.
fn endpoint(name: &str) -> (Method, String) {
    let path = match name {
        "quipu_graph_list" => return (Method::GET, "/graphs".into()),
        "quipu_load_ontology" => "/ontology",
        "quipu_resolve_entity" => "/resolve",
        "quipu_propose_schema_change" => "/propose",
        "quipu_list_proposals" => "/proposals",
        "quipu_accept_proposal" => "/proposal/accept",
        "quipu_reject_proposal" => "/proposal/reject",
        "quipu_retract_episode" => "/episode/retract",
        "quipu_retract_source" => "/retract/source",
        "quipu_episodes_complete" => "/episodes/complete",
        "quipu_path_cone" => "/path/cone",
        "quipu_path_backtest" => "/path/backtest",
        "quipu_align_propose" => "/align/propose",
        "quipu_align_decide" => "/align/decide",
        "quipu_align_apply" => "/align/apply",
        "quipu_policy_check" => "/policy/check",
        "quipu_verdict_verify" => "/verdict/verify",
        "quipu_verifier_authorized" => "/verifier/authorized",
        "quipu_overlay_create" => "/overlay/create",
        "quipu_overlay_write" => "/overlay/write",
        "quipu_overlay_compose" => "/overlay/compose",
        "quipu_graph_freeze" => "/graph/freeze",
        "quipu_graph_thaw" => "/graph/thaw",
        _ => {
            return (
                Method::POST,
                format!("/{}", name.trim_start_matches("quipu_")),
            );
        }
    };
    (Method::POST, path.into())
}

impl McpServer {
    /// Build tools from the same schema manifest used by library consumers.
    pub fn new(app: Router) -> Self {
        let tools = crate::tool_definitions()
            .into_iter()
            .map(|mut definition| {
                let (method, path) = endpoint(definition["name"].as_str().expect("manifest name"));
                let write = crate::http_auth::is_write_request(&path, method.as_str());
                definition["annotations"] = serde_json::json!({
                    "readOnlyHint": !write, "destructiveHint": write,
                    "openWorldHint": true
                });
                serde_json::from_value(definition).expect("MCP tool schema")
            })
            .collect();
        Self {
            app,
            tools: Arc::new(tools),
            stdio_headers: HeaderMap::new(),
        }
    }

    async fn dispatch(
        &self,
        name: &str,
        input: Value,
        headers: &HeaderMap,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.tools.iter().any(|tool| tool.name == name) {
            return Err(ErrorData::invalid_params("unknown Quipu tool", None));
        }
        let (method, mut path) = endpoint(name);
        if method == Method::GET {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in input.as_object().into_iter().flatten() {
                let value = value.as_str().ok_or_else(|| {
                    ErrorData::invalid_params("graph filters must be strings", None)
                })?;
                query.append_pair(key, value);
            }
            path.push('?');
            path.push_str(&query.finish());
        }
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("x-quipu-client", "native-mcp");
        for key in ["authorization", "x-quipu-task"] {
            if let Some(value) = headers.get(key) {
                builder = builder.header(key, value);
            }
        }
        let request = builder
            .body(Body::from(input.to_string()))
            .map_err(|_| ErrorData::internal_error("cannot construct tool request", None))?;
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .map_err(|_| ErrorData::internal_error("tool dispatch failed", None))?;
        let success = response.status().is_success();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
            .await
            .map_err(|_| ErrorData::internal_error("tool response exceeds limit", None))?;
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| ErrorData::internal_error("tool response is not text", None))?;
        Ok(if success {
            CallToolResult::success(vec![Content::text(text)])
        } else {
            CallToolResult::error(vec![Content::text(text)])
        })
    }

    /// Serve local stdio without binding a network listener. An optional bearer
    /// is read from a file; it is never accepted as an argument value.
    pub async fn stdio(
        mut self,
        token_file: Option<&std::path::Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(path) = token_file {
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() > 4096 {
                return Err("invalid MCP token file".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err("MCP token file must be private (0600 or 0400)".into());
                }
            }
            use std::io::Read;
            let mut token = String::new();
            file.take(4097).read_to_string(&mut token)?;
            if token.len() > 4096 {
                return Err("MCP token file too large".into());
            }
            if token.trim().is_empty() {
                return Err("empty MCP token file".into());
            }
            let header = format!("Bearer {}", token.trim())
                .parse()
                .map_err(|_| "invalid MCP credential")?;
            self.stdio_headers.insert("authorization", header);
        }
        self.serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
        Ok(())
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "quipu".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Quipu knowledge graph. Writes follow the server's bearer and read-only policy."
                    .into(),
            ),
            ..Default::default()
        }
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: self.tools.as_ref().clone(),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // Each HTTP request supplies its OWN Parts, including after reconnects.
        // Never take a credential from initialize or a caller-selected session ID.
        let headers = context
            .extensions
            .get::<axum::http::request::Parts>()
            .map_or(&self.stdio_headers, |parts| &parts.headers);
        self.dispatch(
            &request.name,
            Value::Object(request.arguments.unwrap_or_default()),
            headers,
        )
        .await
    }
}

/// Mount stateless HTTP: restarts cannot orphan MCP sessions. Browser requests
/// must carry an explicitly allowed Origin; absent Origin is normal for agents.
pub fn http_router(app: Router, allowed_origins: Vec<String>) -> Router {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    let service: StreamableHttpService<McpServer, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(McpServer::new(app.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig {
            stateful_mode: false,
            ..Default::default()
        },
    );
    let origins: Vec<axum::http::HeaderValue> = allowed_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
            axum::http::HeaderName::from_static("mcp-protocol-version"),
            axum::http::HeaderName::from_static("mcp-session-id"),
        ]);
    Router::new()
        .nest_service("/mcp", service)
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            64 * 1024 * 1024,
        ))
        .layer(cors)
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let allowed = allowed_origins.clone();
                async move {
                    if let Some(origin) = request.headers().get("origin")
                        && !origin
                            .to_str()
                            .ok()
                            .is_some_and(|v| allowed.iter().any(|a| a == v))
                    {
                        return axum::http::StatusCode::FORBIDDEN.into_response();
                    }
                    next.run(request).await
                }
            },
        ))
}
use axum::response::IntoResponse;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_tool_has_a_classified_registered_route() {
        let source = include_str!("server.rs");
        for tool in crate::tool_definitions() {
            let name = tool["name"].as_str().unwrap();
            let (_, path) = endpoint(name);
            assert!(
                crate::http_auth::WRITE_ENDPOINTS.contains(&path.as_str())
                    || crate::http_auth::READ_ENDPOINTS.contains(&path.as_str()),
                "{name}: {path}"
            );
            // Alignment routes live in their own router module.
            assert!(
                source.contains(&format!("\"{path}\""))
                    || include_str!("server/align.rs").contains(&format!("\"{path}\""))
                    || include_str!("server/snapshot_upload.rs").contains(&format!("\"{path}\"")),
                "{name}: {path}"
            );
        }
    }
}
