//! Term aliases across term spaces (quipu #76, Track B step 4).
//!
//! The same IRI interned independently in two databases gets two ids. That is
//! an **alias**, not a collision — see `docs/design/multi-db-composition.md`
//! §1.2. Term spaces (#74) guarantee the ids differ; nothing yet made them
//! mean the same thing.
//!
//! # What breaks without this, measured before writing it
//!
//! On a two-store composition where both files intern `urn:shared:entity`:
//!
//! ```text
//! same IRI  ->  local id 2 ,  attached id 23089744183298
//! SELECT ?s ?lo ?go WHERE { ?s <local> ?lo . GRAPH ?g { ?s <layer> ?go } }
//!   -> 0 ROWS
//! ```
//!
//! The join is between two ids that denote one entity, so nothing matches and
//! **nothing errors** — the composed store is mountable and not queryable
//! across. #75 delivered the mount; this delivers the join.
//!
//! # Why the alias table is TEMP, deviating from the design's DDL
//!
//! §1.2 writes `CREATE TABLE term_alias`. This creates it as a **`TEMP`**
//! table instead, deliberately:
//!
//! - It is **derived state**, recomputable from the attachments in one join,
//!   and it is rebuilt on every open. Persisting it buys nothing and creates a
//!   copy that is wrong the moment the attachment set changes — a store opened
//!   once with a layer would carry that layer's aliases forever after.
//! - It holds **term ids in two columns**, so persisting it would widen the
//!   surface `quipu db respace` has to rewrite. #74's acceptance amendment
//!   exists because every term-id-bearing column added so far has been missed
//!   by somebody; the cheapest way to not be missed is to not be on disk.
//!   (Same reasoning that put `graphs.source` in as TEXT rather than an id.)
//!
//! A `TEMP` table lives for the connection, which is exactly the lifetime of
//! the composition it describes.

use rusqlite::Connection;

use crate::error::Result;

use super::attach::Attachment;

/// The alias table. `TEMP` — see the module docs for why.
///
/// `canonical_id` is the LOCAL (`main`) id; `alias_id` is the attached one.
/// The direction matters: canonicalisation always resolves *towards* `main`,
/// so a query's bindings are expressed in the ids the local store can resolve,
/// label and write against.
const ALIAS_SQL: &str = "\
CREATE TEMP TABLE IF NOT EXISTS term_alias (
    canonical_id INTEGER NOT NULL,
    alias_id     INTEGER NOT NULL PRIMARY KEY
);
CREATE INDEX IF NOT EXISTS temp.idx_term_alias_canonical ON term_alias(canonical_id);
";

/// Build the alias table for this composition.
///
/// Populated by joining `main.terms` against each attachment's `terms` on the
/// IRI — bounded by the vocabulary the two files SHARE, which for the intended
/// deployment (a shared reference layer beside a per-tenant store) is hundreds
/// to thousands of rows, not the size of either store.
///
/// `alias_id` is the primary key: one attached id has exactly one canonical
/// meaning. Two different attachments both interning the same IRI each get
/// their own row, all pointing at the one local canonical id.
///
/// Rebuilt from empty every time, because it describes the attachment set as
/// it is now.
///
/// # Errors
/// [`Error::Sqlite`](crate::error::Error::Sqlite) if the table cannot be built.
pub(crate) fn build_term_alias(conn: &Connection, attachments: &[Attachment]) -> Result<usize> {
    conn.execute_batch(ALIAS_SQL)?;
    conn.execute("DELETE FROM term_alias", [])?;
    if attachments.is_empty() {
        return Ok(0);
    }
    let mut n = 0;
    for a in attachments {
        let alias = &a.alias;
        n += conn.execute(
            &format!(
                "INSERT OR IGNORE INTO term_alias (canonical_id, alias_id) \
                 SELECT l.id, r.id FROM main.terms l \
                 JOIN {alias}.terms r ON r.iri = l.iri \
                 WHERE l.id <> r.id"
            ),
            [],
        )?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests;
