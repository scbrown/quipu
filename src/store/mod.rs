//! The core fact log store backed by `SQLite`.

pub mod events;
pub mod ops;
pub mod overlays;
pub mod push;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use rusqlite::{Connection, params};

use crate::config::{
    EmbeddingConfig, GovernanceConfig, ResolutionConfig, SearchConfig, ShaclConfig,
};
use crate::embedding::EmbeddingProvider;
use crate::error::{Error, Result};
use crate::governance::PolicyRegistry;
use crate::schema::INIT_SQL;
use crate::types::Value;
use crate::vector::{KnowledgeVectorStore, VECTORS_SQL};
use crate::vector_delegate::{DelegatingVectorStore, VectorSearchDelegate};

/// The core fact log store backed by `SQLite`.
pub struct Store {
    pub(crate) conn: Connection,
    /// v1 verdict-signing identity (ed25519, host-file key). When set, the
    /// governance committed-tier evaluator signs verdicts (aegis-g1al / the
    /// loom, Phase 0). The server loads it at startup; None → verdicts are
    /// evaluated but unsigned.
    pub(crate) signing: Option<Arc<crate::signing::SigningIdentity>>,
    pub(crate) embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pub(crate) embedding_config: EmbeddingConfig,
    /// Entity-resolution policy applied on the episode write paths. Defaults to
    /// disabled; the server sets it from `[quipu.resolution]` at startup so
    /// dedup actually fires on ingest (hq-uye).
    pub(crate) resolution_config: ResolutionConfig,
    /// Search/limit guardrails. Defaults are conservative; the server sets
    /// these from `[quipu.search]` at startup so callers can't request
    /// unbounded result sets (hq-gkd).
    pub(crate) search_config: SearchConfig,
    /// SHACL validation policy. When `validate_on_write` is set, episode
    /// ingest is validated against the persistently-loaded shapes (hq-c6s).
    pub(crate) shacl_config: ShaclConfig,
    /// Governance enforcement policy. When `enforce_on_write` is set, the write
    /// path evaluates `boundary:"action"` policies against the pending state and
    /// rejects a write that leaves a governed target non-compliant (the loom's
    /// write-time gate). Default disabled. See `docs/design/policy-edit-hooks.md`.
    pub(crate) governance_config: GovernanceConfig,
    /// Cached registry of active action-boundary policies, indexed by target
    /// type. Built lazily on the first enforced write and invalidated when a
    /// transaction defines or amends a policy. `None` = not yet built / stale.
    pub(crate) policy_registry: Option<PolicyRegistry>,
    /// Base namespace new IRIs are minted under on the episode write paths.
    /// Defaults to the built-in aegis namespace; the server sets it from
    /// `[quipu].base_ns` at startup so a non-aegis deployment does not silently
    /// mint aegis IRIs (aegis-4h3x). An IRI namespace is data identity, not a
    /// setting — it cannot be changed after the first write without orphaning
    /// every fact already stored, so the configured value MUST reach the ingest
    /// path. Before this field, `config.base_ns` was read by nothing and every
    /// REST/MCP ingest hardcoded `DEFAULT_BASE_NS`.
    pub(crate) base_ns: String,
    /// When set, vector search is delegated to an external provider (e.g.
    /// Bobbin's `LanceDB`). Auto-embedding on write is skipped.
    pub(crate) vector_delegate: Option<DelegatingVectorStore>,
    /// When set, vector operations use this local backend instead of the
    /// built-in `SQLite` vectors table. Unlike `vector_delegate`, this is a
    /// full read+write backend and auto-embedding still works.
    pub(crate) local_vector_backend: Option<Box<dyn KnowledgeVectorStore + Send + Sync>>,
    /// Registered transaction observers. Called after each successful
    /// `transact()`. Feature-gated behind `reactive-reasoner`.
    #[cfg(feature = "reactive-reasoner")]
    pub(crate) observers: Vec<Arc<dyn TransactObserver>>,
    /// Advisory events queued by the CURRENT write's pre-validation (event
    /// P3: `quipu:onViolation "emit"` shapes, event-based design §5/§7). Drained by
    /// `emit_events` INSIDE the write's savepoint, so they commit or roll back
    /// atomically with the facts they describe. `RefCell` because the emit
    /// path runs under `&self`; single-connection Store is not `Sync`-shared.
    pub(crate) pending_write_events: std::cell::RefCell<Vec<PendingWriteEvent>>,
}

