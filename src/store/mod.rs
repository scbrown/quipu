//! The core fact log store backed by `SQLite`.

pub mod alias;
pub mod attach;
/// Durable session bindings + replay state for the common attestation
/// verifier. Native only: `session_attestation` is itself `cfg(not(wasm32))`.
#[cfg(not(target_arch = "wasm32"))]
pub mod attestation;
pub mod changes;
pub mod datasets;
pub mod events;
pub mod forks;
pub mod freeze;
mod freeze_io;
mod gate;
mod identity;
pub mod import;
pub mod inferred;
pub mod labels;
pub mod labels_advisory;
mod migrate;
mod open;
pub mod ops;
pub mod overlays;
pub mod push;
pub mod queries;
pub mod read_model;
mod reads;
mod registry;
pub mod registry_list;
pub mod respace;
mod respace_map;
mod retraction;
mod serialize;
mod set;
mod settings;
#[cfg(not(target_arch = "wasm32"))]
pub mod snapshot_upload;
pub mod terms;
pub mod wal;
pub(crate) use terms::TermCache;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod attestation_tests;
#[cfg(test)]
mod term_space_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::{
    EmbeddingConfig, GovernanceConfig, OwlConfig, ResolutionConfig, SearchConfig, ShaclConfig,
};
use crate::embedding::EmbeddingProvider;
use crate::error::{Error, Result};
use crate::governance::PolicyRegistry;
use crate::types::Value;
use crate::vector::KnowledgeVectorStore;
use crate::vector_delegate::DelegatingVectorStore;

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
    pub(crate) labels_config: crate::config::LabelsConfig,
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
    /// The gate refusal of the CURRENT write, staged by the gate that refused
    /// inside the `quipu_transact` savepoint and recorded as a `write.refused`
    /// event AFTER that savepoint has rolled back (camayoc-0d3) — an event
    /// inserted before the rollback would die with it, same reason as
    /// `pending_verdicts`. Taken (and thus cleared) on the write's error path.
    pub(crate) pending_refusal: Option<events::PendingRefusal>,
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
    /// Read-only databases mounted alongside this one (quipu #75). Empty for
    /// every store that did not ask for attachments, which is the default and
    /// must stay indistinguishable from before the feature existed.
    pub(crate) attachments: Vec<attach::Attachment>,
    /// Manifests surfaced from attached knowledge packs (quipu #82).
    pub(crate) pack_manifests: Vec<(String, crate::pack::Manifest)>,
    /// The table source a composed query reads `facts` from — see
    /// [`attach::build_facts_source`]. Exactly `"facts"` with no attachments,
    /// which is the byte-identical SQL every query built before quipu #75.
    ///
    /// A cached `String` rather than the design's `Cow<'_, str>`: building it
    /// reads each attachment's `terms` table for its meta-graph id, and a
    /// triple pattern is evaluated per BGP per solution — recomputing it per
    /// query would put a `SELECT` behind every one. It is fixed at open, as the
    /// attachments themselves are.
    pub(crate) facts_source: String,
    /// The SQL [`Self::resolve`] uses to turn a term id back into an IRI — see
    /// [`attach::build_resolve_sql`]. Exactly today's single-table query with
    /// no attachments.
    pub(crate) resolve_sql: String,
    /// Memoized term dictionary — see [`TermCache`].
    pub(crate) term_cache: std::cell::RefCell<TermCache>,
    /// Resident read models, one per graph, built on demand (quipu-nip) —
    /// see [`read_model::ReadModel`] and [`Store::read_model_for`]. The
    /// combined size is bounded by `read_model_max_triples`, so a large ROOT
    /// past the budget keeps the SQL path while a small derived graph stays
    /// resident.
    pub(crate) read_model:
        std::cell::RefCell<std::collections::HashMap<i64, read_model::ReadModel>>,
    /// Last graph projection, memoized with a tx stamp (quipu-tz5) — see
    /// [`crate::graph::project_cached`].
    pub(crate) projected_graph: std::cell::RefCell<Option<crate::graph::ProjectionCacheEntry>>,
    /// Whether SPARQL consults the read model. Off by default — see
    /// [`Store::set_read_model_enabled`] for the measurements that decided it.
    pub(crate) read_model_enabled: std::cell::Cell<bool>,
    /// Set while a write holds an open savepoint. See
    /// [`Store::write_in_progress`].
    pub(crate) write_in_progress: std::cell::Cell<bool>,
    /// Ceiling on the resident read model's size — see
    /// [`Store::set_read_model_max_triples`].
    pub(crate) read_model_max_triples: std::cell::Cell<usize>,
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

    // -- SQL access (for SPARQL evaluator) --

    /// Prepare a SQL statement against the underlying connection.
    pub(crate) fn prepare(&self, sql: &str) -> Result<rusqlite::Statement<'_>> {
        Ok(self.conn.prepare(sql)?)
    }
}

