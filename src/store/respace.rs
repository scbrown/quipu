//! Offline term-space remap — `quipu db respace` (quipu #74, Track B step 2).
//!
//! Moves a database from the term space it currently owns into a chosen one, so
//! two files can be composed without their ids colliding. Term ids are the
//! store's identity: they key `terms`, they are the `e`/`a`/`g` of every fact,
//! they are the primary key of the named-graph registry, and one of them is
//! embedded inside an opaque `Value::Ref` BLOB that SQL cannot rewrite.
//!
//! # The failure this module is shaped around
//!
//! **A respace that misses a term-id-bearing column does not error.** It
//! produces a store that opens, answers queries, and is wrong — the registry
//! points at terms that have moved out from under it. There is no natural
//! detector, and it happens in the migration path, on data that has already
//! been rewritten.
//!
//! So the authority for what gets rewritten is **the live schema of the
//! database in front of us**, never a list written here and hoped to be
//! current. Every column of every table is enumerated at run time and looked up
//! in [`COLUMN_CLASSIFICATION`]; a column that is not classified makes respace
//! **refuse before it writes anything**. Adding a column anywhere in the schema
//! is therefore a decision someone has to record, rather than an omission
//! nothing notices. `a_new_column_must_be_classified` is the same gate at test
//! time.
//!
//! That is a real tax on unrelated schema changes — add a column to
//! `subscriptions` and this refuses until you classify it. The tax is the
//! mechanism. The alternative on offer was a hand-maintained list of the
//! interesting columns, and that list had already drifted once (quipu #74's own
//! scope names three `facts` columns and misses `graphs.g`,
//! `graphs.parent_branch` and `vectors.entity_id`).
//!
//! # The original is never opened for writing
//!
//! The source is opened `SQLITE_OPEN_READ_ONLY` and copied with `VACUUM INTO`;
//! every rewrite happens in the new file. The old file is not a backup that
//! respace promises to leave alone — it is a file respace holds no write
//! handle to. `original_is_byte_identical_after_respace` pins it by digest.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

use crate::error::{Error, Result};
use crate::schema::{ROOT_GRAPH, SPACE_SIZE};
use crate::types::{TAG_REF, Value};

/// The largest space a database can be moved into: space `s` owns
/// `[s·2^40, (s+1)·2^40)`, so the top of the last whole space must still fit in
/// a positive `i64` rowid.
pub const MAX_SPACE: i64 = (i64::MAX / SPACE_SIZE) - 1;

/// How a stored column relates to term identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermIdKind {
    /// Holds a term id directly. Remapped arithmetically.
    ///
    /// [`ROOT_GRAPH`] (`0`) is exempt wherever it can appear: it is a reserved
    /// sentinel naming the default committed graph, not an interned term, and
    /// it means the same thing in every space. Shifting it would invent a graph
    /// id that no `terms` row backs.
    Id,
    /// Holds a `Value` blob. `Value::Ref` embeds a term id; every other variant
    /// is opaque payload and is left exactly as it is.
    RefBlob,
    /// Carries no term identity — remapping it would corrupt the store.
    ///
    /// Note what is deliberately here: `facts.tx` and `graphs.labels_tx` are
    /// transaction ids, `term_spaces.space` is a space number, `events.subject`
    /// and `dataset_members.graph_iri` are IRIs. Several are integers that look
    /// exactly like term ids and are not.
    None,
}

