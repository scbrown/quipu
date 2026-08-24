//! Configuration for Quipu — loaded from `.bobbin/config.toml` `[quipu]` section.
//!
//! Config resolution order:
//! 1. CLI flags (highest priority)
//! 2. `.bobbin/config.toml` in current directory
//! 3. `~/.config/bobbin/config.toml`
//! 4. Built-in defaults

use std::path::PathBuf;

use serde::Deserialize;

mod federation;
pub use federation::{FederationConfig, RemoteEndpoint};

use crate::namespace;

/// The single source of truth for how aggressively search layers oversample
/// candidates before post-filtering. Previously scattered as inline `*10`,
/// `*5`, `*3` literals across the search/graphiti/vector paths (hq-gkd).
pub const DEFAULT_OVERSAMPLE_FACTOR: usize = 10;

/// Graph-label enforcement floors (quipu #68, graph-labels.md §5).
///
/// **All unset by default, and unset means zero behaviour change** — no query
/// is ever refused by a store that has not configured a floor.
///
/// ⚠️ **These are NOT access control.** A floor refuses a *query*; it does not
/// hide rows, and nothing stops a caller who names a graph directly from
/// reading it. `aegis:authorityOver` gates writes only; a read-side authority
/// check does not exist and is not built here. Presenting trust labels as a
/// confidentiality boundary would repeat the `group_id` mistake this stack
/// already documents (graph-labels.md §11).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LabelsConfig {
    /// Refuse queries whose composed dataset freshness is below this
    /// (`fresh` | `recomputing` | `stale`). Unset = no freshness floor.
    pub min_freshness: Option<String>,

    /// Refuse queries whose composed trust rank is below this.
    ///
    /// **Must be set together with [`LabelsConfig::min_trust_chain`].** A bare
    /// rank floor is exactly the category error the trust axis refuses at
    /// runtime: a rank means nothing outside the chain that declared it, so
    /// "at least 30" is unanswerable until you say *thirty in which chain*.
    /// The design writes this as a single `min_trust`; splitting it is what
    /// makes it checkable rather than a number that silently compares across
    /// vocabularies.
    pub min_trust_rank: Option<i64>,

    /// The chain [`LabelsConfig::min_trust_rank`] is expressed in.
    pub min_trust_chain: Option<String>,

    /// Refuse queries whose composed policy carries any of these obligation
    /// tokens (e.g. `no-export`). Empty = no policy floor.
    pub deny_policy_tokens: Vec<String>,

    /// Refuse queries over graphs declaring any of these data kinds (e.g.
    /// `archive`, to keep frozen graphs out of implicit reads). A BLOCKLIST,
    /// not a minimum: an undeclared kind passes. Empty = no kind floor.
    pub deny_data_kinds: Vec<String>,
}

impl LabelsConfig {
    /// Whether any floor is configured. The fast path: an unconfigured store
    /// does no label work at all on the query path.
    #[must_use]
    pub fn is_unset(&self) -> bool {
        self.min_freshness.is_none()
            && self.min_trust_rank.is_none()
            && self.deny_policy_tokens.is_empty()
            && self.deny_data_kinds.is_empty()
    }
}

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

    /// Wall-clock budget for a single SPARQL query, in milliseconds
    /// (default: 30000; 0 disables). Enforced INSIDE evaluation — a `SQLite`
    /// progress handler interrupts a grinding `sqlite3_step`, and the pattern
    /// evaluator checks the deadline between operators — so a runaway query
    /// stops burning and releases the store lock instead of holding it for its
    /// full runtime (the observed wedge: one unbound
    /// `FILTER(CONTAINS(...))` scan ground >15min while every store request
    /// serialized behind it).
    pub query_timeout_ms: u64,

    /// Ceiling on INTERMEDIATE binding rows during SPARQL evaluation
    /// (default: 1,000,000; 0 disables). The wall-clock budget alone is not
    /// enough: an exploding join burns its whole timeout at 100% CPU while
    /// holding the store lock before it aborts. This cap stops the explosion
    /// as soon as it is *recognizable* — a join or BGP accumulation whose
    /// output exceeds the cap aborts immediately with a complexity error
    /// naming the limit, usually within milliseconds of going quadratic.
    pub max_join_rows: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 10,
            max_limit: 1000,
            oversample_factor: DEFAULT_OVERSAMPLE_FACTOR,
            max_sparql_rows: 10_000,
            query_timeout_ms: 30_000,
            max_join_rows: 1_000_000,
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

