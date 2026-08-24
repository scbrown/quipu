//! Schema migrations, run in order on every writable open.
//!
//! Split from `mod.rs` (quipu-bu3). Each migration is idempotent and
//! additive unless its doc says otherwise; `init_with_attachments` (see
//! `open`) owns the calling order, which is load-bearing — see its comments.

use rusqlite::{Connection, params};

use super::{Store, intern_in_space};
use crate::error::Result;

impl Store {
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
    pub(super) fn migrate_named_graphs(conn: &Connection) -> Result<()> {
        let has_g: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('facts') WHERE name = 'g'")?
            .exists([])?;
        if !has_g {
            conn.execute_batch("ALTER TABLE facts ADD COLUMN g INTEGER NOT NULL DEFAULT 0;")?;
        }
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_geav ON facts(g, e, a, v);")?;
        Ok(())
    }

    /// Drop the redundant `idx_eavt` permutation (quipu-fcg).
    ///
    /// Once `idx_geav (g, e, a, v)` exists (the migration above), `idx_eavt
    /// (e, a, v, valid_from)` is never the plan: every hot read path binds `g`
    /// alongside `e` — SPARQL pushes a graph condition on every triple pattern,
    /// and the direct read paths are ROOT-scoped (quipu #56) — and the one
    /// e-only probe (the event log's prior-fact check) is a covering lookup on
    /// the `(e, a, v, tx)` PK autoindex. Measured at 10k episodes (83MB store):
    /// every representative plan identical without it, and the file is 14.1MB
    /// (16.9%) smaller. Verified against the full suite.
    ///
    /// Runs AFTER `migrate_named_graphs`, which is what guarantees `idx_geav`
    /// exists before the fallback it provides is removed. Idempotent.
    pub(super) fn migrate_drop_eavt(conn: &Connection) -> Result<()> {
        conn.execute_batch("DROP INDEX IF EXISTS idx_eavt;")?;
        Ok(())
    }

    /// Additive migration for graph labels (quipu #65). Adds the five nullable
    /// cache columns to `graphs` and reserves the label meta-graph.
    ///
    /// Follows `migrate_named_graphs` above deliberately, including the
    /// aegis-akb8 hazard: any index on the new columns is created **here**, not
    /// in `schema::INIT_SQL`, because `INIT_SQL` runs first and against
    /// pre-label stores, where `CREATE INDEX … ON graphs(trust_chain)` would
    /// hard-fail with `no such column` before this ALTER could add it.
    ///
    /// **The meta-graph is seeded here and could not be seeded in `INIT_SQL`.**
    /// ROOT is `g = 0`, a constant an `INSERT` can hardcode; the meta-graph's
    /// `g` is `intern("urn:quipu:graph:meta")` — a *runtime rowid* that depends
    /// on what the store has already interned. It has to be looked up.
    ///
    /// Idempotent, and additive in the strict sense: the columns are nullable
    /// with no default, so every existing graph reads back *undeclared* rather
    /// than a fabricated label, and the reserved meta-graph starts with zero
    /// facts — so no query's results change. `created_at` is the same fixed
    /// epoch ROOT uses rather than a clock read, so the migration is
    /// deterministic and fixtures stay stable.
    pub(super) fn migrate_graph_labels(conn: &Connection) -> Result<()> {
        // Five nullable cache columns, each guarded independently: a store part
        // way through an interrupted migration must converge, not fail.
        //
        // `trust_chain` is TEXT holding the chain's IRI, deliberately NOT an
        // interned term id. Term ids are exactly what quipu #74's `respace`
        // must rewrite, and every term-id-bearing column added here widens that
        // surface — a respace that misses one does not error, it silently
        // repoints the registry. Text costs a few bytes and stays out of it.
        for (col, decl) in [
            ("fresh_rank", "INTEGER"),
            ("durability_rank", "INTEGER"),
            ("trust_rank", "INTEGER"),
            ("trust_chain", "TEXT"),
            ("policy", "TEXT"),
            ("labels_tx", "INTEGER"),
            ("labels_valid_to", "TEXT"),
            // The dataKind axis and the freeze lifecycle. TEXT tokens, not
            // interned ids, for the same respace reason as `trust_chain`.
            ("data_kind", "TEXT"),
            ("lifecycle", "TEXT"),
        ] {
            let present: bool = conn
                .prepare("SELECT 1 FROM pragma_table_info('graphs') WHERE name = ?1")?
                .exists(params![col])?;
            if !present {
                conn.execute_batch(&format!("ALTER TABLE graphs ADD COLUMN {col} {decl};"))?;
            }
        }

        // Reserve the meta-graph. Interning is the same INSERT-OR-IGNORE then
        // SELECT that `Store::intern` does; done in SQL because this runs
        // against a bare `Connection`, before a `Store` exists.
        let meta_g: i64 = intern_in_space(conn, crate::namespace::META_GRAPH_IRI)?;
        // `committed`, not `overlay`: labels are durable and bitemporal, and
        // overlay-class graphs are excluded from bitemporality by design.
        // Self-rooted like ROOT, so it is never resolved against a parent.
        conn.execute(
            "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at) \
             VALUES (?1, 'committed', NULL, '1970-01-01T00:00:00Z')",
            params![meta_g],
        )?;

        Ok(())
    }

