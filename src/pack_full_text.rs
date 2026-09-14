//! The `--full --format text` pack: a LOSSLESS whole-store artifact **as text**.
//!
//! [`crate::pack_full`] is lossless and BINARY — a `VACUUM INTO` copy. Stiwi's
//! directive (aegis-9f899e) asks for one artifact that is both: *"a git-friendly
//! TEXT export that losslessly RECONSTRUCTS the store (unpack == identical
//! sqlite contents)"*, and earlier, *"I dont want tk be tied to a sqlite blob"*.
//! A lossy text share plus a lossless sqlite blob is two halves of neither
//! (sattler's ruling, 2026-09-14). This module is the conjunction.
//!
//! ## It must answer `pack_full`'s own argument, not ignore it
//!
//! That module chose copy-and-prune deliberately:
//!
//! > "an enumerate-and-rebuild pack is lossy whenever someone forgets a table,
//! > and nothing tells you. A copy-and-prune is lossless unless someone deletes
//! > something."
//!
//! A text pack is unavoidably an enumerate-and-serialise, which is exactly that
//! failure mode. So it is built ON the pruned copy rather than beside it: the
//! same `VACUUM INTO`, the same [`crate::share_completeness::DECLARED`] prune,
//! and the table list comes from `sqlite_master` at pack time — never a literal
//! in this file. A table that exists in the store and is missing from the dump
//! changes the row counts and the content hash, so "someone forgot a table"
//! fails loudly instead of shipping a quietly smaller store.
//!
//! ## Why the hash's canonical form is reused, and where it must NOT be
//!
//! [`crate::pack_full::content_hash_of`] already renders the whole store as
//! deterministic text — per table, rows ordered by the row text itself, so the
//! rendering is insensitive to the physical order a VACUUM is free to change.
//! That ordering is also what makes this format git-friendly: a row moving on
//! disk produces no diff.
//!
//! ⚠️ But that exact rendering **cannot be the wire format**. It joins columns
//! with `'|'`, and `quote()` of a text value may itself contain `|`, so
//! splitting a line on the delimiter is ambiguous — and the ambiguity is
//! data-dependent, so it would reconstruct wrongly only for some stores and
//! pass every fixture that happens to hold no pipes. A form that is adequate as
//! a HASH INPUT is not automatically adequate as a wire format.
//!
//! So rows are emitted as one `INSERT` statement each, values via the same
//! `quote()`, rows in the same order: valid SQL, unambiguous, replayable, and
//! still a deterministic sorted text file. A canonicalised `.dump`, not a novel
//! encoding.

use std::path::Path;

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::pack::{Manifest, PackOptions};
use crate::pack_full::{content_hash_of, live_tables, manifest_for};

/// A lossless whole-store pack rendered as text. Read by `restore`.
pub const FORMAT_FULL_TEXT: &str = "quipu-pack-full-text/1";

/// The manifest lives beside the data rather than inside it, because the data
/// is the thing being hashed and a manifest inside its own hash is circular —
/// the same reason [`crate::pack_full::content_hash_of`] skips `pack_manifest`.
pub const MANIFEST_FILE: &str = "manifest.json";
/// DDL for every object, so the reconstruction does not depend on this build's
/// migrations having produced the same schema as the producer's.
pub const SCHEMA_FILE: &str = "schema.sql";
/// One file per table: a table's history stays readable in `git log -- <file>`.
pub const DATA_DIR: &str = "data";

