//! `ATTACH`ed read-only layers (quipu #75, Track B step 3).
//!
//! An attached database contributes **named graphs**, not a fourth axis. Term
//! spaces (quipu #74) make its ids already valid locally, so every existing
//! `FROM` / `GRAPH` / dataset / label mechanism works on it unchanged — see
//! `docs/design/multi-db-composition.md` §3.
//!
//! # Three properties this module exists to hold
//!
//! 1. **Attaching changes no existing query's result.** Attachments add named
//!    graphs; the default dataset stays main-ROOT-alone. Silence must not
//!    widen the dataset — the same rule named-graphs.md §4 settled.
//! 2. **Quipu never writes to an attached database, and never writes a local
//!    fact INTO an attached graph.** Those are two different claims and only
//!    the first is structural. `mode=ro` and unqualified (`main`) table names
//!    protect the attached FILE — measured, and they do. They do nothing about
//!    the second: with `Store::assert_graph_is_writable` removed, a write
//!    aimed at an attached graph SUCCEEDS, landing a row in `main.facts`
//!    tagged with that graph's id while the attached file stays byte-for-byte
//!    unchanged. Every composed query then reads that local row as if the
//!    layer had supplied it, and nothing errors. So the Rust guard is not
//!    belt-and-braces; for that case it is the only mechanism there is.
//! 3. **Attached databases are never migrated.** They are verified and
//!    refused, never fixed in place.
//!
//! # Why `ATTACH` before the migrations is safe — measured, not assumed
//!
//! The design puts `ATTACH` immediately after `INIT_SQL` and before the
//! `migrate_*` functions, which raises an obvious question: those migrations
//! use *unqualified* table names, and the attachment also has a `facts` table.
//! Measured on `SQLite` before relying on it:
//!
//! | statement | binds to |
//! |---|---|
//! | `pragma_table_info('facts')` unqualified | `main` |
//! | `ALTER TABLE facts ADD COLUMN …` unqualified | `main` only; the attachment is untouched |
//! | `CREATE TABLE IF NOT EXISTS x` when only the attachment has `x` | creates it in `main` |
//! | `INSERT` into a `file:…?mode=ro` attachment | refused, `attempt to write a readonly database` |
//!
//! The last row is the one worth having in writing: read-only attach is
//! enforced by `SQLite`, not by our care.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::schema::SPACE_SIZE;

/// One database to mount alongside the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// `SQLite` schema name. Validated `^[a-z][a-z0-9_]*$` — a schema alias is
    /// a name, not an expression, so it cannot be a bound parameter and is
    /// interpolated into SQL.
    pub alias: String,
    /// Path to the database file.
    pub path: String,
    /// Mount read-only. Defaults to true via [`Attachment::read_only`], and
    /// there is currently no supported reason to set it false — the field
    /// exists because the design names it, and a writable attachment would
    /// need every guarantee in this module re-argued.
    pub read_only: bool,
}

impl Attachment {
    /// A read-only attachment — the only kind quipu composes today.
    #[must_use]
    pub fn read_only(alias: &str, path: &str) -> Self {
        Self {
            alias: alias.to_string(),
            path: path.to_string(),
            read_only: true,
        }
    }
}

/// `SQLite` schema names quipu can never use as an alias.
const RESERVED_ALIASES: &[&str] = &["main", "temp"];

/// Validate a schema alias.
///
/// # Errors
/// [`Error::Store`] if the alias is empty, reserved, or not `^[a-z][a-z0-9_]*$`.
pub fn validate_alias(alias: &str) -> Result<()> {
    let ok = !alias.is_empty()
        && alias.starts_with(|c: char| c.is_ascii_lowercase())
        && alias
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        return Err(Error::Store(format!(
            "invalid attachment alias {alias:?}: must match ^[a-z][a-z0-9_]*$. \
             A schema alias is interpolated into SQL because SQLite cannot bind \
             a parameter to a schema name, so it is validated rather than quoted."
        )));
    }
    if RESERVED_ALIASES.contains(&alias) {
        return Err(Error::Store(format!(
            "invalid attachment alias {alias:?}: reserved by SQLite"
        )));
    }
    Ok(())
}