/// Every column of every table quipu creates, and whether it carries term
/// identity.
///
/// Kept in schema order (`schema::INIT_SQL`, then the `migrate_*` functions,
/// then `vector::VECTORS_SQL`) so it reads against the DDL side by side.
///
/// This is **not** the list respace iterates. Respace iterates the live
/// schema and consults this; the difference is the whole point, because a
/// column that exists in the database and not here is the failure case, and a
/// list that drives the work can never notice it.
pub const COLUMN_CLASSIFICATION: &[(&str, &str, TermIdKind)] = &[
    // -- schema::INIT_SQL --
    ("terms", "id", TermIdKind::Id),
    ("terms", "iri", TermIdKind::None),
    ("transactions", "id", TermIdKind::None),
    ("transactions", "timestamp", TermIdKind::None),
    ("transactions", "actor", TermIdKind::None),
    ("transactions", "source", TermIdKind::None),
    ("facts", "e", TermIdKind::Id),
    ("facts", "a", TermIdKind::Id),
    ("facts", "v", TermIdKind::RefBlob),
    ("facts", "g", TermIdKind::Id),
    ("facts", "tx", TermIdKind::None),
    ("facts", "valid_from", TermIdKind::None),
    ("facts", "valid_to", TermIdKind::None),
    ("facts", "op", TermIdKind::None),
    ("graphs", "g", TermIdKind::Id),
    ("graphs", "class", TermIdKind::None),
    ("graphs", "parent_branch", TermIdKind::Id),
    ("graphs", "created_at", TermIdKind::None),
    ("shapes", "name", TermIdKind::None),
    ("shapes", "turtle", TermIdKind::None),
    ("shapes", "loaded_at", TermIdKind::None),
    ("proposals", "id", TermIdKind::None),
    ("proposals", "kind", TermIdKind::None),
    ("proposals", "target", TermIdKind::None),
    ("proposals", "diff", TermIdKind::None),
    ("proposals", "rationale", TermIdKind::None),
    ("proposals", "proposer", TermIdKind::None),
    ("proposals", "trigger_ref", TermIdKind::None),
    ("proposals", "status", TermIdKind::None),
    ("proposals", "decided_by", TermIdKind::None),
    ("proposals", "decided_at", TermIdKind::None),
    ("proposals", "decision_note", TermIdKind::None),
    ("proposals", "created_at", TermIdKind::None),
    ("ontologies", "name", TermIdKind::None),
    ("ontologies", "turtle", TermIdKind::None),
    ("ontologies", "loaded_at", TermIdKind::None),
    // `events.subject` and every id inside `events.payload` are RESOLVED IRIs,
    // not term ids — `store::events` calls `resolve_cached` before writing
    // either. Checked, because an integer-bearing JSON payload is exactly where
    // a term id would hide from this table.
    ("events", "offset", TermIdKind::None),
    ("events", "type", TermIdKind::None),
    ("events", "ts", TermIdKind::None),
    ("events", "subject", TermIdKind::None),
    ("events", "group_id", TermIdKind::None),
    ("events", "tx_id", TermIdKind::None),
    ("events", "payload", TermIdKind::None),
    ("consumers", "consumer_id", TermIdKind::None),
    ("consumers", "committed_offset", TermIdKind::None),
    ("consumers", "filter", TermIdKind::None),
    ("consumers", "updated_at", TermIdKind::None),
    ("subscriptions", "id", TermIdKind::None),
    ("subscriptions", "consumer_id", TermIdKind::None),
    ("subscriptions", "types", TermIdKind::None),
    ("subscriptions", "sparql_ask", TermIdKind::None),
    ("subscriptions", "mode", TermIdKind::None),
    ("subscriptions", "webhook_url", TermIdKind::None),
    ("subscriptions", "batch_size", TermIdKind::None),
    ("subscriptions", "batch_window_s", TermIdKind::None),
    ("subscriptions", "created_at", TermIdKind::None),
    ("schema_terms", "term", TermIdKind::None),
    ("schema_terms", "kind", TermIdKind::None),
    ("schema_terms", "first_offset", TermIdKind::None),
    ("term_spaces", "space", TermIdKind::None),
    ("term_spaces", "db", TermIdKind::None),
    ("term_spaces", "local", TermIdKind::None),
    // -- Store::migrate_graph_labels (quipu #65) --
    // `trust_chain` is TEXT holding the chain's IRI, deliberately not interned,
    // precisely to stay off this surface. `labels_tx` is a transaction id.
    ("graphs", "fresh_rank", TermIdKind::None),
    ("graphs", "trust_rank", TermIdKind::None),
    ("graphs", "trust_chain", TermIdKind::None),
    ("graphs", "policy", TermIdKind::None),
    ("graphs", "labels_tx", TermIdKind::None),
    // -- attach::migrate_graph_source (quipu #75) --
    // TEXT holding an attachment ALIAS, not a term id. Added by #75 — and this
    // entry exists because respace REFUSED to run until it did, one hour after
    // the gate was built, in the ordinary course of building the next issue.
    // That is the acceptance-5 mechanism working on its author.
    ("graphs", "source", TermIdKind::None),
    // -- Store::migrate_datasets (quipu #69) --
    // Members are graph IRIs, not term ids, for the same reason.
    ("datasets", "name", TermIdKind::None),
    ("datasets", "created_at", TermIdKind::None),
    ("dataset_members", "dataset", TermIdKind::None),
    ("dataset_members", "graph_iri", TermIdKind::None),
    ("dataset_members", "ord", TermIdKind::None),
    // -- Store::migrate_bitemporal_registries (quipu #71) --
    ("shapes", "valid_from", TermIdKind::None),
    ("shapes", "valid_to", TermIdKind::None),
    ("shapes", "tx", TermIdKind::None),
    ("ontologies", "valid_from", TermIdKind::None),
    ("ontologies", "valid_to", TermIdKind::None),
    ("ontologies", "tx", TermIdKind::None),
    // -- Store::migrate_query_registry (quipu #79) --
    ("queries", "name", TermIdKind::None),
    ("queries", "description", TermIdKind::None),
    ("queries", "template", TermIdKind::None),
    ("queries", "dataset", TermIdKind::None),
    ("queries", "valid_from", TermIdKind::None),
    ("queries", "valid_to", TermIdKind::None),
    ("queries", "tx", TermIdKind::None),
    ("query_params", "name", TermIdKind::None),
    ("query_params", "valid_from", TermIdKind::None),
    ("query_params", "ord", TermIdKind::None),
    ("query_params", "param", TermIdKind::None),
    ("query_params", "kind", TermIdKind::None),
    ("query_params", "required", TermIdKind::None),
    ("query_params", "default_val", TermIdKind::None),
    ("query_params", "description", TermIdKind::None),
    // -- Store::migrate_retraction_tx (quipu #83) --
    ("facts", "retracted_tx", TermIdKind::None),
    // -- vector::VECTORS_SQL --
    // `entity_id` IS a term id: `embedding::build_entity_text` feeds it straight
    // to `Store::entity_facts`, i.e. it is a `facts.e`. Nothing in #74's scope,
    // its acceptance amendment, or any comment in the store named this column —
    // it was found by enumerating the schema, which is the argument for
    // enumerating the schema.
    ("vectors", "entity_id", TermIdKind::Id),
    ("vectors", "text", TermIdKind::None),
    ("vectors", "embedding", TermIdKind::None),
    ("vectors", "valid_from", TermIdKind::None),
    ("vectors", "valid_to", TermIdKind::None),
];

