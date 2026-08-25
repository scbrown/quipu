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

    /// The trust value this remote's rows carry, **declared by the LOCAL
    /// operator** (quipu-fd1) — the IRI of a trust value, e.g.
    /// `urn:trust:partner`. Never read from the remote itself: a remote
    /// asserting its own trustworthiness would defeat the boundary this label
    /// exists to draw (the SARC trust boundary, surfaced at the federation
    /// edge — multi-db-composition.md §5).
    ///
    /// Must be declared together with [`RemoteEndpoint::trust_chain`] and
    /// [`RemoteEndpoint::trust_rank`]; a partial declaration is refused (see
    /// `RemoteEndpoint::declared_label` in `src/provider/label.rs`).
    ///
    /// **Undeclared is undeclared**, exactly as for an unlabelled local graph:
    /// no value is fabricated, and a configured `[quipu.labels]` trust or
    /// freshness floor refuses an undeclared remote rather than reading
    /// silence as trust.
    #[serde(default)]
    pub trust: Option<String>,

    /// The chain [`RemoteEndpoint::trust`] is ranked in — required with it,
    /// because a rank means nothing outside the chain that declared it.
    #[serde(default)]
    pub trust_chain: Option<String>,

    /// The rank of [`RemoteEndpoint::trust`] within its chain.
    #[serde(default)]
    pub trust_rank: Option<i64>,

    /// The freshness this remote's rows carry, declared by the local operator
    /// (`fresh` | `recomputing` | `stale`). Optional and independent of the
    /// trust declaration, matching [`crate::store::labels::GraphLabel`].
    #[serde(default)]
    pub freshness: Option<String>,
}

impl RemoteEndpoint {
    /// An open remote with the default timeout and no declared label — the
    /// minimal config shape.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            auth_token: None,
            timeout_ms: None,
            trust: None,
            trust_chain: None,
            trust_rank: None,
            freshness: None,
        }
    }
}