/// An advisory event observed before a write and appended with it (P3).
#[derive(Debug, Clone)]
pub struct PendingWriteEvent {
    /// Event type, e.g. `shacl.violation`.
    pub event_type: String,
    /// The subject entity IRI (e.g. the violating focus node).
    pub subject: Option<String>,
    /// Structured detail (shape, message, component, path, severity).
    pub payload: serde_json::Value,
}

/// A summary of what changed in a committed transaction.
///
/// Built by [`Store::transact`] after the SQLite commit succeeds and
/// delivered to every registered [`TransactObserver`]. Observers can
/// inspect which facts were asserted or retracted and decide whether to
/// react (e.g. re-derive affected rules).
#[cfg(feature = "reactive-reasoner")]
#[derive(Debug, Clone)]
pub struct Delta {
    /// The transaction id that produced this delta.
    pub tx: i64,
    /// Facts that were asserted in this transaction.
    pub asserts: Vec<Datum>,
    /// Facts that were retracted in this transaction.
    pub retracts: Vec<Datum>,
    /// The `source` tag on the transaction (e.g. `"reasoner:R1"`).
    /// Observers use this to avoid re-triggering on their own output.
    pub source: Option<String>,
}

/// Trait for components that want to react to committed transactions.
///
/// Register an observer via [`Store::add_observer`]. After every
/// successful [`Store::transact`] call, the store builds a [`Delta`]
/// and calls [`after_commit`](TransactObserver::after_commit) on each
/// registered observer in registration order.
///
/// **Recursion safety:** observers must check `delta.source` and skip
/// transactions they produced — otherwise they will recurse infinitely.
/// The observer dispatch clones the observer vec before calling, so
/// calling `store.transact()` inside `after_commit` is safe.
#[cfg(feature = "reactive-reasoner")]
pub trait TransactObserver: Send + Sync {
    /// Called after a transaction commits. May call `store.transact()`
    /// to persist derived facts. Must skip its own output via
    /// `delta.source` to avoid infinite recursion.
    fn after_commit(&self, store: &mut Store, delta: &Delta) -> crate::error::Result<()>;
}

/// A write-side assertion or retraction within a transaction.
#[derive(Debug, Clone)]
pub struct Datum {
    pub entity: i64,
    pub attribute: i64,
    pub value: Value,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub op: crate::types::Op,
}

/// Temporal query parameters.
pub struct AsOf {
    /// Maximum transaction id to consider (None = latest).
    pub tx: Option<i64>,
    /// Point-in-time for valid-time filtering (None = current).
    pub valid_at: Option<String>,
}

impl Store {
    /// Queue an advisory event to ride the NEXT write's savepoint (event P3).
    /// Cleared by the write (commit or not); callers on an aborted path should
    /// call [`Store::clear_pending_write_events`] so nothing leaks forward.
    pub fn queue_write_event(&self, ev: PendingWriteEvent) {
        self.pending_write_events.borrow_mut().push(ev);
    }

    /// Drop any queued advisory events (abort-path hygiene).
    pub fn clear_pending_write_events(&self) {
        self.pending_write_events.borrow_mut().clear();
    }