/// Build a lossless TEXT whole-store pack in the directory `out_dir`.
///
/// # Errors
/// Refuses an outward destination for the same reason [`crate::pack_full`]
/// does — this carries `events` and every operational table, and publishing one
/// is the operator's decision, not this command's. Also propagates SQLite and
/// filesystem errors.
pub fn pack_full_text(
    store: &crate::store::Store,
    out_dir: &str,
    opts: &PackOptions,
    timestamp: &str,
) -> Result<Manifest> {
    if !opts.destination.is_internal() {
        return Err(Error::PolicyDenied(format!(
            "pack --full --format text is INTERNAL ONLY and this pack is bound \
             outward. A full pack is lossless: it carries the event log and \
             every operational table, so publishing one exposes far more than a \
             share does, and that is a decision for the operator rather than \
             for this command. Pass {} if this pack is bound for a LAN-internal \
             destination (aegis-9f899e).",
            crate::share_scrub::INTERNAL_FLAG
        )));
    }

    let build_path = format!("{out_dir}.building.db");
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{build_path}{suffix}");
        if Path::new(&p).exists() {
            std::fs::remove_file(&p)
                .map_err(|e| Error::Store(format!("pack --full --format text: {p}: {e}")))?;
        }
    }

    let built = (|| -> Result<Manifest> {
        // VACUUM INTO, not a filesystem copy: the store runs in WAL mode, so
        // copying the file alone silently drops whatever is still in the -wal,
        // and a pack missing the tail of the log is not lossless.
        store.conn.execute(
            &format!("VACUUM INTO '{}'", build_path.replace('\'', "''")),
            [],
        )?;

        let conn = Connection::open(&build_path)?;
        let present = live_tables(&conn)?;
        let mut pruned = Vec::new();
        for table in crate::pack_full::excluded() {
            // A declared exclusion that is ABSENT is normal, not an error:
            // `pack_loads` is created lazily by the unpack path.
            if present.iter().any(|t| t == table) {
                conn.execute(&format!("DELETE FROM \"{table}\""), [])?;
                pruned.push(table);
            }
        }

        let mut manifest = manifest_for(&conn, opts, timestamp, &pruned)?;
        // The format is what tells `restore` which reader to use, and the gate
        // reads it BEFORE hashing so a wrong-verb artifact is never reported as
        // corrupt (see `pack_restore`).
        manifest.pack_format = FORMAT_FULL_TEXT.to_string();

        write_tree(&conn, out_dir, &manifest)?;
        drop(conn);
        Ok(manifest)
    })();

    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{build_path}{suffix}");
        if Path::new(&p).exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
    built
}

/// Write `schema.sql`, `data/<table>.sql` and the manifest for a pruned copy.
fn write_tree(conn: &Connection, out_dir: &str, manifest: &Manifest) -> Result<()> {
    let data_dir = Path::new(out_dir).join(DATA_DIR);
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| Error::Store(format!("pack --full --format text: {out_dir}: {e}")))?;

    let mut schema = String::from(
        "-- quipu-pack-full-text/1 schema. Applied before any INSERT so the\n\
         -- reconstruction does not depend on the reader's migrations.\n",
    );
    {
        // ORDER BY type puts tables before the indexes and triggers that
        // reference them; name keeps the file stable across runs.
        let mut stmt = conn.prepare(
            "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL \
             AND name NOT LIKE 'sqlite_%' ORDER BY (type <> 'table'), name",
        )?;
        for sql in stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
        {
            schema.push_str(&sql);
            schema.push_str(";\n");
        }
    }
    std::fs::write(data_dir.parent().unwrap().join(SCHEMA_FILE), schema)
        .map_err(|e| Error::Store(format!("pack --full --format text: {SCHEMA_FILE}: {e}")))?;

    for name in live_tables(conn)? {
        // `pack_manifest` is excluded for the reason the hash excludes it: it
        // describes the artifact, so carrying it inside the artifact's own
        // hashed data is circular. `restore` rewrites it from manifest.json.
        if name == "pack_manifest" {
            continue;
        }
        std::fs::write(
            data_dir.join(format!("{name}.sql")),
            table_sql(conn, &name)?,
        )
        .map_err(|e| Error::Store(format!("pack --full --format text: {name}.sql: {e}")))?;
    }

    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| Error::Store(format!("pack --full --format text: manifest: {e}")))?;
    std::fs::write(Path::new(out_dir).join(MANIFEST_FILE), json + "\n")
        .map_err(|e| Error::Store(format!("pack --full --format text: {MANIFEST_FILE}: {e}")))?;
    Ok(())
}

/// One table as canonical, deterministically ordered `INSERT` statements.
fn table_sql(conn: &Connection, name: &str) -> Result<String> {
    let cols: Vec<String> = {
        let mut c = conn.prepare(&format!("PRAGMA table_info(\"{name}\")"))?;
        c.query_map([], |r| r.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut out = format!("-- {name}\n");
    if cols.is_empty() {
        return Ok(out);
    }
    let quoted = cols
        .iter()
        .map(|c| format!("quote(\"{c}\")"))
        .collect::<Vec<_>>();
    // Values joined with ',' for the INSERT, but ORDERED BY the same '|'
    // concatenation the hash uses, so the text file and the hash agree on row
    // order by construction rather than by a second sort kept in step by hand.
    let values = quoted.join(" || ',' || ");
    let order = quoted.join(" || '|' || ");
    let names = cols
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT {values} AS v FROM \"{name}\" ORDER BY {order}"
    ))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Option<String>>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for row in rows.into_iter().flatten() {
        out.push_str(&format!(
            "INSERT INTO \"{name}\" ({names}) VALUES ({row});\n"
        ));
    }
    Ok(out)
}

