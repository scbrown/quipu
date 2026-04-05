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
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            auto_embed: false,
            embed_batch_size: 32,
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