/// The space a bare connection allocates from (quipu #74).
///
/// A store with no local row predates the registry, and a legacy store is space
/// 0 by definition — so the absence reads as 0 rather than as an error.
pub(crate) fn local_term_space(conn: &Connection) -> Result<i64> {
    let space: Option<i64> = conn
        .query_row("SELECT space FROM term_spaces WHERE local = 1", [], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(space.unwrap_or(0))
}

/// Intern an IRI, allocating within this database's term space (quipu #74).
///
/// Takes a bare `Connection` because `migrate_graph_labels` interns the
/// meta-graph IRI before a `Store` exists, and that intern must be space-aware
/// for exactly the same reason every other one is.
///
/// ## ⛔ `k` MUST be derived from the table, never from an independent counter.
///
/// Space 0 is already densely occupied on every existing store, at positions
/// this code did not choose: measured on a fresh store, `id 1` is the reserved
/// meta-graph and the first user term is `2`, and on a store migrated long
/// after creation the meta-graph sits at whatever rowid happened to be next.
/// An allocator that invents `k` from its own counter hands back an id that is
/// already bound — silently repointing a live term. That is the failure this
/// branch exists to prevent, and `allocation_never_returns_an_id_already_bound`
/// is the test that catches it (verified by sabotage: a naive counter fails 5
/// of the 8 tests in that file).
///
/// The space-0 branch below keeps the literal rowid path, which is what makes
/// allocation byte-identical to pre-#74 and is the reason #74 is inert by
/// default. **But note what is NOT true:** "space 0 must delegate to the rowid"
/// cannot be enforced by a test, because the `else` branch computes
/// `MAX(id) + 1` within the space and for space 0 that IS the rowid — the two
/// are the same function. Sabotage confirmed it: disabling this branch entirely
/// changed no observable behaviour. The rowid path is kept for clarity and
/// because it is obviously right, not because anything would catch its removal.
/// The property that is genuinely load-bearing is the one in the heading.
///
/// If some future change needs `k` for space 0 from any source other than the
/// table, that is a STOP and a fresh go/no-go — not an implementation detail
/// (sattler, ruled 2026-08-06).
pub(crate) fn intern_in_space(conn: &Connection, iri: &str) -> Result<i64> {
    // Already interned is the overwhelmingly common path and is space-agnostic.
    let existing: Option<i64> = conn
        .query_row("SELECT id FROM terms WHERE iri = ?1", params![iri], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let space = local_term_space(conn)?;
    if space == 0 {
        // The rowid path, unchanged. See the ⛔ note above.
        conn.execute(
            "INSERT OR IGNORE INTO terms (iri) VALUES (?1)",
            params![iri],
        )?;
    } else {
        // Allocate the next free id WITHIN this space's half-open range. A
        // fresh non-zero-space store has an empty `terms`, where the rowid
        // would be 1 — outside the space entirely — so the id is explicit.
        // `k` starts at 1, exactly as it does in space 0. The base `s * 2^40`
        // (k = 0) is deliberately never allocated: space 0 reserves id 0 for
        // ROOT_GRAPH, and giving every space the identical k-range is what lets
        // "legacy ids are 1..n" mean "space 0", rather than nearly meaning it.
        let lo = space * crate::schema::SPACE_SIZE;
        let hi = lo + crate::schema::SPACE_SIZE;
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(id), ?1) + 1 FROM terms WHERE id >= ?1 AND id < ?2",
            params![lo, hi],
            |r| r.get(0),
        )?;
        if next >= hi {
            return Err(Error::InvalidValue(format!(
                "term space {space} is exhausted ({} terms); \
                 respace this database into a fresh space",
                crate::schema::SPACE_SIZE
            )));
        }
        conn.execute(
            "INSERT OR IGNORE INTO terms (id, iri) VALUES (?1, ?2)",
            params![next, iri],
        )?;
    }

    Ok(
        conn.query_row("SELECT id FROM terms WHERE iri = ?1", params![iri], |r| {
            r.get(0)
        })?,
    )
}