    /// Record which transaction RETRACTED a fact (quipu #83).
    ///
    /// `as_of_tx = N` is meant to answer "what did the store know as of
    /// transaction N?". It could not: retraction sets `valid_to` to a
    /// TIMESTAMP and leaves the row's original `tx` untouched, so no retraction
    /// transaction was recorded anywhere — while the as-of-tx query still
    /// required the row be live NOW. A fact live at N but retracted since was
    /// invisible at EVERY N, and silently: a smaller answer, never an error.
    ///
    /// Additive and nullable. **Deliberately NOT backfilled**, because the
    /// information to backfill it with was never recorded — that is the whole
    /// defect. A legacy row closed before this migration has `valid_to` set and
    /// `retracted_tx` NULL, and the as-of predicate
    /// (`valid_to IS NULL OR retracted_tx > N`) leaves it invisible exactly as
    /// it is today. So existing stores see NO behaviour change; only retractions
    /// made from here on become time-travelable. Inventing a plausible
    /// `retracted_tx` for legacy rows would make them visible at windows they
    /// may not have been live in, which is a worse answer than the honest gap.
    pub(super) fn migrate_retraction_tx(conn: &Connection) -> Result<()> {
        let present: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('facts') WHERE name = 'retracted_tx'")?
            .exists([])?;
        if !present {
            conn.execute_batch("ALTER TABLE facts ADD COLUMN retracted_tx INTEGER;")?;
        }
        Ok(())
    }

