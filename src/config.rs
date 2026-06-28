//! Configuration for Quipu — loaded from `.bobbin/config.toml` `[quipu]` section.
//!
//! Config resolution order:
//! 1. CLI flags (highest priority)
//! 2. `.bobbin/config.toml` in current directory
//! 3. `~/.config/bobbin/config.toml`
//! 4. Built-in defaults

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::namespace;

/// The single source of truth for how aggressively search layers oversample
/// candidates before post-filtering. Previously scattered as inline `*10`,
/// `*5`, `*3` literals across the search/graphiti/vector paths (hq-gkd).
pub const DEFAULT_OVERSAMPLE_FACTOR: usize = 10;

/// Search/limit guardrails (hq-gkd). Without these, callers could pass
/// `limit: 1_000_000` and unbounded SPARQL could scan the whole fact log.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Result count used when a caller omits a limit (default: 10).
    pub default_limit: usize,

    /// Hard ceiling a caller-supplied limit is clamped to (default: 1000).
    pub max_limit: usize,

    /// Multiplier for how many candidates to fetch before post-filtering
    /// (default: `DEFAULT_OVERSAMPLE_FACTOR`).
    pub oversample_factor: usize,

    /// Server-side ceiling on rows returned by a SPARQL query, bounding
    /// unbounded (LIMIT-less) scans (default: 10000).
    pub max_sparql_rows: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 10,
            max_limit: 1000,
            oversample_factor: DEFAULT_OVERSAMPLE_FACTOR,
            max_sparql_rows: 10_000,
        }
    }
}

impl SearchConfig {
    /// Resolve a caller-supplied limit: fall back to `default_limit` when
    /// absent, clamp to `max_limit`, and never return 0.
    pub fn clamp_limit(&self, requested: Option<u64>) -> usize {
        let v = requested.map_or(self.default_limit, |v| v as usize);
        v.min(self.max_limit).max(1)
    }

    /// Number of candidates to fetch before post-filtering for a target result
    /// count, using the unified oversample factor (saturating).
    pub fn oversample(&self, limit: usize) -> usize {
        limit.saturating_mul(self.oversample_factor).max(limit)
    }
}

/// Vector storage backend selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VectorBackend {
    /// `SQLite`-backed vectors (default, brute-force cosine similarity).
    #[default]
    Sqlite,
    /// `LanceDB`-backed vectors (ANN search, FTS, predicate pushdown).
    #[serde(alias = "lance")]
    Lancedb,
}

/// Entity resolution configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ResolutionConfig {
    /// Whether entity resolution is enabled (default: false).
    pub enabled: bool,

    /// Similarity threshold (0.0 to 1.0) for candidate matches (default: 0.85).
    pub threshold: f64,

    /// Maximum number of candidates to return (default: 3).
    pub top_k: usize,

    /// When true, reject writes with near-duplicate candidates unless the
    /// entity is explicitly marked with `quipu:distinctFrom` (default: false).
    pub strict_mode: bool,
}

impl Default for ResolutionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.85,
            top_k: 3,
            strict_mode: false,
        }
    }
}

/// Vector storage backend configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VectorConfig {
    /// Which backend to use for vector storage (default: sqlite).
    pub backend: VectorBackend,

    /// Path to the `LanceDB` database directory (default: `.bobbin/quipu/quipu-vectors`).
    pub lancedb_path: PathBuf,
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            backend: VectorBackend::Sqlite,
            lancedb_path: PathBuf::from(".bobbin/quipu/quipu-vectors"),
        }
    }
}

/// Top-level config file structure — we only care about the `[quipu]` section.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    quipu: QuipuConfig,
}

/// Quipu configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QuipuConfig {
    /// Path to the triple store database (default: `.bobbin/quipu/quipu.db`).
    pub store_path: PathBuf,

    /// Path to directory containing OWL/SHACL schema files.
    pub schema_path: Option<PathBuf>,

    /// Base namespace URI for ontology entities (default: `DEFAULT_BASE_NS`).
    pub base_ns: String,

    /// REST API server configuration.
    pub server: ServerConfig,

    /// Federation configuration for remote Quipu instances.
    pub federation: FederationConfig,

    /// Embedding configuration for auto-embedding on write.
    pub embedding: EmbeddingConfig,

    /// Vector storage backend configuration.
    pub vector: VectorConfig,

    /// Entity resolution configuration.
    pub resolution: ResolutionConfig,

    /// Search/limit guardrails.
    pub search: SearchConfig,
}

