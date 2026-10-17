//! Explicit, expiring operator diagnostics; never enabled by default.
use serde::Deserialize;

/// One private query specimen. Raw SPARQL may contain sensitive literals.
/// No request can enable this configuration or choose its output directory.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryCaptureConfig {
    /// Explicit enablement; false performs no capture initialization.
    pub enabled: bool,
    /// Lowercase SHA-256 of the exact UTF-8 SPARQL text to match.
    pub query_sha256: String,
    /// Exact UTF-8 byte length, at most 4096.
    pub query_bytes: usize,
    /// Unique operator nonce, never reused after a restart or failed attempt.
    pub window_id: String,
    /// Inclusive UTC Unix second for the capture window.
    pub start_epoch_secs: u64,
    /// Exclusive UTC Unix second; at most 300 seconds after start.
    pub end_epoch_secs: u64,
}