/// Rebuild a store from a text pack directory into `build_path`.
///
/// Returns the rebuilt connection's recomputed content hash alongside the
/// manifest, so the caller can refuse a mismatch before installing anything.
///
/// # Errors
/// Propagates a missing manifest, schema or data file, and SQLite errors from
/// replaying the dump.
pub fn rebuild(dir: &str, build_path: &str) -> Result<(Manifest, String)> {
    let manifest = read_manifest_dir(dir)?;
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{build_path}{suffix}");
        if Path::new(&p).exists() {
            std::fs::remove_file(&p)
                .map_err(|e| Error::Store(format!("quipu restore: cannot replace {p}: {e}")))?;
        }
    }

    let conn = Connection::open(build_path)?;
    // Replay cannot satisfy foreign keys in file order, and MUST NOT try to.
    // A topological sort of the tables would be a second, hand-maintained model
    // of the schema's own references — wrong the first time a migration adds
    // one, and wrong silently. Instead the constraints are deferred for the
    // replay and then CHECKED, which is strictly stronger: it validates the
    // reconstructed store's referential integrity as a whole rather than
    // trusting an insertion order to have implied it.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let schema = std::fs::read_to_string(Path::new(dir).join(SCHEMA_FILE)).map_err(|e| {
        Error::InvalidValue(format!(
            "quipu restore: {dir}/{SCHEMA_FILE} is missing or unreadable ({e}). \
             A text pack without its schema cannot be reconstructed."
        ))
    })?;
    conn.execute_batch(&schema)?;

    let data_dir = Path::new(dir).join(DATA_DIR);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&data_dir)
        .map_err(|e| {
            Error::InvalidValue(format!(
                "quipu restore: {dir}/{DATA_DIR} is missing or unreadable ({e})."
            ))
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();
    for path in files {
        let sql = std::fs::read_to_string(&path)
            .map_err(|e| Error::Store(format!("quipu restore: {}: {e}", path.display())))?;
        conn.execute_batch(&sql)?;
    }

    // The integrity gate the deferral above owes. A dangling reference here
    // means the dump is internally inconsistent, which the content hash alone
    // would not catch: a pack missing a parent row still hashes to whatever it
    // does contain, self-consistently.
    {
        let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
        let violations: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !violations.is_empty() {
            let mut names = violations.clone();
            names.sort();
            names.dedup();
            return Err(Error::InvalidValue(format!(
                "quipu restore: the reconstructed store fails referential integrity in \
                 {} table(s): {}. The text pack is internally inconsistent — a \
                 referenced row is missing. Nothing was written (aegis-9f899e).",
                names.len(),
                names.join(", ")
            )));
        }
    }

    let recomputed = content_hash_of(&conn)?;
    crate::pack::write_manifest(&conn, &manifest)?;
    drop(conn);
    Ok((manifest, recomputed))
}

/// Read a text pack's manifest from its directory.
///
/// # Errors
/// Reports a missing or malformed manifest as exactly that, rather than letting
/// it degrade into a more specific-sounding and less true diagnosis downstream.
pub fn read_manifest_dir(dir: &str) -> Result<Manifest> {
    let path = Path::new(dir).join(MANIFEST_FILE);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        Error::InvalidValue(format!(
            "quipu restore: {} is missing or unreadable ({e}). \
             A text pack is a DIRECTORY containing {MANIFEST_FILE}, {SCHEMA_FILE} and {DATA_DIR}/.",
            path.display()
        ))
    })?;
    serde_json::from_str(&raw).map_err(|e| {
        Error::InvalidValue(format!(
            "quipu restore: {} is malformed: {e}",
            path.display()
        ))
    })
}

/// Is `path` a text pack directory?
#[must_use]
pub fn is_text_pack(path: &str) -> bool {
    Path::new(path).join(MANIFEST_FILE).is_file() && Path::new(path).join(DATA_DIR).is_dir()
}

#[cfg(test)]
#[path = "pack_full_text_tests.rs"]
mod tests;