/// SHACL validation policy (hq-c6s).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ShaclConfig {
    /// Validate every write against the persistently-loaded shapes (those
    /// stored via `quipu_shapes`), not just shapes carried inline on an
    /// episode. Default false — opt in to enforce "start strict" on all writes.
    pub validate_on_write: bool,
}

/// OWL write-time constraint policy (aegis-bmqup).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OwlConfig {
    /// Enforce `owl:disjointWith` and `owl:FunctionalProperty` from the
    /// persistently-loaded ontologies on every write, rejecting the transaction
    /// when a proposed batch violates one.
    ///
    /// Default FALSE, mirroring `shacl.validate_on_write` — and deliberately so
    /// rather than on-by-default. `Ontology::validate()` shipped with NO CALLER
    /// while `docs/book/src/concepts/owl.md` claimed write-time enforcement, so
    /// turning it on is a behaviour change for every existing deployment: axioms
    /// that have been accumulating unenforced would start rejecting writes the
    /// moment the flag flipped, against a population never checked against them.
    /// Load the axioms, measure the existing violations, THEN enable.
    pub validate_on_write: bool,
}

/// Governance enforcement policy (the loom, write-path gate).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GovernanceConfig {
    /// Enforce `boundary:"action"` governance policies on every write. When set,
    /// `Store::transact` evaluates the applicable policies against the pending
    /// post-state and rejects the write (`Error::PolicyDenied`) if a `deny`
    /// policy's claim is unsatisfied for a touched target. Default false — opt
    /// in, mirroring `shacl.validate_on_write`. See
    /// `docs/design/policy-edit-hooks.md`.
    pub enforce_on_write: bool,

    /// Validate SARC class↔placement conformance when a write DEFINES or
    /// AMENDS an `aegis:Policy` (`src/governance/placement.rs`). Rejects a
    /// hard constraint declared at the Post-Action Auditor, an action-boundary
    /// policy with no class, an escalation with no reversibility window, and
    /// the rest of Table 3.
    ///
    /// Independent of [`Self::enforce_on_write`], which governs *evaluation* of
    /// policies on every write; this governs *definition* of them, and runs
    /// only on governance writes. Default false — opt in, mirroring
    /// `shacl.validate_on_write`. Note the consequence of leaving it off with
    /// `enforce_on_write` on: a constraint whose class and enforcement point
    /// disagree will be accepted and then evaluated somewhere it cannot do its
    /// job, and nothing will say so.
    pub validate_placement: bool,

    /// Enforce authority intersection over named graphs on the write path
    /// (`src/governance/authority.rs`, SARC I5). When a caller has set a
    /// principal chain, a write to a graph the chain's INTERSECTED authority
    /// does not cover is refused.
    ///
    /// Default false. Note it is inert for a caller that sets no chain: an
    /// unattributed write is the shape every existing caller has, and making
    /// attribution a hard requirement beneath a running deployment would break
    /// all of them at once. The flag makes a supplied chain BINDING, so adopting
    /// attribution is per-caller and cannot silently widen.
    pub enforce_authority: bool,
}

/// Event-log retention policy (quipu-9z9).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EventsConfig {
    /// Prune events older than this many days, provided every registered
    /// consumer has committed past them (`Store::prune_events` — a lagging
    /// consumer's backlog is retained regardless of age). Unset = keep
    /// forever, today's behaviour and the reactor-down-6wk guarantee.
    /// Measured cost of forever at 10k episodes: the log is 30% of the
    /// database (`docs/design/wasm-support.md` §5.1).
    pub retention_days: Option<u32>,
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

