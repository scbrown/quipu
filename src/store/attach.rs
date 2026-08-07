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

/// Every `facts` column a composed query projects from each branch of the
/// union (see [`build_facts_source`]).
///
/// This list is a **contract with the schema**, not documentation. Two
/// mechanisms hold it to the schema rather than to anyone's memory:
///
/// - `facts_columns_matches_the_schema` fails the moment a column is added to
///   `facts` without a decision about whether the composed source projects it;
/// - [`verify_attached_schema`] refuses an attachment whose `facts` lacks any
///   of these, because a `UNION ALL` branch that cannot produce a column is a
///   query-time failure in a path the caller did not ask to compose.
///
/// The rule quipu #74's acceptance amendment set, one level up: derive the
/// work from the schema and refuse what you cannot classify.
pub(crate) const FACTS_COLUMNS: &[&str] = &[
    "e",
    "a",
    "v",
    "g",
    "tx",
    "valid_from",
    "valid_to",
    "op",
    "retracted_tx",
];

/// The graphs an attachment **contributes**, as a SQL predicate on `g`.
///
/// ONE definition, used by both [`register_attached_graphs`] (which of the
/// layer's graphs enter the local registry) and [`build_facts_source`] (which
/// of the layer's rows a composed query can read). These two sets must be
/// identical: a registry entry with no readable rows is a graph that resolves
/// and returns nothing, and a readable row with no registry entry is a fact
/// arriving from a graph the store cannot name or label. Sharing the predicate
/// is what makes drifting apart impossible rather than merely unlikely;
/// `the_union_reads_exactly_the_registered_graphs` pins it.
///
/// `meta` is the attachment's own label meta-graph id, in ITS term space, or
/// `None` for a layer predating graph labels.
fn contributed_graphs_sql(meta: Option<i64>) -> String {
    // Reserved graphs are per-database and are NOT contributed — see
    // `register_attached_graphs`. `g <> 0` drops the layer's own ROOT, which is
    // the compatibility guarantee itself: without it the layer's default-graph
    // facts join the LOCAL default graph, and every existing query returns more
    // rows than it did before the attach. Measured on a two-store composition:
    // one local ROOT fact became two.
    match meta {
        Some(m) => format!("g <> 0 AND g <> {m}"),
        None => "g <> 0".to_string(),
    }
}

/// The attachment's own label meta-graph id, in ITS term space.
///
/// Absent is fine — a layer predating graph labels simply has none.
fn attached_meta_graph(conn: &Connection, alias: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            &format!("SELECT id FROM {alias}.terms WHERE iri = ?1"),
            params![crate::namespace::META_GRAPH_IRI],
            |r| r.get(0),
        )
        .optional()?)
}

/// The table source a composed query reads `facts` from
/// (`multi-db-composition.md` §4).
///
/// - **No attachments → the literal `"facts"`.** Byte-identical to the SQL
///   every query built before this feature existed. That is acceptance 1, and
///   it is a stronger requirement than "the same results": a store that never
///   asked to compose must not pay a planner cost, and the diff of this
///   feature against an unattached store must be empty.
/// - **With attachments → `(SELECT … FROM main.facts UNION ALL SELECT …
///   FROM <alias>.facts WHERE <contributed>) AS facts`.**
///
/// **Never a SQL VIEW named `facts`** — a view would shadow the real table and
/// break every write.
///
/// # Why the caller's `WHERE` may stay outside
///
/// Acceptance 2 requires the graph predicate to reach *inside* each branch, so
/// each file's own `idx_geav` is used instead of both files being scanned per
/// triple pattern. Measured on the bundled `SQLite` 3.48.0 rather than assumed:
/// its push-down optimisation moves the outer `WHERE` into both branches, and
/// `EXPLAIN QUERY PLAN` for the outer-`WHERE` form is **identical** to the form
/// with the predicate written into each branch by hand — `SEARCH main.facts
/// USING INDEX idx_geav (g=?)` and the same for the attachment.
///
/// So this returns a drop-in table source and callers are unchanged apart from
/// one interpolation. That is a property of the planner, not of our SQL, which
/// is why `graph_predicate_is_pushed_into_each_union_branch` asserts the plan:
/// if a future `SQLite` stops pushing down, the union silently becomes a
/// double table scan on every triple pattern, and a test is the only thing that
/// would say so.
///
/// # Errors
/// [`Error::Sqlite`] if an attachment's `terms` cannot be read.
pub(crate) fn build_facts_source(conn: &Connection, attachments: &[Attachment]) -> Result<String> {
    if attachments.is_empty() {
        return Ok("facts".to_string());
    }
    let cols = FACTS_COLUMNS.join(", ");
    let mut branches = vec![format!("SELECT {cols} FROM main.facts")];
    for a in attachments {
        let alias = &a.alias;
        let meta = attached_meta_graph(conn, alias)?;
        branches.push(format!(
            "SELECT {cols} FROM {alias}.facts WHERE {}",
            contributed_graphs_sql(meta)
        ));
    }
    Ok(format!("({}) AS facts", branches.join(" UNION ALL ")))
}

/// The term-resolution query for a store with no attachments — today's exact
/// SQL, and the byte-identity baseline for [`build_resolve_sql`].
pub(crate) const RESOLVE_SQL_LOCAL: &str = "SELECT iri FROM terms WHERE id = ?1";

