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
    EmbeddingConfig, GovernanceConfig, OwlConfig, ResolutionConfig, SearchConfig, ShaclConfig,
};
use crate::embedding::EmbeddingProvider;
use crate::error::{Error, Result};
use crate::governance::PolicyRegistry;
use crate::schema::INIT_SQL;
#[cfg(feature = "owl")]
use crate::types::Op;
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
    /// OWL write-time constraint policy (aegis-bmqup).
    pub(crate) owl_config: OwlConfig,
    /// Ontology built from the stored ontologies, cached because the write gate
    /// would otherwise re-parse every stored TTL on EVERY transaction. Set to
    /// `None` to invalidate — `load_ontology`/`remove_ontology` do exactly that,
    /// so a newly loaded axiom is enforced on the very next write rather than
    /// after a restart.
    #[cfg(feature = "owl")]
    pub(crate) owl_cache: Option<Box<crate::owl::Ontology>>,
    /// Governance enforcement policy. When `enforce_on_write` is set, the write
    /// path evaluates `boundary:"action"` policies against the pending state and
    /// rejects a write that leaves a governed target non-compliant (the loom's
    /// write-time gate). Default disabled. See `docs/design/policy-edit-hooks.md`.
    pub(crate) governance_config: GovernanceConfig,
    /// Cached registry of active action-boundary policies, indexed by target
    /// type. Built lazily on the first enforced write and invalidated when a
    /// transaction defines or amends a policy. `None` = not yet built / stale.
    pub(crate) policy_registry: Option<PolicyRegistry>,
    /// Verdicts the write gate decided and has not yet written. Staged rather
    /// than emitted in place because a DENIED write is rolled back, and a
    /// verdict written inside that savepoint would go with it — losing exactly
    /// the record worth keeping. Drained after the savepoint resolves.
    pub(crate) pending_verdicts: Vec<crate::governance::verdict_facts::PendingVerdict>,
    /// `DecisionRequest`s the escalation router decided to open. Staged for the
    /// same reason the verdicts are: the refusal that opens one also rolls the
    /// savepoint back, and a request written in place would vanish with it —
    /// leaving a refusal with nothing for an operator to act on.
    pub(crate) pending_requests: Vec<crate::governance::router::PendingRequest>,
    /// The principal-and-agent chain the current caller is acting under (SARC
    /// §9.6's `P`). Empty means unattributed, which is NOT the same as
    /// unconstrained — see `enforce_graph_authority`.
    pub(crate) principal_chain: Vec<String>,
    /// True while the store is writing verdict facts. The gate honours it and
    /// skips: a policy targeting `aegis:Verdict` would otherwise deny the
    /// verdict recording its own denial.
    pub(crate) recording_verdicts: bool,
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
    /// When true, auto-embed after a write COLLECTS work (texts + closes)
    /// instead of running the multi-second ONNX embed inline under the
    /// caller's lock; the caller drains via [`Store::take_deferred_embed`],
    /// embeds lock-free, and writes back with [`Store::apply_deferred_embed`]
    ///. Default false: the CLI and library inline path is
    /// unchanged.
    pub(crate) defer_auto_embed: bool,
    /// Un-drained deferred embed work from committed transactions. The server
    /// drains this after every write handler; work only accumulates here
    /// within a single locked write section.
    pub(crate) pending_embed: Option<crate::embedding::DeferredEmbed>,
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
/// Built by [`Store::transact`] after the `SQLite` commit succeeds and
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

    /// Open an existing store READ-ONLY, for a reader in a connection pool.
    ///
    /// WAL already permits N concurrent readers alongside one writer; before
    /// this, the server serialised every read behind the single writer
    /// connection's mutex, so effective parallelism was 1.0 at every
    /// concurrency. A pool needs N connections because
    /// `rusqlite::Connection` is `Send` but **not** `Sync` — it cannot be
    /// shared behind a read lock, only moved and owned exclusively.
    ///
    /// Two deliberate differences from [`Self::open`]:
    ///
    /// - `SQLITE_OPEN_READ_ONLY`, so a bug on a read path cannot write. This is
    ///   the mechanism, not the comment: a stray `INSERT` here fails at SQLite
    ///   rather than racing the writer.
    /// - **No DDL and no migration.** `init` runs `INIT_SQL`, `VECTORS_SQL` and
    ///   `migrate_named_graphs` on every open. Running schema setup N more times
    ///   at startup is at best redundant work against the writer's database and
    ///   at worst a concurrent migration; a read-only connection cannot do it
    ///   anyway. The writer owns schema, exclusively.
    ///
    /// The returned store carries DEFAULT configuration. Callers must copy the
    /// read-relevant policy across with [`Self::adopt_read_config_from`] — a
    /// pooled reader running different search guardrails than the writer would
    /// silently answer the same question two ways depending on which connection
    /// served it.
    pub fn open_read_only(path: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        // foreign_keys is a no-op for reads but keeps the connection's semantics
        // identical to the writer's; query_only makes the read-only intent true
        // at the SQL layer as well as the file-handle layer.
        //
        // `mmap_size` is NOT a micro-optimisation here, it is what makes the
        // pool a win at all — MEASURED, and it was measured because the first
        // version of this function omitted it and the acceptance curve caught
        // the result.
        //
        // Going from one connection to N trades a lock for N private page
        // caches. The single shared connection had ONE cache that every
        // serialised reader warmed for the next; eight connections each fault
        // the same pages in separately. With 8 concurrent readers on a 160k-fact
        // store that showed up as `wait` falling to 0.000s — the lock contention
        // genuinely gone — while `held` rose from 0.136s to 1.72s per query.
        // Parallelism was real and per-query cost had inflated 13x to pay for
        // it, so wall time barely moved. A pool can serialise silently; it can
        // also PARALLELISE silently and still buy nothing, and only the curve
        // tells them apart.
        //
        // Memory-mapped I/O restores the sharing: every connection maps the same
        // file pages through the OS page cache instead of copying them into a
        // private heap cache, so N readers cost roughly one reader's memory.
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA query_only=ON;
             PRAGMA mmap_size=268435456;
             PRAGMA cache_size=-32000;",
        )?;
        Ok(Self::with_connection(conn))
    }

    /// Copy the READ-relevant configuration from another store.
    ///
    /// Reads are policy-bearing: `search_config` bounds result sets (hq-gkd),
    /// `base_ns` decides which IRIs a query is even about, and
    /// `owl_config` governs subclass inference on the read path. A pooled reader
    /// left on defaults would answer differently from the writer for reasons no
    /// caller could see — the same class of defect as a fact that is present and
    /// unretrievable.
    ///
    /// Write-path policy (`resolution`, `shacl`, `governance`, `signing`) is
    /// deliberately NOT copied: a read-only connection never reaches those
    /// gates, and copying them would imply a reader could write.
    ///
    /// Not copied either, and the reason is a real constraint rather than a
    /// preference: the vector backends (`vector_delegate`,
    /// `local_vector_backend`) are boxed trait objects and are not `Clone`, so
    /// vector-search handlers stay on the writer connection. See `ReadPool` in
    /// the server for which handlers are pooled.
    pub fn adopt_read_config_from(&mut self, other: &Self) {
        self.base_ns.clone_from(&other.base_ns);
        self.search_config.clone_from(&other.search_config);
        self.owl_config.clone_from(&other.owl_config);
        self.embedding_config.clone_from(&other.embedding_config);
        self.embedding_provider
            .clone_from(&other.embedding_provider);
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(INIT_SQL)?;
        conn.execute_batch(VECTORS_SQL)?;
        Self::migrate_named_graphs(&conn)?;
        Ok(Self::with_connection(conn))
    }

    /// Wrap an already-opened connection in a default-configured `Store`,
    /// running no DDL. Split out of `init` so `open_read_only` can share the
    /// struct construction without sharing the schema setup — the two must not
    /// drift, or a pooled reader would be a different kind of Store than the
    /// writer.
    fn with_connection(conn: Connection) -> Self {
        Self {
            conn,
            signing: None,
            embedding_provider: None,
            embedding_config: EmbeddingConfig::default(),
            resolution_config: ResolutionConfig::default(),
            search_config: SearchConfig::default(),
            shacl_config: ShaclConfig::default(),
            owl_config: OwlConfig::default(),
            #[cfg(feature = "owl")]
            owl_cache: None,
            governance_config: GovernanceConfig::default(),
            pending_write_events: std::cell::RefCell::new(Vec::new()),
            policy_registry: None,
            pending_verdicts: Vec::new(),
            pending_requests: Vec::new(),
            principal_chain: Vec::new(),
            recording_verdicts: false,
            base_ns: crate::namespace::DEFAULT_BASE_NS.to_string(),
            vector_delegate: None,
            local_vector_backend: None,
            defer_auto_embed: false,
            pending_embed: None,
            #[cfg(feature = "reactive-reasoner")]
            observers: Vec::new(),
        }
    }

    /// Defer auto-embedding: writes collect embed work instead of running the
    /// ONNX embed under the caller's lock. The caller MUST drain with
    /// [`Self::take_deferred_embed`] after each write and finish via
    /// [`Self::apply_deferred_embed`], or new/changed entities silently get no
    /// embeddings (the server's write handlers drain uniformly).
    pub fn set_defer_auto_embed(&mut self, on: bool) {
        self.defer_auto_embed = on;
    }

    /// Take any pending deferred-embed work (None when the last write touched
    /// nothing embeddable, or deferral is off).
    pub fn take_deferred_embed(&mut self) -> Option<crate::embedding::DeferredEmbed> {
        self.pending_embed.take()
    }

    /// Write vectors computed outside the lock for previously-taken work.
    /// Entities whose text changed since collection are skipped (a later
    /// writer owns them — see `embedding::apply_deferred_embed`). Returns the
    /// number written.
    pub fn apply_deferred_embed(
        &self,
        work: &crate::embedding::DeferredEmbed,
        embeddings: &[Vec<f32>],
    ) -> Result<usize> {
        crate::embedding::apply_deferred_embed(self, work, embeddings)
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

    /// The OWL write-time constraint policy.
    pub fn owl_config(&self) -> &OwlConfig {
        &self.owl_config
    }

    /// Mutable OWL policy, for enabling write-time enforcement at runtime.
    pub fn owl_config_mut(&mut self) -> &mut OwlConfig {
        &mut self.owl_config
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
        if !self.governance_config.enforce_on_write || self.recording_verdicts {
            return Ok(());
        }
        if self.policy_registry.is_none() {
            self.policy_registry = Some(PolicyRegistry::build(self)?);
        }
        // Take the registry out so the evaluator can borrow `&self` (SPARQL);
        // restore it afterwards. Evaluation never mutates the registry.
        let registry = self.policy_registry.take().expect("registry just built");
        let mut verdicts = Vec::new();
        let mut requests = Vec::new();
        let result = registry.evaluate_write(self, datums, graph, &mut verdicts, &mut requests);
        self.policy_registry = Some(registry);
        self.pending_requests = requests;
        // STAGED, not written. The caller writes them after the savepoint
        // resolves — a denial rolls back, and a verdict written inside that
        // savepoint would be rolled back with it.
        self.pending_verdicts = verdicts;
        result
    }

    /// Drop the cached ontology so the next write rebuilds it (aegis-bmqup).
    #[cfg(feature = "owl")]
    pub fn invalidate_owl_cache(&mut self) {
        self.owl_cache = None;
    }

    /// Close the prior value of a functional property so a new one can replace it
    /// (aegis-7vn3b).
    ///
    /// THE BUG THIS FIXES. The write path only ever CLOSED a fact on an exact
    /// `(e,a,v)` retraction, so asserting a different value for the same `(e,a)`
    /// left both live. Measured: `contentHash = "aaa"` then `contentHash = "bbb"`
    /// yields BOTH as current facts. Two consequences, one silent and one loud —
    /// cleaning duplicate scalars is undone by the next re-ingest (the aegis-h69po
    /// filePath fix went 205 → 0 → 50 in hours), and declaring the property
    /// `owl:FunctionalProperty` made every ordinary update an HTTP 400, because the
    /// update itself manufactured the second value.
    ///
    /// THE SEMANTICS. In a bitemporal store `owl:FunctionalProperty` means *at most
    /// one value AT A TIME*, so a new value must CLOSE the old — that is an update,
    /// and it is the common case. Rejection remains correct for two distinct values
    /// inside ONE batch, where nothing says which should win; those are left alone
    /// here and `enforce_owl_constraints` still refuses them.
    ///
    /// Tied to the same `owl.validate_on_write` flag as the rejection half, so the
    /// switch turns on one coherent behaviour rather than half of one.
    #[cfg(feature = "owl")]
    pub(crate) fn supersede_functional_values(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        graph: i64,
    ) -> Result<usize> {
        if !self.owl_config.validate_on_write || self.recording_verdicts {
            return Ok(0);
        }
        self.ensure_owl_cache()?;
        let Some(ontology) = self.owl_cache.take() else {
            return Ok(0);
        };
        let functional = ontology.axioms.functional_properties.clone();
        self.owl_cache = Some(ontology);
        if functional.is_empty() {
            return Ok(0);
        }

        // Group this batch's asserts by (entity, attribute) for functional attrs.
        let mut proposed: std::collections::HashMap<(i64, i64), Vec<Vec<u8>>> =
            std::collections::HashMap::new();
        for d in datums {
            if d.op != Op::Assert {
                continue;
            }
            let Ok(attr_iri) = self.resolve(d.attribute) else {
                continue;
            };
            if functional.contains(&attr_iri) {
                proposed
                    .entry((d.entity, d.attribute))
                    .or_default()
                    .push(d.value.to_bytes());
            }
        }

        let mut closed = 0usize;
        let mut close_other = self.conn.prepare(
            "UPDATE facts SET valid_to = ?1 \
             WHERE e = ?2 AND a = ?3 AND v != ?4 AND g = ?5 AND op = 1 AND valid_to IS NULL",
        )?;
        for ((entity, attribute), values) in &proposed {
            // AMBIGUOUS BATCH: two distinct values for one functional property in
            // a single write. Superseding here would silently pick whichever the
            // loop saw last. Leave it untouched — the validator rejects it, which
            // is the honest outcome when the caller has not said which wins.
            let distinct: std::collections::HashSet<&Vec<u8>> = values.iter().collect();
            if distinct.len() > 1 {
                continue;
            }
            let Some(new_value) = values.first() else {
                continue;
            };
            closed +=
                close_other.execute(params![timestamp, entity, attribute, new_value, graph])?;
        }
        Ok(closed)
    }

    /// Build the combined ontology cache if it is not already populated.
    #[cfg(feature = "owl")]
    fn ensure_owl_cache(&mut self) -> Result<()> {
        if self.owl_cache.is_some() {
            return Ok(());
        }
        let stored = self.list_ontologies()?;
        if stored.is_empty() {
            return Ok(());
        }
        let combined: String = stored
            .iter()
            .map(|(_, turtle, _)| turtle.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.owl_cache = Some(Box::new(crate::owl::Ontology::from_turtle(&combined)?));
        Ok(())
    }

    /// Reject a write that violates `owl:disjointWith` or `owl:FunctionalProperty`
    /// (aegis-bmqup).
    ///
    /// `Ontology::validate()` implemented both constraints and had NO CALLER in
    /// the server, while `docs/book/src/concepts/owl.md` stated that Quipu
    /// "enforces at write time" and listed them as enforced. This is that caller.
    ///
    /// Cost is bounded by the WRITE, not the store: `validate()` derives the
    /// touched entities from the proposed datums and then reads only those
    /// entities' existing facts. The ontology itself is cached because otherwise
    /// every transaction would re-parse every stored TTL.
    ///
    /// Off by default (`owl.validate_on_write`) — see `OwlConfig` for why
    /// flipping it on is a behaviour change, not a bug fix.
    #[cfg(feature = "owl")]
    pub(crate) fn enforce_owl_constraints(&mut self, datums: &[Datum]) -> Result<()> {
        if !self.owl_config.validate_on_write || self.recording_verdicts {
            return Ok(());
        }
        // One ontology over the union of every stored TTL: a disjointness
        // declared in one set must still bite a write validated against all.
        self.ensure_owl_cache()?;
        let Some(ontology) = self.owl_cache.take() else {
            return Ok(());
        };
        let result = ontology.validate(self, datums);
        self.owl_cache = Some(ontology);
        let violations = result?;
        if violations.is_empty() {
            return Ok(());
        }
        // Structured, and naming EVERY violation rather than just the first —
        // an author fixing them one round-trip at a time is how a strict gate
        // gets switched off.
        let detail = violations
            .iter()
            .map(|v| format!("{} ({})", v.message, v.focus_node))
            .collect::<Vec<_>>()
            .join("; ");
        Err(Error::InvalidValue(format!(
            "OWL constraint violation ({} violation(s)): {detail}",
            violations.len()
        )))
    }

    /// Write the verdicts the gate staged, in their own transaction.
    ///
    /// Called after the write's savepoint has resolved EITHER WAY, so the
    /// verdict of a denial survives the rollback that denial caused. Failures
    /// here are swallowed: a verdict that cannot be recorded must not turn a
    /// successful write into a failed one, nor a denial into a different error
    /// than the policy's.
    pub(crate) fn flush_pending_verdicts(&mut self, timestamp: &str) {
        self.flush_pending_requests(timestamp);
        let pending = std::mem::take(&mut self.pending_verdicts);
        if pending.is_empty() || self.recording_verdicts {
            return;
        }
        let mut datums = Vec::new();
        for verdict in &pending {
            match crate::governance::verdict_facts::datums_for(self, verdict, timestamp) {
                Ok(mut d) => datums.append(&mut d),
                // No signing identity => no verdict, never an unsigned one.
                Err(_) => return,
            }
        }
        if datums.is_empty() {
            return;
        }
        self.recording_verdicts = true;
        let _ = self.transact(
            &datums,
            timestamp,
            Some("quipu"),
            Some("write-gate verdict"),
        );
        self.recording_verdicts = false;
    }

    /// Set the principal-and-agent chain for subsequent writes.
    ///
    /// Ordered outermost-first: `[originating principal, …, executor]`. The
    /// effective authority is the INTERSECTION along it, so appending a delegate
    /// can only narrow what may be written (SARC §9.3).
    pub fn set_principal_chain(&mut self, chain: Vec<String>) {
        self.principal_chain = chain;
    }

    /// The current chain.
    #[must_use]
    pub fn principal_chain(&self) -> &[String] {
        &self.principal_chain
    }

    /// Refuse a write to `graph` that the chain's authority does not cover.
    ///
    /// Gated by `[quipu.governance] enforce_authority`, default off. With NO
    /// chain set the check does not apply: an unattributed write is the shape
    /// every existing caller has, and turning attribution into a hard
    /// requirement beneath a running deployment would break every one of them
    /// at once. What the flag buys is that a chain, once supplied, is BINDING —
    /// so adopting attribution is opt-in per caller and cannot silently widen.
    pub(crate) fn enforce_graph_authority(&self, graph: i64) -> Result<()> {
        if !self.governance_config.enforce_authority
            || self.recording_verdicts
            || self.principal_chain.is_empty()
        {
            return Ok(());
        }
        let graph_iri = if graph == crate::schema::ROOT_GRAPH {
            crate::schema::ROOT_GRAPH_IRI.to_string()
        } else {
            self.resolve(graph)?
        };
        let authority = crate::governance::authority::chain_authority(self, &self.principal_chain)?;
        if authority.permits(&graph_iri) {
            return Ok(());
        }
        Err(Error::PolicyDenied(crate::governance::authority::refusal(
            &self.principal_chain,
            &graph_iri,
            &authority,
        )))
    }

    /// Write the `DecisionRequest`s the router staged, after the savepoint has
    /// resolved. Same ordering, same reason, as the verdicts.
    fn flush_pending_requests(&mut self, timestamp: &str) {
        let pending = std::mem::take(&mut self.pending_requests);
        if pending.is_empty() || self.recording_verdicts {
            return;
        }
        let mut datums = Vec::new();
        for request in &pending {
            match crate::governance::router::mint_request(
                self,
                &request.policy_iri,
                &request.target_iri,
                None,
                request.window_secs,
                request.now,
                timestamp,
            ) {
                Ok(mut d) => datums.append(&mut d),
                Err(_) => return,
            }
        }
        if datums.is_empty() {
            return;
        }
        self.recording_verdicts = true;
        let _ = self.transact(
            &datums,
            timestamp,
            Some("quipu"),
            Some("escalation request"),
        );
        self.recording_verdicts = false;
    }

    /// Validate the SARC class↔placement rules for any policy this write
    /// defines or amends (`src/governance/placement.rs`). Gated by
    /// `[quipu.governance] validate_placement`, default off.
    ///
    /// Deliberately NOT gated by `enforce_on_write`: definition-time
    /// well-formedness of a constraint is a different question from
    /// evaluation-time enforcement of it, and a deployment may reasonably want
    /// its policy definitions checked while it is still staging enforcement in
    /// advise mode.
    pub(crate) fn validate_policy_placement(&self, datums: &[Datum], graph: i64) -> Result<()> {
        if !self.governance_config.validate_placement || self.recording_verdicts {
            return Ok(());
        }
        crate::governance::validate_placement(self, datums, graph)
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
            u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
            u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
        ))
    }

    /// The latest committed transaction id (0 for an empty store).
    ///
    /// Cheap (indexed MAX on the rowid-keyed transactions table) and
    /// monotonic, so callers can use it as a change-generation stamp for
    /// caches of derived read-side data: rebuild when it moves, reuse when it
    /// hasn't. Motivated by the spotlight reader-starvation incident, where
    /// re-deriving the labeled-entity list under the store lock on every call
    /// starved all readers.
    pub fn latest_tx_id(&self) -> Result<i64> {
        let mut stmt = self
            .conn
            .prepare("SELECT COALESCE(MAX(id), 0) FROM transactions")?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(0),
        }
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
        // NOTE: &self, so the cache is invalidated by the /ontology tool after
        // this returns (see invalidate_owl_cache). Kept here as the reminder that
        // a stale cache would enforce yesterday's axioms.
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