/// Quipu configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QuipuConfig {
    /// Path to the triple store database (default: `.bobbin/quipu/quipu.db`).
    pub store_path: PathBuf,

    /// Graph-label enforcement floors (quipu #68). All unset by default.
    ///
    /// Named `label_floors` rather than `labels`, though the TOML key stays
    /// `[quipu.labels]` per the design. The wiring guard below greps the tree
    /// for `.field`, so a field sharing its name with ANY other struct's field
    /// is reported wired by that other struct — `LabeledResult.labels` (#67)
    /// did exactly that, and the guard passed with this field's only consumer
    /// deleted. A distinctive Rust name is what makes the guard able to check
    /// it at all; serde keeps the config file unchanged.
    #[serde(rename = "labels")]
    pub label_floors: LabelsConfig,

    // aegis-4h3x: `schema_path` was removed here. It was a documented config key
    // with NO reader anywhere in the tree — accepted, defaulted, and inert, the
    // same false-affordance as `base_ns` was on the ingest path. Deleting is
    // parse-safe (this struct is `#[serde(default)]`, no `deny_unknown_fields`,
    // so an old config carrying `schema_path` is simply ignored). If a schema
    // directory is wanted later, add it back WITH the code that reads it.
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

    /// SHACL validation policy.
    pub shacl: ShaclConfig,

    /// OWL write-time constraint policy (disjointWith, `FunctionalProperty`).
    pub owl: OwlConfig,

    /// Governance enforcement policy (write-path gate).
    pub governance: GovernanceConfig,

    /// Event-log retention (quipu-9z9). Default: keep forever.
    pub events: EventsConfig,
}

impl Default for QuipuConfig {
    fn default() -> Self {
        Self {
            store_path: PathBuf::from(".bobbin/quipu/quipu.db"),
            label_floors: LabelsConfig::default(),
            base_ns: namespace::DEFAULT_BASE_NS.to_string(),
            server: ServerConfig::default(),
            federation: FederationConfig::default(),
            embedding: EmbeddingConfig::default(),
            vector: VectorConfig::default(),
            resolution: ResolutionConfig::default(),
            search: SearchConfig::default(),
            shacl: ShaclConfig::default(),
            owl: OwlConfig::default(),
            governance: GovernanceConfig::default(),
            events: EventsConfig::default(),
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

    /// Bearer token required on write endpoints (hq-azs). When set, a write
    /// must present `Authorization: Bearer <token>`. None (default) = open
    /// writes, preserving today's LAN-trusted behaviour.
    pub auth_token: Option<String>,

    /// Reject all write endpoints with 403 (hq-azs). Reads stay available.
    pub read_only: bool,

    /// CORS origin allowlist (hq-azs). Empty (default) = allow any origin,
    /// preserving the existing browser-tab behaviour; non-empty restricts
    /// cross-origin requests to these exact origins.
    pub cors_allowed_origins: Vec<String>,

    /// Number of read-only connections serving reads concurrently
    /// this. WAL permits N concurrent readers; before the pool every
    /// read queued behind the single writer connection, so effective
    /// parallelism was 1.0 at every concurrency.
    ///
    /// **0 disables the pool** and every read serialises behind the writer
    /// lock — the pre-pool behaviour, kept reachable as the rollback that does
    /// not need a redeploy.
    ///
    /// Default 4: enough to show the curve flatten without opening a file
    /// handle per request. Readers are cheap but not free — each is an open
    /// `SQLite` connection with its own page cache.
    pub read_pool_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1:3030".to_string(),
            auth_token: None,
            read_only: false,
            cors_allowed_origins: Vec::new(),
            read_pool_size: 4,
        }
    }
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

// `QuipuConfig::load` / `load_from` (the host-file half) live in
// `config_load` — split for the file-size ratchet, native-only by nature.
impl QuipuConfig {
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

    /// Warnings for config knobs that are SET to a non-default value but that the
    /// `quipu` CLI and `quipu-server` binaries do not act on.
    ///
    /// These are documented, settable capabilities that this repo wires to
    /// nothing: `vector.backend = "lancedb"` (the binaries never install a
    /// non-SQLite backend — that is an embedder-only path via
    /// `Store::set_local_vector_backend`) and `federation.remotes` (there is no
    /// remote `GraphProvider`, so remotes are ignored). Returned as strings so the
    /// binaries can print them and a test can assert exactly which knobs are
    /// unwired — the point is that a set-but-inert knob is LOUD, not silent. When
    /// one of these is actually wired, remove its branch here AND its entry in
    /// `config_knobs_are_wired_or_listed_unwired`.
    pub fn unwired_warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        if self.vector.backend == VectorBackend::Lancedb {
            w.push(
                "vector.backend = \"lancedb\" is set but the quipu CLI/server do not read it; \
                 queries still use the SQLite vectors table. LanceDB is an embedder-only backend \
                 (Store::set_local_vector_backend)."
                    .to_string(),
            );
        }
        w
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