/// `ATTACH` every attachment. Runs after `INIT_SQL`, before the migrations.
///
/// # Errors
/// [`Error::Store`] for an invalid or duplicated alias; [`Error::Sqlite`] if a
/// file cannot be attached.
pub(crate) fn attach_all(conn: &Connection, attachments: &[Attachment]) -> Result<()> {
    let mut seen: Vec<&str> = Vec::new();
    for a in attachments {
        validate_alias(&a.alias)?;
        if seen.contains(&a.alias.as_str()) {
            return Err(Error::Store(format!(
                "attachment alias {:?} is used twice — each attached database \
                 needs its own schema name",
                a.alias
            )));
        }
        seen.push(&a.alias);

        // `mode=ro` is what actually makes it read-only: `PRAGMA query_only` is
        // connection-wide and cannot be scoped to one attachment.
        let uri = if a.read_only {
            format!("file:{}?mode=ro", a.path)
        } else {
            format!("file:{}", a.path)
        };
        conn.execute(&format!("ATTACH DATABASE ?1 AS {}", a.alias), params![uri])?;
    }
    Ok(())
}

/// The term space an attached database owns.
///
/// Absence reads as space 0, exactly as it does for the local store: a
/// database with no registry predates it and its ids are `1..n`.
fn attached_term_space(conn: &Connection, alias: &str) -> Result<i64> {
    let has_table: bool = conn
        .prepare(&format!(
            "SELECT 1 FROM {alias}.sqlite_master WHERE type='table' AND name='term_spaces'"
        ))?
        .exists([])?;
    if !has_table {
        return Ok(0);
    }
    let space: Option<i64> = conn
        .query_row(
            &format!("SELECT space FROM {alias}.term_spaces WHERE local = 1"),
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(space.unwrap_or(0))
}

/// Refuse an attachment quipu cannot compose, naming the fix.
///
/// Two refusals, both **before** anything reads the attachment:
///
/// - its `facts` has no `g` column, so it predates named graphs and has no
///   notion of the graph a composed query selects on;
/// - its term space collides with the local store's or with another
///   attachment's, so ids from the two files mean different things.
///
/// The collision case is quipu #74's acceptance 4 — *"two space-0 DBs in one
/// composition → refused with a message naming `respace`"* — and it is
/// enforced here rather than in #74 because this is the first point at which a
/// composition exists.
///
/// # Errors
/// [`Error::Store`] naming the attachment and the remedy.
pub(crate) fn verify_attached_schema(
    conn: &Connection,
    attachments: &[Attachment],
    local_space: i64,
) -> Result<()> {
    // (space, who owns it) — main first, so a collision message can always name
    // the incumbent.
    let mut claimed: Vec<(i64, String)> = vec![(local_space, "the local store".to_string())];

    for a in attachments {
        let alias = &a.alias;
        let has_g: bool = conn
            .prepare(&format!(
                "SELECT 1 FROM pragma_table_info('facts', '{alias}') WHERE name = 'g'"
            ))?
            .exists([])?;
        if !has_g {
            return Err(Error::Store(format!(
                "cannot attach {:?} as {alias}: its `facts` table has no `g` \
                 column, so it predates named graphs and a composed query has no \
                 graph to select on. Open it once with a current quipu to migrate \
                 it, then attach it.",
                a.path
            )));
        }

        let space = attached_term_space(conn, alias)?;
        if let Some((_, incumbent)) = claimed.iter().find(|(s, _)| *s == space) {
            let lo = space * SPACE_SIZE;
            let hi = lo + SPACE_SIZE;
            let path = &a.path;
            return Err(Error::Store(format!(
                "cannot attach {path:?} as {alias}: it owns term space {space} \
                 [{lo}, {hi}), and so does {incumbent}. Two databases in one \
                 composition cannot share a space — their ids would mean \
                 different things while looking identical. Move one into a free \
                 space first:\n    quipu db respace --into <space> --out <new-file> \
                 --db {path}\nand attach the new file."
            )));
        }
        claimed.push((space, format!("attachment {alias}")));
    }
    Ok(())
}

/// Additive migration for attachment provenance (quipu #75): `graphs.source`.
///
/// Names the attachment alias a graph came from; NULL means local. **Not a
/// foreign key** — a foreign key cannot span attached databases, and there is
/// nothing local for it to reference anyway.
///
/// TEXT holding the alias, not a term id, following the rule quipu #74's
/// acceptance amendment set: do not put a term id in a new column unless you
/// need term identity. Every one added widens the surface `respace` must
/// rewrite.
///
/// Guarded on `pragma_table_info` and idempotent, in the same shape as
/// `migrate_named_graphs` — including the aegis-akb8 hazard, which is why any
/// index on the new column would be created here rather than in `INIT_SQL`.
pub(crate) fn migrate_graph_source(conn: &Connection) -> Result<()> {
    let present: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('graphs') WHERE name = 'source'")?
        .exists([])?;
    if !present {
        conn.execute_batch("ALTER TABLE graphs ADD COLUMN source TEXT;")?;
    }
    Ok(())
}

/// Register every graph an attachment contributes into `main.graphs`.
///
/// One registry, so `GRAPH <iri>` resolution and graph labels are uniform over
/// local and attached graphs alike (§3). The attached row is copied, not
/// referenced: `main.graphs` is the only registry a query consults.
///
/// **Reserved graphs are per-database and are NOT contributed.** ROOT (`g = 0`)
/// and the label meta-graph both mean "this database's own X". Copying either
/// gives the local store a second row claiming to be its own reserved graph —
/// a row that resolves to nothing locally, because every lookup of ROOT or the
/// meta-graph goes through the LOCAL registry. An attachment contributes its
/// *named* graphs. (Found by a test expecting one attached graph and getting
/// two: the second was the attachment's meta-graph.)
///
/// **Graph labels travel; `labels_tx` does not.** `fresh_rank`, `trust_rank`,
/// `trust_chain` and `policy` are properties of the graph and are exactly what
/// §3's "uniform labels" means — dropping them would make an attached layer
/// arrive silently untrusted, and a layer is attached *because* of its trust
/// rank. `labels_tx` is deliberately left NULL: it is a transaction id in the
/// ATTACHED store's sequence, meaningless in this one, and copying it would put
/// a foreign integer where a local tx id is expected — which is precisely the
/// looks-valid-and-is-not failure quipu #74 spent its acceptance on.
pub(crate) fn register_attached_graphs(
    conn: &Connection,
    attachments: &[Attachment],
) -> Result<usize> {
    let mut registered = 0;
    for a in attachments {
        let alias = &a.alias;
        // The attachment's own meta-graph id, in ITS term space. Absent is
        // fine — a layer predating graph labels simply has none.
        let meta: Option<i64> = conn
            .query_row(
                &format!("SELECT id FROM {alias}.terms WHERE iri = ?1"),
                params![crate::namespace::META_GRAPH_IRI],
                |r| r.get(0),
            )
            .optional()?;
        // `INSERT OR REPLACE` keyed on g: re-opening a store with the same
        // attachment must converge rather than accumulate or fail. The source
        // column is re-asserted each time, so a graph that moved between
        // attachments is re-attributed rather than left claiming the old one.
        registered += conn.execute(
            &format!(
                "INSERT OR REPLACE INTO main.graphs \
                     (g, class, parent_branch, created_at, source, \
                      fresh_rank, trust_rank, trust_chain, policy) \
                 SELECT g, class, parent_branch, created_at, ?1, \
                        fresh_rank, trust_rank, trust_chain, policy \
                 FROM {alias}.graphs WHERE g <> 0 AND g IS NOT ?2"
            ),
            params![alias, meta],
        )?;
    }
    Ok(registered)
}

#[cfg(test)]
mod tests;
