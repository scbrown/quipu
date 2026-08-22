//! Store construction: open paths, schema init, and the attachment surface.
//!
//! Split from `mod.rs` (quipu-bu3). Everything here runs before or at the
//! moment a [`Store`] exists: the open variants, `init` and its migration
//! ordering, and the read-only accessors over what was mounted at open.

use rusqlite::{Connection, OptionalExtension, params};

use super::{Store, TermCache, alias, attach, local_term_space, read_model};
use crate::config::{
    EmbeddingConfig, GovernanceConfig, OwlConfig, ResolutionConfig, SearchConfig, ShaclConfig,
};
use crate::error::{Error, Result};
use crate::schema::INIT_SQL;
use crate::vector::VECTORS_SQL;

impl Store {
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
    ///   the mechanism, not the comment: a stray `INSERT` here fails at `SQLite`
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
        self.labels_config.clone_from(&other.labels_config);
        self.owl_config.clone_from(&other.owl_config);
        self.embedding_config.clone_from(&other.embedding_config);
        self.embedding_provider
            .clone_from(&other.embedding_provider);
    }

    /// Open a store with read-only databases mounted alongside it (quipu #75).
    ///
    /// The attachments contribute NAMED graphs. No existing query's result
    /// changes: the default dataset stays main-ROOT-alone, so an attachment is
    /// only visible to a query that names one of its graphs.
    ///
    /// # Errors
    /// [`Error::Store`] for an invalid alias, a duplicate alias, or an
    /// attachment quipu refuses to compose (no `g` column, or a term space
    /// that collides — see `attach::verify_attached_schema`).
    pub fn open_with_attachments(path: &str, attachments: &[attach::Attachment]) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init_with_attachments(conn, attachments)
    }

    pub(crate) fn init(conn: Connection) -> Result<Self> {
        Self::init_with_attachments(conn, &[])
    }

    fn init_with_attachments(conn: Connection, attachments: &[attach::Attachment]) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(INIT_SQL)?;
        conn.execute_batch(VECTORS_SQL)?;
        // ATTACH here, after INIT_SQL and before the migrations, per
        // multi-db-composition.md §2. Safe because every migration below uses
        // unqualified table names, which SQLite binds to `main` — measured, not
        // assumed; the table is in `attach`'s module docs.
        attach::attach_all(&conn, attachments)?;
        Self::migrate_named_graphs(&conn)?;
        // AFTER migrate_named_graphs: idx_geav must exist before the index it
        // makes redundant is dropped (quipu-fcg).
        Self::migrate_drop_eavt(&conn)?;
        // BEFORE `migrate_graph_labels`, which interns the meta-graph IRI: that
        // intern must know the store's space, or on a non-zero-space store it
        // allocates the reserved graph OUTSIDE the space that owns it.
        Self::migrate_term_spaces(&conn)?;
        Self::migrate_graph_labels(&conn)?;
        Self::migrate_datasets(&conn)?;
        Self::migrate_bitemporal_registries(&conn)?;
        Self::migrate_query_registry(&conn)?;
        Self::migrate_retraction_tx(&conn)?;
        // AFTER migrate_named_graphs: the fork registry references graphs(g).
        Self::migrate_forks(&conn)?;
        attach::migrate_graph_source(&conn)?;

        // Verification runs AFTER the migrations, not at attach time, because
        // the space-collision check needs the local store's own space — which
        // `migrate_term_spaces` is what establishes. Nothing has read the
        // attachment before this point.
        if !attachments.is_empty() {
            let local_space = local_term_space(&conn)?;
            attach::verify_attached_schema(&conn, attachments, local_space)?;
            attach::register_attached_graphs(&conn, attachments)?;
        }
        let pack_manifests = attach::attached_pack_manifests(&conn, attachments)?;

        // AFTER verification and registration: the source it builds is only
        // meaningful for attachments quipu has agreed to compose, and it reads
        // the same meta-graph id registration filtered on.
        let facts_source = attach::build_facts_source(&conn, attachments)?;
        let resolve_sql = attach::build_resolve_sql(attachments);
        // quipu #76: the alias table is TEMP and derived, so it is built here
        // rather than migrated — see `alias`'s module docs for why it is not
        // persisted. Safe on an unattached store: it creates an empty table.
        alias::build_term_alias(&conn, attachments)?;

        let mut store = Self::with_connection(conn);
        store.attachments = attachments.to_vec();
        store.pack_manifests = pack_manifests;
        store.facts_source = facts_source;
        store.resolve_sql = resolve_sql;
        Ok(store)
    }

    /// The SQL [`Self::resolve`] runs. Exposed for the test that pins it
    /// byte-identical on an unattached store.
    #[cfg(test)]
    pub(crate) fn resolve_sql(&self) -> &str {
        &self.resolve_sql
    }

    /// The table source a query reads `facts` from: `"facts"` for a store with
    /// no attachments, a `UNION ALL` over `main` and every attached layer
    /// otherwise (`multi-db-composition.md` §4).
    ///
    /// Interpolate this in place of the `facts` table name in a query that
    /// should see attached graphs. **Not for the write path or for local
    /// bookkeeping** — writes are `main`-only by design (§6), and a read that
    /// is deliberately ROOT-scoped cannot see an attached graph anyway, since
    /// attachments contribute only NAMED graphs.
    pub(crate) fn facts_source(&self) -> &str {
        &self.facts_source
    }

    /// The databases mounted alongside this store.
    #[must_use]
    pub fn attachments(&self) -> &[attach::Attachment] {
        &self.attachments
    }

    /// Knowledge-pack manifests contributed by attachments, paired with alias.
    #[must_use]
    pub fn pack_manifests(&self) -> &[(String, crate::pack::Manifest)] {
        &self.pack_manifests
    }

    /// Advisory embedding model/dimension mismatches for attached packs.
    #[must_use]
    pub fn pack_embedding_warnings(&self) -> Vec<String> {
        let consumer_model = self
            .embedding_config
            .model_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().to_string());
        self.pack_manifests.iter().filter_map(|(alias, manifest)| {
            let counts: serde_json::Value = serde_json::from_str(&manifest.counts).ok()?;
            let model = counts.get("embedding_model").and_then(|v| v.as_str()).map(str::to_string);
            let dim = counts.get("embedding_dimension").and_then(serde_json::Value::as_u64)
                .and_then(|n| usize::try_from(n).ok());
            if model != consumer_model || dim != Some(self.embedding_config.dimension) {
                Some(format!(
                    "attached pack {alias} embedding model/dimension {:?}/{:?} differs from consumer {:?}/{}; vectors are not converted",
                    model, dim, consumer_model, self.embedding_config.dimension
                ))
            } else { None }
        }).collect()
    }

    /// Recompute content hashes for every attached pack on explicit request.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn verify_attached_pack_hashes(&self) -> Result<Vec<(String, bool)>> {
        self.pack_manifests
            .iter()
            .map(|(alias, _)| {
                let attachment = self
                    .attachments
                    .iter()
                    .find(|a| &a.alias == alias)
                    .ok_or_else(|| {
                        Error::Store(format!("pack manifest has no attachment: {alias}"))
                    })?;
                let (_, _, ok) = crate::pack::verify(&attachment.path)?;
                Ok((alias.clone(), ok))
            })
            .collect()
    }

    /// Refuse a write aimed at a graph that lives in an attached database.
    ///
    /// **This is the only mechanism covering this case** — it reads like a
    /// third redundant layer and is not. `mode=ro` stops a write to the
    /// attached FILE, and unqualified table names route writes to `main`; but
    /// routing to `main` is precisely what makes this defect. Measured with
    /// this check removed: the write succeeds, one row lands in `main.facts`
    /// carrying the attached graph's id, the attached file is untouched, and
    /// every composed query then reads a local fact as though the layer had
    /// supplied it. Nothing errors anywhere.
    ///
    /// # Errors
    /// [`Error::Store`] naming the graph and the attachment that owns it.
    pub fn assert_graph_is_writable(&self, graph: i64) -> Result<()> {
        if graph == crate::schema::ROOT_GRAPH || self.attachments.is_empty() {
            return Ok(());
        }
        let source: Option<String> = self
            .conn
            .query_row(
                "SELECT source FROM main.graphs WHERE g = ?1",
                params![graph],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(alias) = source {
            return Err(Error::Store(format!(
                "refusing to write to graph {graph}: it belongs to the attached \
                 database {alias:?}, which is mounted read-only and is another \
                 owner's artifact. Write to a local graph instead."
            )));
        }
        Ok(())
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
            labels_config: crate::config::LabelsConfig::default(),
            shacl_config: ShaclConfig::default(),
            owl_config: OwlConfig::default(),
            #[cfg(feature = "owl")]
            owl_cache: None,
            governance_config: GovernanceConfig::default(),
            pending_write_events: std::cell::RefCell::new(Vec::new()),
            policy_registry: None,
            pending_verdicts: Vec::new(),
            pending_requests: Vec::new(),
            pending_refusal: None,
            principal_chain: Vec::new(),
            recording_verdicts: false,
            base_ns: crate::namespace::DEFAULT_BASE_NS.to_string(),
            vector_delegate: None,
            local_vector_backend: None,
            defer_auto_embed: false,
            pending_embed: None,
            attachments: Vec::new(),
            pack_manifests: Vec::new(),
            facts_source: "facts".to_string(),
            resolve_sql: attach::RESOLVE_SQL_LOCAL.to_string(),
            term_cache: std::cell::RefCell::new(TermCache::default()),
            read_model: std::cell::RefCell::new(std::collections::HashMap::new()),
            projected_graph: std::cell::RefCell::new(None),
            read_model_enabled: std::cell::Cell::new(true),
            write_in_progress: std::cell::Cell::new(false),
            read_model_max_triples: std::cell::Cell::new(
                read_model::DEFAULT_READ_MODEL_MAX_TRIPLES,
            ),
            #[cfg(feature = "reactive-reasoner")]
            observers: Vec::new(),
        }
    }

    /// Whether any database is attached — the predicate that decides whether
    /// `facts_source()` is a plain table and `canonical_id` is the identity.
    #[must_use]
    pub fn has_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }
}
