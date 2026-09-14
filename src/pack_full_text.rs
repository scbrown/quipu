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

        // THE REGENERATED SET DOES NOT TRAVEL IN TEXT, and `DECLARED` says so at
        // the entry rather than leaving it to this module: "~2.2 GB of floats at
        // homelab scale, which rules out text. The pinned embedding model and
        // config are part of the declared set precisely because regeneration is
        // only reconstruction if the recipe travels."
        //
        // The binary `--full` pack transports them anyway — a backup that forces
        // a re-embed on restore is a poor backup — so this is the one place the
        // two whole-store packs deliberately carry different content, and it is
        // why the two hashes are not comparable to each other.
        //
        // Inlining them would defeat the artifact's whole purpose: `quote()`
        // renders a BLOB as X'<hex>', roughly doubling the bytes, so a
        // vector-bearing store would produce a 4-5 GB "git-friendly" file.
        let mut regenerated = Vec::new();
        for table in crate::pack_full::regenerated() {
            if present.iter().any(|t| t == table) {
                // Counted BEFORE the delete. Afterwards the per-table count is
                // 0, which is honest about the artifact and useless to a
                // consumer asking "how much do I have to rebuild?".
                let n: i64 =
                    conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
                        r.get(0)
                    })?;
                conn.execute(&format!("DELETE FROM \"{table}\""), [])?;
                regenerated.push((table, n));
            }
        }

        let recipe = embedding_recipe(store)?;
        let mut manifest = manifest_for(&conn, opts, timestamp, &pruned)?;
        manifest.counts = merge_counts(&manifest.counts, &regenerated, &recipe)?;
        drop(present);
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

/// The recipe a consumer needs to REGENERATE what this pack does not carry.
///
/// A name alone is not a recipe: `all-MiniLM-L6-v2` names a family, and two
/// files under that name need not produce the same vectors. The digest is what
/// makes "re-embed with the same model" checkable rather than assumed, which is
/// the whole of `DECLARED`'s "regeneration is only reconstruction if the recipe
/// travels".
fn embedding_recipe(store: &crate::store::Store) -> Result<serde_json::Value> {
    let config = store.embedding_config();
    let model = config
        .model_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|p| p.to_string_lossy().into_owned());
    // Absent model, absent digest — and reported as null rather than as a
    // string, so a consumer cannot mistake "no model configured" for a match.
    let digest = match config.model_path.as_ref() {
        Some(path) if path.exists() => Some(sha256_file(path)?),
        _ => None,
    };
    Ok(serde_json::json!({
        "embedding_model": model,
        "embedding_model_sha256": digest,
        "embedding_dimension": config.dimension,
    }))
}

/// SHA-256 of a file, streamed rather than read whole: an ONNX model is ~90 MB.
fn sha256_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| {
        Error::Store(format!(
            "pack --full --format text: {}: {e}",
            path.display()
        ))
    })?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| {
            Error::Store(format!(
                "pack --full --format text: {}: {e}",
                path.display()
            ))
        })?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    let mut hex = String::from("sha256:");
    for byte in ctx.finish().as_ref() {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

/// Fold the regenerated set and its recipe into the manifest's counts JSON.
///
/// Recorded IN the manifest rather than only in this module's documentation so
/// a consumer can tell what is missing and how to rebuild it **from the
/// artifact**, without having to trust that the producer followed a convention.
fn merge_counts(
    counts: &str,
    regenerated: &[(&str, i64)],
    recipe: &serde_json::Value,
) -> Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(counts)
        .map_err(|e| Error::Store(format!("pack --full --format text: counts: {e}")))?;
    let map = value
        .as_object_mut()
        .ok_or_else(|| Error::Store("pack --full --format text: counts is not an object".into()))?;
    // The per-table counts are taken AFTER the prune, so a regenerated table
    // reads 0 there. That zero is honest about the artifact and misleading
    // about the source, so the source count is recorded separately.
    let source_counts: serde_json::Map<String, serde_json::Value> = regenerated
        .iter()
        .map(|(t, n)| ((*t).to_string(), serde_json::json!(n)))
        .collect();
    let names: Vec<&str> = regenerated.iter().map(|(t, _)| *t).collect();
    map.insert("regenerated".into(), serde_json::json!(names));
    map.insert(
        "regenerated_source_counts".into(),
        serde_json::Value::Object(source_counts),
    );
    map.insert("regeneration_recipe".into(), recipe.clone());
    serde_json::to_string(&value)
        .map_err(|e| Error::Store(format!("pack --full --format text: counts: {e}")))
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

/// Warn at creation when omitted vectors have no complete recorded recipe.
///
/// The backup remains useful for facts and history; this diagnostic prevents
/// its successful creation from implying reproducible vector reconstruction.
#[must_use]
pub fn pack_recipe_warning(counts: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(counts).ok()?;
    let rows = value["regenerated_source_counts"]["vectors"].as_i64()?;
    let recipe = &value["regeneration_recipe"];
    let complete = ["embedding_model", "embedding_model_sha256"]
        .iter()
        .all(|key| recipe[key].as_str().is_some_and(|s| !s.is_empty()));
    if rows <= 0 || complete {
        return None;
    }
    Some(format!(
        "text pack omits {rows} vector row(s), but its embedding recipe lacks a model \
         name or SHA-256 digest; the original vectors are not reproducible from this \
         recipe. Configure [quipu.embedding] model_path to the original readable model \
         file and repack, or use binary pack --full to retain vectors."
    ))
}

/// A human-readable statement of what a restored text pack still has to rebuild.
///
/// Returns `None` when the pack carried everything — so a caller printing this
/// says nothing rather than saying "regenerate: none", which reads like a
/// reassurance nobody checked.
#[must_use]
pub fn regeneration_notice(counts: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(counts).ok()?;
    let names = value.get("regenerated")?.as_array()?;
    if names.is_empty() {
        return None;
    }
    let sources = value.get("regenerated_source_counts");
    let listed: Vec<String> = names
        .iter()
        .filter_map(|n| n.as_str())
        .map(|n| {
            let rows = sources
                .and_then(|s| s.get(n))
                .and_then(serde_json::Value::as_i64);
            match rows {
                Some(rows) => format!("{n} ({rows} row(s))"),
                None => n.to_string(),
            }
        })
        .collect();
    let recipe = value.get("regeneration_recipe");
    let model = recipe
        .and_then(|r| r.get("embedding_model"))
        .and_then(serde_json::Value::as_str);
    let digest = recipe
        .and_then(|r| r.get("embedding_model_sha256"))
        .and_then(serde_json::Value::as_str);
    let with = match (model, digest) {
        // The digest is what makes "the same model" checkable; a bare name is a
        // family, not a model, so it is reported as the weaker claim it is.
        (Some(m), Some(d)) => format!(" with {m} ({d})"),
        (Some(m), None) => format!(" with {m} (no digest recorded — name only)"),
        _ => " — NO MODEL RECORDED, so the original vectors are not reproducible".to_string(),
    };
    Some(format!("{}{with}", listed.join(", ")))
}

/// Is `path` a text pack directory?
#[must_use]
pub fn is_text_pack(path: &str) -> bool {
    Path::new(path).join(MANIFEST_FILE).is_file() && Path::new(path).join(DATA_DIR).is_dir()
}

#[cfg(test)]
#[path = "pack_full_text_tests.rs"]
mod tests;
