//! The store as the handlers see it — writer lock, read pool, and the
//! request-time config the handlers need (split from `server.rs`, quipu-tkh).

use std::sync::Arc;

use parking_lot::FairMutex;

/// FAIR (FIFO) mutex on purpose: std's Mutex is unfair, so a
/// sustained stream of episode writers could re-acquire the lock ahead of
/// readers indefinitely — during the mfg0 incident a `SELECT ... LIMIT 1`
/// measured a 38.5s wait behind a write flood. `FairMutex` hands the lock to
/// the longest waiter, bounding every request's wait to the queue ahead of
/// it. (`parking_lot`'s `lock()` has no poison Result — a panic while holding
/// the lock simply unlocks, which is fine: Store keeps its invariants in
/// `SQLite` transactions, not in Rust-visible state.)
pub(crate) type SharedStore = Arc<StoreHandle>;

/// The store as the handlers see it: ONE writer connection, plus a pool of
/// read-only connections.
///
/// WAL already permits N concurrent readers alongside one writer. Before this,
/// every read took the writer's mutex, so `SQLite`'s concurrency was present and
/// unused — measured at effective parallelism **1.0** for N = 1, 2, 4, 8 on a
/// quiet store, i.e. 8 concurrent queries cost 8x one query's wall time
/// the pre-pool measurement this replaced.
///
/// `lock()` keeps its exact former meaning — the writer, behind the same
/// `FairMutex` — so every existing call site is unchanged and writes are still
/// serialised. `read()` is the new path.
pub(crate) struct StoreHandle {
    pub(crate) writer: FairMutex<quipu::Store>,
    pub(crate) readers: ReadPool,
    /// Read-pool stores share the built-in `SQLite` vectors table, but cannot
    /// clone boxed external/LanceDB backends. Keep those searches on the writer
    /// so moving `SQLite` search to WAL readers never changes backend semantics.
    pub(crate) vector_reads_pooled: bool,
    /// The configured federation remotes, held for the per-request federated
    /// query path (quipu-tkh). Empty means `federated: true` fans out to the
    /// local store alone.
    pub(crate) federation: quipu::config::FederationConfig,
    /// The registered reactive reasoner, kept concrete (not as the
    /// `dyn TransactObserver` the store holds) so `POST /shapes` can hot-swap
    /// its ruleset — quipu-923, gap G6: without this handle, rules loaded at
    /// runtime needed a server restart to take effect.
    #[cfg(feature = "reactive-reasoner")]
    pub(crate) reasoner: Option<Arc<quipu::ReactiveReasoner>>,
}

/// A fixed set of read-only connections, each owned exclusively while in use.
///
/// `rusqlite::Connection` is `Send` but **not** `Sync`, so this cannot be an
/// `RwLock` over one connection however much the access pattern looks like one:
/// the shape has to be N connections, not a smarter lock over one.
///
/// **What happens when every connection is busy** — the question the design has
/// to answer out loud, because it is where the starvation the `FairMutex` was
/// introduced to bound would come back wearing a new name:
///
/// 1. Fast path: take any connection that is free right now. Work-conserving —
///    a reader never queues while a connection sits idle.
/// 2. Otherwise: queue on ONE connection chosen round-robin, and wait on its
///    `FairMutex`, which is FIFO. So a reader's wait is bounded by the queue on
///    its own connection — roughly 1/N of today's single queue — and it cannot
///    be starved indefinitely by later arrivals.
///
/// The writer keeps its own `FairMutex` and is never in this pool, so the mfg0
/// case that motivated fairness (a `SELECT ... LIMIT 1` measured waiting 38.5s
/// behind a write flood) improves strictly: readers leave the writer's queue
/// entirely rather than sharing it more politely.
///
/// An EMPTY pool is a supported configuration, not a degenerate one: `read()`
/// falls back to the writer lock, which is exactly today's behaviour. In-memory
/// stores take that path because each `:memory:` connection would be a
/// different, empty database — a pool there would not be slow, it would be
/// wrong.
pub(crate) struct ReadPool {
    pub(crate) conns: Vec<FairMutex<quipu::Store>>,
    pub(crate) next: std::sync::atomic::AtomicUsize,
}

impl ReadPool {
    pub(crate) fn empty() -> Self {
        Self {
            conns: Vec::new(),
            next: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.conns.len()
    }
}

impl StoreHandle {
    /// A handle with NO read pool: every read takes the writer lock, which is
    /// the pre-pool behaviour. Used by the in-memory server tests, where a pool
    /// is not merely unhelpful but wrong — each `:memory:` connection would be
    /// its own empty database.
    #[cfg(test)]
    pub(crate) fn writer_only(store: quipu::Store) -> Self {
        Self {
            vector_reads_pooled: store.has_sqlite_vector_backend(),
            writer: FairMutex::new(store),
            readers: ReadPool::empty(),
            federation: quipu::config::FederationConfig::default(),
            #[cfg(feature = "reactive-reasoner")]
            reasoner: None,
        }
    }

    /// The WRITER connection. Unchanged semantics: one at a time, FIFO-fair.
    /// Every pre-existing `.lock()` call site means exactly what it meant
    /// before, which is why this refactor does not have to audit them.
    pub(crate) fn lock(&self) -> parking_lot::FairMutexGuard<'_, quipu::Store> {
        self.writer.lock()
    }

    /// A READ connection from the pool, or the writer when the pool is empty.
    ///
    /// Only call this where the work is genuinely read-only: the connection is
    /// opened `SQLITE_OPEN_READ_ONLY` with `PRAGMA query_only`, so a write
    /// attempted through it fails at `SQLite` rather than corrupting anything —
    /// but failing a request is still a bug, and the borrow checker cannot
    /// catch it here the way `&Store` vs `&mut Store` does in the tool layer.
    pub(crate) fn read(&self) -> parking_lot::FairMutexGuard<'_, quipu::Store> {
        let mut guard = if self.readers.conns.is_empty() {
            self.writer.lock()
        } else if let Some(g) = self.readers.conns.iter().find_map(FairMutex::try_lock) {
            // Work-conserving fast path.
            g
        } else {
            // All busy: queue FIFO on one connection. Relaxed is right — this
            // is a load-spreading hint, not a synchronisation edge.
            let i = self
                .readers
                .next
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                % self.readers.conns.len();
            self.readers.conns[i].lock()
        };
        // Deep freeze: a pooled reader opened before a freeze has no pack
        // attached, and its `facts_source` is plain "facts" — a query over
        // the frozen graph would silently read zero rows. Sync against the
        // frozen-pack registry on every acquisition (one indexed SELECT when
        // nothing changed). A failed sync is reported on stderr and the read
        // proceeds against main only — the archive graphs then read as
        // absent from THIS request, which the frozen registry rows at least
        // make diagnosable, and refusing every read for one bad pack file
        // would take the whole store down with it.
        if let Err(e) = guard.sync_frozen_attachments() {
            eprintln!(
                "{} read-pool frozen-pack sync failed: {e}",
                quipu::time::now_iso()
            );
        }
        guard
    }

    /// A read connection that preserves the configured vector backend.
    pub(crate) fn vector_read(&self) -> parking_lot::FairMutexGuard<'_, quipu::Store> {
        if self.vector_reads_pooled {
            self.read()
        } else {
            self.lock()
        }
    }
}
