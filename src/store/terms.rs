//! Term dictionary — the `id <-> IRI` interning layer and its memo.
//!
//! Split out of `store/mod.rs` (quipu-yzf): interning, resolution, alias
//! canonicalisation and the [`TermCache`] are one responsibility, and they are
//! the layer every read path goes through to turn a term id back into
//! something an agent can read.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::Store;
use super::{Connection, intern_in_space, local_term_space};
use crate::error::{Error, Result};

/// Memoized `id <-> IRI` dictionary, populated on demand by [`Store::resolve`]
/// and [`Store::lookup`].
///
/// # Why a cache here needs no invalidation
///
/// `terms` is **append-only for an open store**. The only writes are the two
/// `INSERT OR IGNORE` statements in [`Store::intern`] and
/// [`Store::intern_with_id`] — no `UPDATE`, no `DELETE`, no `REPLACE`. The one
/// operation that rewrites `terms.id` is `respace_file`, and it `VACUUM INTO`s
/// a **copy** and rewrites the destination; the source database is never
/// touched (`src/store/respace.rs`). So a mapping this cache has already
/// observed cannot become wrong while the store is open.
///
/// That is the whole safety argument, and it is why this is a memo rather than
/// a coherence problem. If a future change makes `terms` mutable in place, this
/// cache becomes incorrect and must gain invalidation — hence the assertion
/// test `terms_table_is_append_only`.
///
/// # Only positive results are cached
///
/// A miss is NOT cached. [`Store::lookup`] returns `Option`, and an IRI that is
/// absent now can be interned by the very next write — caching the absence
/// would make `intern` invisible to a reader that had already asked.
#[derive(Debug, Default)]
pub(crate) struct TermCache {
    id_to_iri: HashMap<i64, String>,
    iri_to_id: HashMap<String, i64>,
}

impl TermCache {
    /// Record a mapping in both directions.
    fn insert(&mut self, id: i64, iri: &str) {
        self.id_to_iri.insert(id, iri.to_string());
        self.iri_to_id.insert(iri.to_string(), id);
    }

    /// Number of distinct terms memoized. Test and telemetry surface.
    pub(crate) fn len(&self) -> usize {
        self.id_to_iri.len()
    }
}

impl Store {
    pub fn intern(&self, iri: &str) -> Result<i64> {
        intern_in_space(&self.conn, iri)
    }

    /// The space this database allocates term ids from (quipu #74).
    ///
    /// Exactly one row carries `local = 1` (enforced by a partial unique index).
    /// A store with no such row predates the registry and is space 0 by
    /// definition — see `migrate_term_spaces`.
    ///
    /// # Errors
    /// Store errors reading the registry.
    pub fn local_term_space(&self) -> Result<i64> {
        local_term_space(&self.conn)
    }

    /// Seed the term-space registry (quipu #74).
    ///
    /// Idempotent and respace-safe: seeds `(0, 'main', 1)` only when the store
    /// has no local row at all. A store respaced into space 7 already has one,
    /// and re-seeding space 0 here would leave two rows claiming to be local —
    /// which is why this is not an `INSERT OR IGNORE` in `schema::INIT_SQL`.
    ///
    /// **A legacy store is space 0 by definition, so this is a one-row insert
    /// and never a rewrite.** Its ids are `1..n`, exactly the range space 0
    /// owns; that is the property the whole migration story rests on.
    pub(crate) fn migrate_term_spaces(conn: &Connection) -> Result<()> {
        let has_local: bool = conn
            .prepare("SELECT 1 FROM term_spaces WHERE local = 1")?
            .exists([])?;
        if !has_local {
            conn.execute(
                "INSERT INTO term_spaces (space, db, local) VALUES (0, 'main', 1)",
                [],
            )?;
        }
        Ok(())
    }

    /// Resolve a term id back to its IRI.
    ///
    /// With attachments mounted this also reads their `terms` tables — see
    /// [`Self::resolve_sql`] for why that is unambiguous, and why the
    /// *opposite* direction is deliberately not done here.
    /// Memoized via [`TermCache`]: the SPARQL binding path calls this once per
    /// result row per pattern, so the same few thousand terms were being
    /// re-fetched millions of times.
    pub fn resolve(&self, id: i64) -> Result<String> {
        // Scoped borrow: the SQL fetch below must not run while the cache is
        // borrowed, or a re-entrant resolve would panic.
        if let Some(iri) = self.term_cache.borrow().id_to_iri.get(&id) {
            return Ok(iri.clone());
        }
        let iri: String = self
            .conn
            .query_row(&self.resolve_sql, params![id], |row| row.get(0))
            .map_err(|_| Error::UnknownTerm(id))?;
        self.term_cache.borrow_mut().insert(id, &iri);
        Ok(iri)
    }