    /// Open (or create) a Quipu store at the given path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Create an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(INIT_SQL)?;
        conn.execute_batch(VECTORS_SQL)?;
        Self::migrate_named_graphs(&conn)?;
        Ok(Self {
            conn,
            signing: None,
            embedding_provider: None,
            embedding_config: EmbeddingConfig::default(),
            resolution_config: ResolutionConfig::default(),
            search_config: SearchConfig::default(),
            shacl_config: ShaclConfig::default(),
            governance_config: GovernanceConfig::default(),
            pending_write_events: std::cell::RefCell::new(Vec::new()),
            policy_registry: None,
            base_ns: crate::namespace::DEFAULT_BASE_NS.to_string(),
            vector_delegate: None,
            local_vector_backend: None,
            #[cfg(feature = "reactive-reasoner")]
            observers: Vec::new(),
        })
    }

    /// Attach an embedding provider for auto-embedding on write.
    pub fn set_embedding_provider(&mut self, provider: Arc<dyn EmbeddingProvider>) {
        self.embedding_provider = Some(provider);
    }

    /// Get a mutable reference to the embedding config.
    pub fn embedding_config_mut(&mut self) -> &mut EmbeddingConfig {
        &mut self.embedding_config
    }

    /// Get a reference to the embedding config.
    pub fn embedding_config(&self) -> &EmbeddingConfig {
        &self.embedding_config
    }

    /// Get a mutable reference to the entity-resolution config.
    pub fn resolution_config_mut(&mut self) -> &mut ResolutionConfig {
        &mut self.resolution_config
    }

    /// Get a reference to the entity-resolution config.
    pub fn resolution_config(&self) -> &ResolutionConfig {
        &self.resolution_config
    }

    /// The base namespace new IRIs are minted under on the episode write paths
    /// (aegis-4h3x). The server sets this from `[quipu].base_ns` at startup.
    pub fn base_ns(&self) -> &str {
        &self.base_ns
    }

    /// Set the base namespace for minted IRIs. Called once at startup from
    /// config; a per-call `--base-ns` still overrides at the ingest call site.
    pub fn set_base_ns(&mut self, base_ns: impl Into<String>) {
        self.base_ns = base_ns.into();
    }

    /// Get a mutable reference to the search/limit config.
    pub fn search_config_mut(&mut self) -> &mut SearchConfig {
        &mut self.search_config
    }

    /// Get a reference to the search/limit config.
    pub fn search_config(&self) -> &SearchConfig {
        &self.search_config
    }

    /// Get a mutable reference to the SHACL validation config.
    pub fn shacl_config_mut(&mut self) -> &mut ShaclConfig {
        &mut self.shacl_config
    }

    /// Get a reference to the SHACL validation config.
    pub fn shacl_config(&self) -> &ShaclConfig {
        &self.shacl_config
    }

    /// Get a mutable reference to the governance enforcement config.
    pub fn governance_config_mut(&mut self) -> &mut GovernanceConfig {
        &mut self.governance_config
    }

    /// Get a reference to the governance enforcement config.
    pub fn governance_config(&self) -> &GovernanceConfig {
        &self.governance_config
    }

    /// Evaluate action-boundary policies for a staged write. No-op unless
    /// `governance.enforce_on_write` is set. Builds and caches the policy
    /// registry on first use. Returns `Err(PolicyDenied)` when a `deny` policy's
    /// claim is unsatisfied for a touched target — the caller rolls the write
    /// back so nothing is committed.
    pub(crate) fn enforce_write_policies(&mut self, datums: &[Datum], graph: i64) -> Result<()> {
        if !self.governance_config.enforce_on_write {
            return Ok(());
        }
        if self.policy_registry.is_none() {
            self.policy_registry = Some(PolicyRegistry::build(self)?);
        }
        // Take the registry out so the evaluator can borrow `&self` (SPARQL);
        // restore it afterwards. Evaluation never mutates the registry.
        let registry = self.policy_registry.take().expect("registry just built");
        let result = registry.evaluate_write(self, datums, graph);
        self.policy_registry = Some(registry);
        result
    }

    /// Invalidate the cached policy registry if this transaction defined or
    /// amended a governance policy. Cheap no-op unless enforcement is enabled.
    pub(crate) fn invalidate_policy_registry_if_governance(
        &mut self,
        datums: &[Datum],
    ) -> Result<()> {
        if !self.governance_config.enforce_on_write {
            return Ok(());
        }
        if crate::governance::is_governance_write(self, datums)? {
            self.policy_registry = None;
        }
        Ok(())
    }

    /// Set an external vector search delegate.
    ///
    /// When set, all vector search methods forward to the delegate and
    /// auto-embedding on write is skipped (embeddings belong in the delegate).
    pub fn set_vector_search_delegate(&mut self, delegate: Arc<dyn VectorSearchDelegate>) {
        self.vector_delegate = Some(DelegatingVectorStore::new(delegate));
    }

    /// Returns `true` if vector search is delegated to an external provider.
    pub fn has_vector_delegate(&self) -> bool {
        self.vector_delegate.is_some()
    }

    /// Set a local vector backend (e.g. `LanceDB`) that replaces the built-in
    /// `SQLite` vectors table for all vector operations.
    ///
    /// Unlike [`set_vector_search_delegate`], this is a full read+write backend
    /// and auto-embedding on write still works.
    pub fn set_local_vector_backend(
        &mut self,
        backend: Box<dyn KnowledgeVectorStore + Send + Sync>,
    ) {
        self.local_vector_backend = Some(backend);
    }

    /// Returns `true` if a local vector backend is configured.
    pub fn has_local_vector_backend(&self) -> bool {
        self.local_vector_backend.is_some()
    }

    /// Register a [`TransactObserver`] that will be called after every
    /// successful [`transact`](Store::transact) call.
    #[cfg(feature = "reactive-reasoner")]
    pub fn add_observer(&mut self, observer: Arc<dyn TransactObserver>) {
        self.observers.push(observer);
    }

    /// Returns `true` if an embedding provider is attached.
    pub fn has_embedding_provider(&self) -> bool {
        self.embedding_provider.is_some()
    }

    /// Attach a verdict-signing identity (v1 root of trust, Phase 0). When set,
    /// the committed-tier evaluator signs the verdicts it produces.
    pub fn set_signing_identity(&mut self, identity: Arc<crate::signing::SigningIdentity>) {
        self.signing = Some(identity);
    }

    /// The attached signing identity, if any.
    pub fn signing_identity(&self) -> Option<Arc<crate::signing::SigningIdentity>> {
        self.signing.clone()
    }

    /// Returns a clone of the embedding provider, if one is attached.
    pub fn embedding_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedding_provider.clone()
    }

    /// Embed a query string using the attached provider.
    ///
    /// Returns `None` if no provider is set. This allows search endpoints
    /// to accept natural-language `query` text and auto-embed it rather
    /// than requiring callers to supply pre-computed vectors.
    pub fn embed_query(&self, text: &str) -> Result<Option<Vec<f32>>> {
        match &self.embedding_provider {
            Some(provider) => Ok(Some(provider.embed_text(text)?)),
            None => Ok(None),
        }
    }

    // -- Term dictionary --

    /// Intern an IRI, returning its integer id.
    /// Additive migration for named-graph support (aegis-g1al / #36). A store
    /// created before the `g` column existed has a `facts` table without it;
    /// `CREATE TABLE IF NOT EXISTS` is a no-op there, so add the column here.
    /// Idempotent: it checks `PRAGMA table_info` and only ALTERs if `g` is
    /// absent. Existing rows default to g=0 (ROOT), so all prior data lands in
    /// the source-of-truth graph un-mutated and a no-dataset query still sees
    /// exactly what it saw before — the migration changes no query's meaning.
    ///
    /// It also owns the `idx_geav` graph index (NOT `schema::INIT_SQL`), and
    /// creates it unconditionally for both fresh and just-migrated stores.
    /// `INIT_SQL` runs first and against pre-quad stores too, so a
    /// `CREATE INDEX ... ON facts(g, ...)` there hard-fails with
    /// `no such column: g` before this ALTER can add the column (aegis-akb8:
    /// caught by a scratch-copy smoke test before a blind swap would have
    /// crash-looped the live graph on open).
    fn migrate_named_graphs(conn: &Connection) -> Result<()> {
        let has_g: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('facts') WHERE name = 'g'")?
            .exists([])?;
        if !has_g {
            conn.execute_batch("ALTER TABLE facts ADD COLUMN g INTEGER NOT NULL DEFAULT 0;")?;
        }
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_geav ON facts(g, e, a, v);")?;
        Ok(())
    }

    pub fn intern(&self, iri: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO terms (iri) VALUES (?1)",
            params![iri],
        )?;
        let id: i64 =
            self.conn
                .query_row("SELECT id FROM terms WHERE iri = ?1", params![iri], |row| {
                    row.get(0)
                })?;
        Ok(id)
    }

    /// Resolve a term id back to its IRI.
    pub fn resolve(&self, id: i64) -> Result<String> {
        self.conn
            .query_row("SELECT iri FROM terms WHERE id = ?1", params![id], |row| {
                row.get(0)
            })
            .map_err(|_| Error::UnknownTerm(id))
    }

    /// Look up a term id by IRI, returning None if not interned.
    pub fn lookup(&self, iri: &str) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM terms WHERE iri = ?1")?;
        let mut rows = stmt.query(params![iri])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Cheap graph-size counts for the /metrics gauges: (entities, facts,
    /// predicates) over LIVE root-graph facts — the same liveness predicate the
    /// query layer uses (`op = 1 AND g = 0 AND valid_to IS NULL`). One SQL
    /// aggregate pass; deliberately NOT the /stats full result-set scan, which
    /// is far too expensive to run on every Prometheus scrape.
    pub fn graph_counts(&self) -> Result<(u64, u64, u64)> {
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(DISTINCT e), COUNT(*), COUNT(DISTINCT a) FROM facts \
             WHERE op = 1 AND g = 0 AND valid_to IS NULL",
        )?;
        let mut rows = stmt.query([])?;
        let row = rows.next()?.expect("aggregate always returns one row");
        Ok((
            row.get::<_, i64>(0)? as u64,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
        ))
    }

    /// Retrieve a transaction by id.
    pub fn get_transaction(&self, tx_id: i64) -> Result<Option<crate::types::Transaction>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, timestamp, actor, source FROM transactions WHERE id = ?1")?;
        let mut rows = stmt.query(params![tx_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(crate::types::Transaction {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                actor: row.get(2)?,
                source: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    // -- Shape storage --

    /// Store a named SHACL shape graph.
    pub fn load_shapes(&self, name: &str, turtle: &str, timestamp: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO shapes (name, turtle, loaded_at) VALUES (?1, ?2, ?3)",
            params![name, turtle, timestamp],
        )?;
        Ok(())
    }

    /// Remove a stored shape graph by name.
    pub fn remove_shapes(&self, name: &str) -> Result<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM shapes WHERE name = ?1", params![name])?;
        Ok(affected > 0)
    }

    /// Get all stored shapes as a list of (name, turtle, `loaded_at`).
    pub fn list_shapes(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, turtle, loaded_at FROM shapes ORDER BY name")?;
        let mut shapes = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            shapes.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(shapes)
    }

    /// Get all stored shapes concatenated as a single Turtle string.
    pub fn get_combined_shapes(&self) -> Result<Option<String>> {
        let shapes = self.list_shapes()?;
        if shapes.is_empty() {
            return Ok(None);
        }
        let combined = shapes
            .iter()
            .map(|(_, turtle, _)| turtle.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Some(combined))
    }

    // -- Ontology storage --

    /// Store a named OWL ontology.
    pub fn load_ontology(&self, name: &str, turtle: &str, timestamp: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ontologies (name, turtle, loaded_at) VALUES (?1, ?2, ?3)",
            params![name, turtle, timestamp],
        )?;
        Ok(())
    }

    /// Remove a stored ontology by name.
    pub fn remove_ontology(&self, name: &str) -> Result<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM ontologies WHERE name = ?1", params![name])?;
        Ok(affected > 0)
    }

    /// Get all stored ontologies as a list of (name, turtle, `loaded_at`).
    pub fn list_ontologies(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, turtle, loaded_at FROM ontologies ORDER BY name")?;
        let mut ontologies = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            ontologies.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(ontologies)
    }

    /// Get all stored ontologies concatenated as a single Turtle string.
    pub fn get_combined_ontologies(&self) -> Result<Option<String>> {
        let ontologies = self.list_ontologies()?;
        if ontologies.is_empty() {
            return Ok(None);
        }
        let combined = ontologies
            .iter()
            .map(|(_, turtle, _)| turtle.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Some(combined))
    }

    // -- SQL access (for SPARQL evaluator) --

    /// Prepare a SQL statement against the underlying connection.
    pub(crate) fn prepare(&self, sql: &str) -> Result<rusqlite::Statement<'_>> {
        Ok(self.conn.prepare(sql)?)
    }
}