/// The SQL `Store::resolve` uses to turn a term id back into an IRI.
///
/// # Why this direction crosses the attachment boundary and the other does not
///
/// The union [`build_facts_source`] builds yields rows whose `e`, `a` and `g`
/// are term ids belonging to an attached file. Without resolution the composed
/// query returns them and then fails — measured before writing this, on a
/// two-store composition: `GRAPH ?g { ?s ?p ?o }` produced
/// `unknown term id: 8796093022210`. The union is not usable without this, so
/// it is part of the same increment; an error that reads like store corruption
/// is not an acceptable interim state for a feature that otherwise looks
/// finished.
///
/// **id → IRI is unambiguous by construction.** Term spaces (quipu #74)
/// partition the id range per database and `verify_attached_schema` refuses a
/// composition whose spaces collide, so an id belongs to exactly one file. The
/// branches cannot disagree; `LIMIT 1` is a bound, not a tie-break.
///
/// The opposite direction is intentionally handled by `Store::lookup_all`:
/// one IRI may denote several ids, and query predicates must match all of them
/// before canonicalising bindings back toward the local id (quipu #76).
pub(crate) fn build_resolve_sql(attachments: &[Attachment]) -> String {
    if attachments.is_empty() {
        return RESOLVE_SQL_LOCAL.to_string();
    }
    let mut branches = vec!["SELECT iri FROM main.terms WHERE id = ?1".to_string()];
    for a in attachments {
        branches.push(format!("SELECT iri FROM {}.terms WHERE id = ?1", a.alias));
    }
    format!("{} LIMIT 1", branches.join(" UNION ALL "))
}

/// Refuse an attachment quipu cannot compose, naming the fix.
///
/// Three refusals, all **before** anything reads the attachment:
///
/// - its `facts` has no `g` column, so it predates named graphs and has no
///   notion of the graph a composed query selects on;
/// - its `facts` is missing some other column of [`FACTS_COLUMNS`], which a
///   `UNION ALL` branch must produce;
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
        let present: Vec<String> = conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('facts', '{alias}')"
            ))?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;

        // `g` first and by itself: it has a MEANING the others do not. Missing
        // `g` says the layer predates named graphs, so there is nothing for a
        // composed query to select on — a different diagnosis, and a different
        // sentence, from "this file is behind on migrations".
        if !present.iter().any(|c| c == "g") {
            return Err(Error::Store(format!(
                "cannot attach {:?} as {alias}: its `facts` table has no `g` \
                 column, so it predates named graphs and a composed query has no \
                 graph to select on. Open it once with a current quipu to migrate \
                 it, then attach it.",
                a.path
            )));
        }

        let missing: Vec<&str> = FACTS_COLUMNS
            .iter()
            .copied()
            .filter(|c| !present.iter().any(|p| p == c))
            .collect();
        if !missing.is_empty() {
            return Err(Error::Store(format!(
                "cannot attach {:?} as {alias}: its `facts` table is missing {}, \
                 which every branch of a composed query's UNION must produce. \
                 Refused here rather than at query time, because a query that \
                 named none of this layer's graphs would still fail. Open it once \
                 with a current quipu to migrate it, then attach it.",
                a.path,
                missing.join(", ")
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

/// Read manifests from attachments that are knowledge packs (quipu #82).
/// Ordinary stores have no `pack_manifest` table and are omitted.
pub(crate) fn attached_pack_manifests(
    conn: &Connection,
    attachments: &[Attachment],
) -> Result<Vec<(String, crate::pack::Manifest)>> {
    let mut out = Vec::new();
    for a in attachments {
        let alias = &a.alias;
        let present = conn
            .prepare(&format!(
                "SELECT 1 FROM {alias}.sqlite_master WHERE type='table' AND name='pack_manifest'"
            ))?
            .exists([])?;
        if !present {
            continue;
        }
        let manifest = conn.query_row(
            &format!(
                "SELECT pack_format,name,version,term_space,content_hash,created_at,source_graph,producer,counts \
                 FROM {alias}.pack_manifest WHERE id=1"
            ),
            [],
            |r| Ok(crate::pack::Manifest {
                pack_format: r.get(0)?, name: r.get(1)?, version: r.get(2)?,
                term_space: r.get(3)?, content_hash: r.get(4)?, created_at: r.get(5)?,
                source_graph: r.get(6)?, producer: r.get(7)?, counts: r.get(8)?,
            }),
        ).map_err(|e| Error::InvalidValue(format!(
            "cannot attach pack {:?} as {alias}: unreadable pack_manifest: {e}", a.path
        )))?;
        let actual = attached_term_space(conn, alias)?;
        if manifest.term_space != actual {
            return Err(Error::Store(format!(
                "cannot attach pack {:?} as {alias}: manifest declares term space {} but the store owns {actual}",
                a.path, manifest.term_space
            )));
        }
        out.push((alias.clone(), manifest));
    }
    Ok(out)
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
        let meta = attached_meta_graph(conn, alias)?;
        // The SAME predicate `build_facts_source` puts on this layer's rows —
        // see `contributed_graphs_sql`. Registry and readable rows are one set
        // expressed once, so they cannot drift into disagreeing.
        let contributed = contributed_graphs_sql(meta);
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
                 FROM {alias}.graphs WHERE {contributed}"
            ),
            params![alias],
        )?;
    }
    Ok(registered)
}

#[cfg(test)]
mod tests;