    /// Stored named-query registry (quipu #79).
    ///
    /// Versioned in #71's close-don't-overwrite style **from day one** rather
    /// than retrofitted: a query definition is policy about how a layer is
    /// read, and losing the prior version loses the answer to "what did this
    /// name mean when that result was produced".
    ///
    /// `dataset` is the optional scope (NULL = global). Stored as the dataset
    /// IRI, not a term id — the #74 respace rule again.
    pub(super) fn migrate_query_registry(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS queries (
                 name        TEXT    NOT NULL,
                 description TEXT    NOT NULL,
                 template    TEXT    NOT NULL,
                 dataset     TEXT,
                 valid_from  TEXT    NOT NULL,
                 valid_to    TEXT,
                 tx          INTEGER,
                 PRIMARY KEY (name, valid_from)
             );
             CREATE INDEX IF NOT EXISTS idx_queries_open ON queries(name, valid_to);
             -- `ord` keeps the display order the ParamSpec array has; the params
             -- of a query are a LIST, and a set would silently reorder a
             -- self-describing catalog between reads.
             CREATE TABLE IF NOT EXISTS query_params (
                 name        TEXT    NOT NULL,
                 valid_from  TEXT    NOT NULL,
                 ord         INTEGER NOT NULL,
                 param       TEXT    NOT NULL,
                 kind        TEXT    NOT NULL CHECK (kind IN ('iri','text','int')),
                 required    INTEGER NOT NULL,
                 default_val TEXT,
                 description TEXT    NOT NULL,
                 PRIMARY KEY (name, valid_from, ord)
             );",
        )?;
        Ok(())
    }

    /// Bitemporal migration for `shapes` and `ontologies` (quipu #71) —
    /// **close, don't overwrite**.
    ///
    /// Both tables were `name PRIMARY KEY` + `INSERT OR REPLACE`, which
    /// discards history: the audit spine had no record that the rules changed,
    /// and `proposals` already *held* that history only for proposal-routed
    /// loads. This gives them the discipline `facts` already has — a load
    /// CLOSES the prior row, `remove` closes rather than deletes.
    ///
    /// ⚠️ **This is the one migration here that rebuilds a table.** Multiple
    /// rows per name means the primary key must change, and `SQLite` cannot alter
    /// a PK in place. Guarded on the presence of `valid_from` so it runs once;
    /// wrapped in a savepoint so a failure leaves the old table intact; and
    /// safe to drop because **nothing references either table by foreign key**
    /// (checked) and every read/write site is in this file.
    ///
    /// Existing rows migrate to `valid_from = loaded_at` with an open
    /// `valid_to`, so `list_shapes` — which now filters to open rows — returns
    /// exactly what it returned before.
    pub(super) fn migrate_bitemporal_registries(conn: &Connection) -> Result<()> {
        for table in ["shapes", "ontologies"] {
            let migrated: bool = conn
                .prepare(&format!(
                    "SELECT 1 FROM pragma_table_info('{table}') WHERE name = 'valid_from'"
                ))?
                .exists([])?;
            if migrated {
                continue;
            }
            conn.execute_batch(&format!(
                "SAVEPOINT quipu_bitemporal_{table};
                 CREATE TABLE {table}_bt (
                     name       TEXT    NOT NULL,
                     turtle     TEXT    NOT NULL,
                     loaded_at  TEXT    NOT NULL,
                     valid_from TEXT    NOT NULL,
                     valid_to   TEXT,
                     tx         INTEGER,
                     PRIMARY KEY (name, valid_from)
                 );
                 INSERT INTO {table}_bt (name, turtle, loaded_at, valid_from, valid_to, tx)
                     SELECT name, turtle, loaded_at, loaded_at, NULL, NULL FROM {table};
                 DROP TABLE {table};
                 ALTER TABLE {table}_bt RENAME TO {table};
                 CREATE INDEX IF NOT EXISTS idx_{table}_open ON {table}(name, valid_to);
                 RELEASE quipu_bitemporal_{table};"
            ))?;
        }
        Ok(())
    }

    /// Additive migration for named datasets (quipu #69) — a *name* for an
    /// arbitrary set of graphs, so it can be reused, labelled and governed.
    ///
    /// **`parent_branch` is deliberately untouched.** The branch tree is not a
    /// taxonomy; it is `compose_view`'s resolution root, bind-once so an
    /// overlay cannot forge presence in a base it was never bound to. Datasets
    /// and the branch tree are different relations over the same node set, and
    /// conflating them is the failure Alexander's essay names.
    ///
    /// **Members are stored as graph IRIs (TEXT), not interned term ids** —
    /// following the rule proposed in the quipu #74 acceptance amendment: do
    /// not put a term id in a new column unless you need term identity. Every
    /// term-id-bearing column widens the surface `respace` must rewrite, and a
    /// respace that misses one does not error, it silently repoints the
    /// registry. Resolution does a `lookup` per member, which `apply_dataset`
    /// already does for every `FROM` IRI anyway.
    /// Fork registry (quipu-gp5) — persistent named forks of ROOT at a pinned
    /// transaction.
    ///
    /// `g` is the fork graph's id (a term id — classified `Id` in
    /// `respace_map`); `parent_branch` follows `graphs.parent_branch`'s
    /// classification, though v1 only ever writes ROOT (`0`, the exempt
    /// sentinel). `fork_tx` is a TRANSACTION id, like `facts.tx` — an integer
    /// that looks exactly like a term id and is not. Rows are never deleted:
    /// `dropped` and `promoted` are terminal statuses, because a fork that
    /// existed is a fact about the past (the `dataset_remove` precedent).
    pub(super) fn migrate_forks(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS forks (
                 name          TEXT PRIMARY KEY,
                 g             INTEGER NOT NULL REFERENCES graphs(g),
                 parent_branch INTEGER NOT NULL,
                 fork_tx       INTEGER NOT NULL,
                 created_at    TEXT NOT NULL,
                 status        TEXT NOT NULL
                     CHECK (status IN ('open','promoted','dropped'))
             );",
        )?;
        Ok(())
    }

    pub(super) fn migrate_datasets(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS datasets (
                 name       TEXT PRIMARY KEY,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS dataset_members (
                 dataset   TEXT NOT NULL REFERENCES datasets(name) ON DELETE CASCADE,
                 graph_iri TEXT NOT NULL,
                 ord       INTEGER,
                 PRIMARY KEY (dataset, graph_iri)
             );
             -- A declared ordering must be unambiguous: two members at the same
             -- rank is a silent tiebreak waiting to happen, and a silent
             -- tiebreak is how 'learned tactic beats canonical' ships. NULL ord
             -- (an unordered dataset) is exempt — SQLite's unique index treats
             -- NULLs as distinct, which is exactly the semantics wanted here.
             CREATE UNIQUE INDEX IF NOT EXISTS idx_dataset_ord
                 ON dataset_members(dataset, ord);",
        )?;
        Ok(())
    }
}