/// What a respace moved. Every count is measured from the rewritten file, not
/// predicted from the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RespaceReport {
    pub from_space: i64,
    pub to_space: i64,
    /// Rows the remapping `UPDATE` touched, per `(table, column)`.
    ///
    /// Rows *touched*, not values *changed*: a `g` column holding only the
    /// [`ROOT_GRAPH`] sentinel, or a `parent_branch` that is all NULL, is
    /// counted here and correctly left alone. Reporting it the other way round
    /// would make "0" mean both "this column was empty" and "this column was
    /// skipped", and those need to be distinguishable.
    pub columns: Vec<(String, String, usize)>,
    /// `Value::Ref` blobs whose embedded term id was rewritten.
    pub ref_blobs: usize,
}

impl RespaceReport {
    /// Total rows touched across every column.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.columns.iter().map(|(_, _, n)| n).sum::<usize>() + self.ref_blobs
    }
}

/// A column of the live schema, paired with its classification.
pub(super) struct ClassifiedColumn {
    table: String,
    column: String,
    kind: TermIdKind,
}

/// Enumerate every column of the database in front of us and classify it.
///
/// # Errors
/// [`Error::Store`] naming every column the database has and
/// [`COLUMN_CLASSIFICATION`] does not. Deliberately reports **all** of them
/// rather than the first: an operator who has added three columns should learn
/// that in one run, not three.
pub(super) fn classify_live_schema(conn: &Connection) -> Result<Vec<ClassifiedColumn>> {
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;

    let mut classified = Vec::new();
    let mut unclassified = Vec::new();
    for table in tables {
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info(?1)")?
            .query_map(params![table], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for column in columns {
            match COLUMN_CLASSIFICATION
                .iter()
                .find(|(t, c, _)| *t == table && *c == column)
            {
                Some((_, _, kind)) => classified.push(ClassifiedColumn {
                    table: table.clone(),
                    column,
                    kind: *kind,
                }),
                None => unclassified.push(format!("{table}.{column}")),
            }
        }
    }

    if !unclassified.is_empty() {
        return Err(Error::Store(format!(
            "respace refuses: {} column(s) in this database are not classified as \
             term-id-bearing or not — {}. Classify each in \
             store::respace::COLUMN_CLASSIFICATION (TermIdKind::Id, ::RefBlob or \
             ::None). Respace cannot guess: remapping a column that is not a term \
             id corrupts it, and skipping one that is leaves the store silently \
             pointing at the wrong terms.",
            unclassified.len(),
            unclassified.join(", ")
        )));
    }
    Ok(classified)
}

/// Map one id from the source space into the destination space. **The
/// definition** of what a respace does to an id.
///
/// [`ROOT_GRAPH`] is the reserved default-graph sentinel in every space and is
/// returned unchanged; see [`TermIdKind::Id`].
const fn remap(id: i64, from_lo: i64, to_lo: i64) -> i64 {
    if id == ROOT_GRAPH {
        ROOT_GRAPH
    } else {
        id - from_lo + to_lo
    }
}

/// The same mapping as [`remap`], as a SQL expression over `column`, with
/// `?1 = from_lo` and `?2 = to_lo`.
///
/// The `Id` columns are rewritten by one bulk `UPDATE` each rather than row by
/// row, so the rule necessarily exists twice: once in Rust for the blob path,
/// once in SQL for the column path. That duplication is a drift hazard and it
/// has already bitten — sabotaging [`remap`]'s `ROOT_GRAPH` branch changed
/// nothing observable, because on the blob path a `Ref` never points at the
/// sentinel, so the branch that looked load-bearing was dead and the real
/// exemption was this string. `sql_and_rust_remap_agree` executes this
/// expression against `SQLite` and compares it to [`remap`] id for id, so the
/// two cannot drift apart silently again.
fn id_remap_sql(column: &str) -> String {
    format!(
        "CASE WHEN \"{column}\" = {ROOT_GRAPH} THEN {ROOT_GRAPH} \
         ELSE \"{column}\" - ?1 + ?2 END"
    )
}

/// Respace the database at `src` into `to_space`, writing the result to `dst`.
///
/// `src` is opened read-only and is never written. `dst` must not exist.
///
/// The destination is brought to the current schema (the usual `Store::open`
/// migrations) **before** it is respaced, so the schema being enumerated is a
/// complete one and a legacy source does not silently skip the columns its own
/// file happens to lack.
///
/// # Errors
/// - [`Error::Store`] if `dst` exists, if `to_space` is out of range, if the
///   source is already in `to_space`, if any column is unclassified, if the
///   source holds ids from a space it does not own, or if the rewritten file
///   fails its post-conditions.
/// - [`Error::Sqlite`] for underlying store errors.
pub fn respace_file(src: &Path, dst: &Path, to_space: i64) -> Result<RespaceReport> {
    if !(0..=MAX_SPACE).contains(&to_space) {
        return Err(Error::Store(format!(
            "respace refuses: space {to_space} is out of range [0, {MAX_SPACE}]"
        )));
    }
    if dst.exists() {
        return Err(Error::Store(format!(
            "respace refuses: {} already exists. The destination is written \
             fresh so the source stays byte-identical; pick a path that does not \
             exist rather than overwriting an artifact.",
            dst.display()
        )));
    }
    if !src.exists() {
        return Err(Error::Store(format!("no such store: {}", src.display())));
    }

    // Read-only, so the copy cannot mutate the original even on a bug. This is
    // the mechanism behind "the original is untouched", not a promise about it.
    let source = Connection::open_with_flags(
        src,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    // VACUUM INTO, not a filesystem copy: the store runs in WAL mode, so
    // copying the file alone silently drops everything still in the -wal.
    // Measured — a fresh store reports `journal_mode=wal`.
    source.execute(
        &format!("VACUUM INTO '{}'", sql_quote(&dst.to_string_lossy())),
        [],
    )?;
    drop(source);

    let result = respace_in_place(dst, to_space);
    if result.is_err() {
        // A half-respaced file is the one artifact this must never leave
        // behind: it opens, it queries, and its registry points at the wrong
        // terms. The source is untouched, so discarding is always safe.
        let _ = std::fs::remove_file(dst);
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = dst.as_os_str().to_owned();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(Path::new(&sidecar));
        }
    }
    result
}

/// Escape a string for a single-quoted SQL literal.
///
/// `VACUUM INTO` takes an expression, but rusqlite cannot bind a parameter to
/// it on every `SQLite` build, and a path is attacker-adjacent input often enough
/// to be worth quoting properly rather than trusting.
fn sql_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// The rewrite half, against a file this function owns exclusively.
fn respace_in_place(db: &Path, to_space: i64) -> Result<RespaceReport> {
    let path = db.to_string_lossy().to_string();
    // Bring the copy to the current schema first, so the enumeration below sees
    // a complete schema rather than whichever subset the source happened to
    // have. `Store::open` is the one definition of "current"; it is opened and
    // dropped purely for its migrations.
    drop(crate::store::Store::open(&path)?);
    let mut conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    let from_space = crate::store::local_term_space(&conn)?;
    if from_space == to_space {
        return Err(Error::Store(format!(
            "respace refuses: this store already owns space {to_space} — \
             nothing to do, and writing a copy would imply otherwise"
        )));
    }
    let from_lo = from_space * SPACE_SIZE;
    let to_lo = to_space * SPACE_SIZE;

    let classified = classify_live_schema(&conn)?;
    assert_source_ids_are_in_space(&conn, from_space)?;

    let tx = conn.transaction()?;
    // `graphs.parent_branch` REFERENCES `graphs(g)`, so rewriting the registry
    // breaks the reference for the length of the statement. Deferring moves the
    // check to COMMIT, which both permits the rewrite and turns the constraint
    // into a genuine post-condition on the finished file.
    tx.execute_batch("PRAGMA defer_foreign_keys = ON;")?;

    let mut columns = Vec::new();
    for col in &classified {
        if col.kind != TermIdKind::Id {
            continue;
        }
        // Disjoint ranges (from_space != to_space) mean no row can collide with
        // a row not yet moved, so a single UPDATE is safe even on a primary key.
        let n = tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"{}\" = {}",
                col.table,
                col.column,
                id_remap_sql(&col.column)
            ),
            params![from_lo, to_lo],
        )?;
        columns.push((col.table.clone(), col.column.clone(), n));
    }

    let mut ref_blobs = 0;
    for col in &classified {
        if col.kind == TermIdKind::RefBlob {
            ref_blobs += rewrite_ref_blobs(&tx, &col.table, &col.column, from_lo, to_lo)?;
        }
    }

    tx.execute(
        "UPDATE term_spaces SET space = ?1 WHERE local = 1",
        params![to_space],
    )?;
    tx.commit()?;

    let report = RespaceReport {
        from_space,
        to_space,
        columns,
        ref_blobs,
    };
    assert_respaced(&conn, &classified, to_space)?;
    Ok(report)
}