impl Default for QuipuConfig {
    fn default() -> Self {
        Self {
            store_path: PathBuf::from(".bobbin/quipu/quipu.db"),
            schema_path: None,
            base_ns: namespace::DEFAULT_BASE_NS.to_string(),
            server: ServerConfig::default(),
            federation: FederationConfig::default(),
            embedding: EmbeddingConfig::default(),
            vector: VectorConfig::default(),
            resolution: ResolutionConfig::default(),
            search: SearchConfig::default(),
        }
    }
}

/// REST API server configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Whether to start the REST server (default: false).
    pub enabled: bool,

    /// Bind address (default: `127.0.0.1:3030`).
    pub bind: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1:3030".to_string(),
        }
    }
}

/// Federation configuration for connecting to remote Quipu instances.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FederationConfig {
    /// List of remote Quipu endpoints.
    pub remotes: Vec<RemoteEndpoint>,
}

/// Embedding configuration for auto-embedding on write.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Whether to auto-embed entities after writes (default: false).
    pub auto_embed: bool,

    /// Number of entities to embed in each batch (default: 32).
    pub embed_batch_size: usize,

    /// Path to the ONNX model file (e.g. all-MiniLM-L6-v2/onnx/model.onnx).
    pub model_path: Option<PathBuf>,

    /// Path to the tokenizer.json file (same directory as model typically).
    pub tokenizer_path: Option<PathBuf>,

    /// Embedding dimension (default: 384 for all-MiniLM-L6-v2).
    pub dimension: usize,

    /// Maximum input tokens fed to the model; longer inputs are truncated
    /// (default: 256). Caps the tensor size so an oversized `episode_body`
    /// can't blow up the sequence length or degrade embeddings (hq-7v0).
    pub max_sequence_length: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            auto_embed: false,
            embed_batch_size: 32,
            model_path: None,
            tokenizer_path: None,
            dimension: 384,
            max_sequence_length: 256,
        }
    }
}

/// A remote Quipu endpoint for federation.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteEndpoint {
    /// Human-readable name for this remote.
    pub name: String,

    /// URL of the remote Quipu REST API (e.g., `http://quipu.svc:3030`).
    pub url: String,
}

impl QuipuConfig {
    /// Load configuration, searching standard locations.
    ///
    /// Resolution: `.bobbin/config.toml` in `project_dir`, then `~/.config/bobbin/config.toml`.
    /// Returns defaults if no config file is found.
    pub fn load(project_dir: &Path) -> Self {
        // Try project-local config first.
        let local_path = project_dir.join(".bobbin/config.toml");
        if let Some(cfg) = Self::load_from(&local_path) {
            return cfg;
        }

        // Try user-level config.
        if let Some(home) = std::env::var_os("HOME") {
            let user_path = PathBuf::from(home).join(".config/bobbin/config.toml");
            if let Some(cfg) = Self::load_from(&user_path) {
                return cfg;
            }
        }

        Self::default()
    }

    /// Load from a specific TOML file. Returns `None` if file doesn't exist or has no `[quipu]` section.
    fn load_from(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let file: ConfigFile = toml::from_str(&content).ok()?;
        Some(file.quipu)
    }

    /// Apply CLI overrides: if a flag was provided, it takes precedence over config.
    pub fn with_db_override(mut self, db: Option<&str>) -> Self {
        if let Some(db) = db {
            self.store_path = PathBuf::from(db);
        }
        self
    }