    /// Load the whole `id <-> IRI` dictionary in ONE statement instead of one
    /// per term, returning how many entries the cache holds afterwards.
    ///
    /// Callers that are about to resolve a large fraction of the store's terms
    /// (a full projection, a read-model build, a pack export) should warm the
    /// cache first: one sweep beats N round-trips as soon as N approaches the
    /// term count.
    ///
    /// Attachment branches are read too, and `INSERT`ed after main, so main
    /// wins a collision — matching [`attach::build_resolve_sql`], whose
    /// `UNION ALL … LIMIT 1` puts main first. `iri_to_id` is deliberately
    /// **main-only**, because [`Self::lookup`] is main-only; the one-IRI-to-many-ids
    /// direction belongs to [`Self::lookup_all`].
    ///
    /// # Errors
    /// [`Error::Sqlite`] if the sweep fails.
    pub fn warm_term_cache(&self) -> Result<usize> {
        let mut cache = self.term_cache.borrow_mut();
        let mut stmt = self.conn.prepare("SELECT id, iri FROM main.terms")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            cache.insert(row.get(0)?, &row.get::<_, String>(1)?);
        }
        // Attached terms resolve id -> IRI but must not claim the IRI -> id
        // direction, which `lookup` keeps local.
        for a in &self.attachments {
            let sql = format!("SELECT id, iri FROM {}.terms", a.alias);
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                cache
                    .id_to_iri
                    .entry(id)
                    .or_insert_with(|| row.get::<_, String>(1).unwrap_or_default());
            }
        }
        Ok(cache.len())
    }

    /// Every id this IRI denotes across the composition — the local id and
    /// each attached layer's id for the same IRI (quipu #76).
    ///
    /// The canonical (local) id comes FIRST when there is one, so a caller
    /// taking `.first()` gets the id the local store can resolve, label and
    /// write against.
    ///
    /// For a store with no attachments this returns exactly what
    /// [`Self::lookup`] returns, as zero or one element — and the predicate
    /// built from it is byte-identical to the pre-#76 SQL.
    ///
    /// # Errors
    /// [`Error::Sqlite`] if the lookup fails.
    pub fn lookup_all(&self, iri: &str) -> Result<Vec<i64>> {
        let local = self.lookup(iri)?;
        if self.attachments.is_empty() {
            return Ok(local.into_iter().collect());
        }
        let mut ids: Vec<i64> = local.into_iter().collect();
        for a in &self.attachments {
            let mut stmt = self
                .conn
                .prepare(&format!("SELECT id FROM {}.terms WHERE iri = ?1", a.alias))?;
            let mut rows = stmt.query(params![iri])?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }

    /// Map an id to the one the LOCAL store uses for the same IRI.
    ///
    /// An id with no alias row — every id on an unattached store, and every
    /// local id — is returned unchanged, so this is an identity function until
    /// a composition actually contains an alias.
    ///
    /// This is the *"sharp edge"* of `multi-db-composition.md` §1.2. SQL
    /// `DISTINCT` runs over raw ids, so an entity present in both files
    /// survives it as two rows that are semantically one. Canonicalising in
    /// Rust **after** the SQL DISTINCT is what collapses them, and it costs
    /// the size of the result set rather than the size of the store.
    ///
    /// # Errors
    /// [`Error::Sqlite`] if the lookup fails.
    pub fn canonical_id(&self, id: i64) -> Result<i64> {
        if self.attachments.is_empty() {
            return Ok(id);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT canonical_id FROM term_alias WHERE alias_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(id))
    }

    /// Look up a term id by IRI, returning None if not interned.
    ///
    /// Memoized via [`TermCache`], but **only on a hit**: an absent IRI can be
    /// interned by the next write, so caching the `None` would hide it.
    pub fn lookup(&self, iri: &str) -> Result<Option<i64>> {
        if let Some(id) = self.term_cache.borrow().iri_to_id.get(iri) {
            return Ok(Some(*id));
        }
        let mut stmt = self.conn.prepare("SELECT id FROM terms WHERE iri = ?1")?;
        let mut rows = stmt.query(params![iri])?;
        let found = match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        };
        drop(rows);
        drop(stmt);
        if let Some(id) = found {
            self.term_cache.borrow_mut().insert(id, iri);
        }
        Ok(found)
    }
}