/// Rewrite the term id inside every `Value::Ref` blob of one column.
///
/// Decodes through [`Value::from_bytes`] / [`Value::to_bytes`] rather than
/// splicing bytes, so the rewrite follows the codec instead of duplicating it.
/// Walks forward by rowid in chunks: the store is not required to fit in
/// memory, and a statement cannot stay open across updates to its own table.
fn rewrite_ref_blobs(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    column: &str,
    from_lo: i64,
    to_lo: i64,
) -> Result<usize> {
    const CHUNK: i64 = 8192;
    let select = format!(
        "SELECT rowid, \"{column}\" FROM \"{table}\" \
         WHERE rowid > ?1 AND substr(\"{column}\", 1, 1) = ?2 ORDER BY rowid LIMIT ?3"
    );
    let update = format!("UPDATE \"{table}\" SET \"{column}\" = ?1 WHERE rowid = ?2");
    let tag = [TAG_REF];

    let mut last_rowid = 0i64;
    let mut rewritten = 0usize;
    loop {
        let batch: Vec<(i64, Vec<u8>)> = tx
            .prepare(&select)?
            .query_map(params![last_rowid, &tag[..], CHUNK], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        if batch.is_empty() {
            return Ok(rewritten);
        }
        for (rowid, blob) in batch {
            last_rowid = rowid;
            // The tag filter said this is a Ref; if the codec disagrees, that is
            // a corrupt store and respace must not paper over it.
            let Value::Ref(id) = Value::from_bytes(&blob)? else {
                return Err(Error::Store(format!(
                    "{table}.{column} rowid {rowid} carries the Ref tag but does \
                     not decode as a Ref — refusing to rewrite a store whose \
                     value encoding is inconsistent"
                )));
            };
            let moved = Value::Ref(remap(id, from_lo, to_lo)).to_bytes();
            tx.execute(&update, params![moved, rowid])?;
            rewritten += 1;
        }
    }
}

/// Refuse a source whose ids are not all inside the space it claims to own.
///
/// A blanket shift is only correct if every id shares one origin. A store
/// holding ids from two spaces has been composed by some other means, and
/// shifting it would move the foreign ids to addresses that belong to nobody —
/// silently, like every other failure in this file.
fn assert_source_ids_are_in_space(conn: &Connection, from_space: i64) -> Result<()> {
    let lo = from_space * SPACE_SIZE;
    let hi = lo + SPACE_SIZE;
    let stray: Option<i64> = conn
        .query_row(
            "SELECT id FROM terms WHERE id < ?1 OR id >= ?2 LIMIT 1",
            params![lo, hi],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = stray {
        return Err(Error::Store(format!(
            "respace refuses: term id {id} lies outside space {from_space} \
             [{lo}, {hi}), which this store claims to own. Its ids have more \
             than one origin, so a single shift cannot be correct."
        )));
    }
    Ok(())
}

/// Post-conditions on the rewritten file, checked against the live schema.
///
/// Not a restatement of the UPDATEs: it re-derives the property from the data,
/// so an UPDATE that silently matched no rows, or a column the enumeration
/// skipped, shows up here rather than in a query months later.
fn assert_respaced(
    conn: &Connection,
    classified: &[ClassifiedColumn],
    to_space: i64,
) -> Result<()> {
    let lo = to_space * SPACE_SIZE;
    let hi = lo + SPACE_SIZE;
    for col in classified {
        if col.kind != TermIdKind::Id {
            continue;
        }
        let stray: Option<i64> = conn
            .query_row(
                &format!(
                    "SELECT \"{}\" FROM \"{}\" WHERE \"{}\" IS NOT NULL \
                     AND \"{}\" <> {ROOT_GRAPH} AND (\"{}\" < ?1 OR \"{}\" >= ?2) LIMIT 1",
                    col.column, col.table, col.column, col.column, col.column, col.column
                ),
                params![lo, hi],
                |r| r.get(0),
            )
            .ok();
        if let Some(id) = stray {
            return Err(Error::Store(format!(
                "respace post-condition failed: {}.{} still holds {id}, outside \
                 space {to_space} [{lo}, {hi})",
                col.table, col.column
            )));
        }
    }

    // Every Ref must point at a term that exists. This is the check that would
    // have caught a missed `terms.id` rewrite, and it is expressed as
    // reachability rather than as arithmetic so it does not repeat the
    // rewrite's own assumption back to itself.
    let mut term_ids: HashSet<i64> = conn
        .prepare("SELECT id FROM terms")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    term_ids.insert(ROOT_GRAPH);
    for col in classified {
        if col.kind != TermIdKind::RefBlob {
            continue;
        }
        let blobs: Vec<Vec<u8>> = conn
            .prepare(&format!(
                "SELECT \"{}\" FROM \"{}\" WHERE substr(\"{}\", 1, 1) = ?1",
                col.column, col.table, col.column
            ))?
            .query_map(params![&[TAG_REF][..]], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for blob in blobs {
            if let Ok(Value::Ref(id)) = Value::from_bytes(&blob)
                && !term_ids.contains(&id)
            {
                return Err(Error::Store(format!(
                    "respace post-condition failed: {}.{} references term {id}, \
                     which no longer exists",
                    col.table, col.column
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