    /// Apply bind address override from CLI flag.
    pub fn with_bind_override(mut self, bind: Option<&str>) -> Self {
        if let Some(bind) = bind {
            self.server.bind = bind.to_string();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_clamp_limit() {
        let cfg = SearchConfig::default(); // default_limit=10, max_limit=1000
        // Absent → default.
        assert_eq!(cfg.clamp_limit(None), 10);
        // In range → unchanged.
        assert_eq!(cfg.clamp_limit(Some(50)), 50);
        // Over the ceiling → clamped (the 1_000_000 attack).
        assert_eq!(cfg.clamp_limit(Some(1_000_000)), 1000);
        // Zero → never returns an empty page.
        assert_eq!(cfg.clamp_limit(Some(0)), 1);
    }

    #[test]
    fn search_oversample_uses_unified_factor() {
        let cfg = SearchConfig::default(); // factor = DEFAULT_OVERSAMPLE_FACTOR (10)
        assert_eq!(cfg.oversample(10), 10 * DEFAULT_OVERSAMPLE_FACTOR);
        // Saturating + never below the input.
        assert_eq!(cfg.oversample(usize::MAX), usize::MAX);
    }

    #[test]
    fn search_defaults() {
        let cfg = SearchConfig::default();
        assert_eq!(cfg.default_limit, 10);
        assert_eq!(cfg.max_limit, 1000);
        assert_eq!(cfg.oversample_factor, DEFAULT_OVERSAMPLE_FACTOR);
        assert_eq!(cfg.max_sparql_rows, 10_000);
    }

    #[test]
    fn defaults() {
        let cfg = QuipuConfig::default();
        assert_eq!(cfg.store_path, PathBuf::from(".bobbin/quipu/quipu.db"));
        assert_eq!(cfg.server.bind, "127.0.0.1:3030");
        assert!(!cfg.server.enabled);
        assert!(cfg.federation.remotes.is_empty());
        assert!(!cfg.embedding.auto_embed);
        assert_eq!(cfg.embedding.embed_batch_size, 32);
        assert_eq!(cfg.vector.backend, VectorBackend::Sqlite);
        assert_eq!(
            cfg.vector.lancedb_path,
            PathBuf::from(".bobbin/quipu/quipu-vectors")
        );
    }

    #[test]
    fn parse_toml() {
        let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"
schema_path = "/schemas"

[quipu.server]
enabled = true
bind = "0.0.0.0:8080"

[[quipu.federation.remotes]]
name = "prod"
url = "http://quipu.svc:3030"

[quipu.embedding]
auto_embed = true
embed_batch_size = 64
"#;
        let file: ConfigFile = toml::from_str(toml_str).unwrap();
        let cfg = file.quipu;
        assert_eq!(cfg.store_path, PathBuf::from("/data/quipu.db"));
        assert_eq!(cfg.schema_path, Some(PathBuf::from("/schemas")));
        assert!(cfg.server.enabled);
        assert_eq!(cfg.server.bind, "0.0.0.0:8080");
        assert_eq!(cfg.federation.remotes.len(), 1);
        assert_eq!(cfg.federation.remotes[0].name, "prod");
        assert_eq!(cfg.federation.remotes[0].url, "http://quipu.svc:3030");
        assert!(cfg.embedding.auto_embed);
        assert_eq!(cfg.embedding.embed_batch_size, 64);
    }

    #[test]
    fn parse_vector_config() {
        let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"

[quipu.vector]
backend = "lancedb"
lancedb_path = "/data/vectors"
"#;
        let file: ConfigFile = toml::from_str(toml_str).unwrap();
        let cfg = file.quipu;
        assert_eq!(cfg.vector.backend, VectorBackend::Lancedb);
        assert_eq!(cfg.vector.lancedb_path, PathBuf::from("/data/vectors"));
    }

    #[test]
    fn parse_vector_config_lance_alias() {
        let toml_str = r#"
[quipu.vector]
backend = "lance"
"#;
        let file: ConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.quipu.vector.backend, VectorBackend::Lancedb);
    }

    #[test]
    fn cli_overrides() {
        let cfg = QuipuConfig::default()
            .with_db_override(Some("/custom/path.db"))
            .with_bind_override(Some("0.0.0.0:9090"));
        assert_eq!(cfg.store_path, PathBuf::from("/custom/path.db"));
        assert_eq!(cfg.server.bind, "0.0.0.0:9090");
    }

    #[test]
    fn partial_toml() {
        let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"
"#;
        let file: ConfigFile = toml::from_str(toml_str).unwrap();
        let cfg = file.quipu;
        assert_eq!(cfg.store_path, PathBuf::from("/data/quipu.db"));
        // Server and federation should have defaults.
        assert_eq!(cfg.server.bind, "127.0.0.1:3030");
        assert!(cfg.federation.remotes.is_empty());
    }

    #[test]
    fn empty_file_gives_defaults() {
        let toml_str = "";
        let file: ConfigFile = toml::from_str(toml_str).unwrap();
        let cfg = file.quipu;
        assert_eq!(cfg.store_path, PathBuf::from(".bobbin/quipu/quipu.db"));
    }

    #[test]
    fn load_nonexistent_dir() {
        let cfg = QuipuConfig::load(Path::new("/nonexistent/dir"));
        assert_eq!(cfg.store_path, PathBuf::from(".bobbin/quipu/quipu.db"));
    }
}
