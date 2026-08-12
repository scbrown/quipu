//! Federation config — the `[[quipu.federation.remotes]]` table (quipu #47).
//!
//! Split from `config.rs` when quipu-tkh grew the endpoint schema; the
//! consumer is `provider::federated_from_config`.

use serde::Deserialize;

/// Federation configuration for connecting to remote Quipu instances.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FederationConfig {
    /// List of remote Quipu endpoints.
    pub remotes: Vec<RemoteEndpoint>,
}

/// A remote Quipu endpoint for federation.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteEndpoint {
    /// Human-readable name for this remote.
    pub name: String,

    /// URL of the remote Quipu REST API (e.g., `http://quipu.example:3030`).
    pub url: String,

    /// Sent as `Authorization: Bearer …` — for a remote that took the
    /// project's own advice and set `server.auth_token`. `None` is an open
    /// remote. Plaintext in config is the same posture the server already
    /// takes for its own token.
    #[serde(default)]
    pub auth_token: Option<String>,

    /// Per-request timeout in milliseconds (default: 5000 — long enough for a
    /// real query, short enough that one dead peer does not dominate a
    /// federated call).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl RemoteEndpoint {
    /// An open remote with the default timeout — the minimal config shape.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            auth_token: None,
            timeout_ms: None,
        }
    }
}
