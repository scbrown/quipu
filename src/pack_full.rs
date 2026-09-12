//! The `--full` pack: a LOSSLESS whole-store artifact, for internal backup.
//!
//! The sibling of [`crate::pack`], and deliberately a separate module rather
//! than a branch inside it, because these are **two artifacts with two
//! contracts** (sattler, aegis-9f899e) and a single implementation over both
//! invites a single test over both — which passes while shipping either one
//! wrongly.
//!
//! | artifact | carries | asserts |
//! |---|---|---|
//! | published (`pack`) | current facts, re-interned | the SCRUB: operational tables absent, no internal identifiers |
//! | full (this) | every carried table, whole rows | LOSSLESS: row counts and content hashes match, table by table |
//!
//! ## Why a copy-and-prune rather than an export-and-rebuild
//!
//! [`crate::pack`] builds a fresh store and replays current facts into it. That
//! is correct for a share and cannot be lossless by construction: the replay
//! goes through the ordinary write path, so `op` is `Assert` for every row and
//! the source's `tx` / `valid_to` / `retracted_tx` / `transactions` do not
//! travel. Measured on a three-write fixture: `facts 6 -> 4`, `terms 10 -> 9`,
//! `transactions 6 -> 2` — and the `terms` loss is the sharp one, because a
//! retracted entity's IRI disappears entirely, so "was there ever a bob?"
//! becomes unanswerable.
//!
//! So this copies the whole store and REMOVES the declared exclusions. That
//! inverts the failure mode, which is the point: an enumerate-and-rebuild pack
//! is lossy whenever someone forgets a table, and nothing tells you. A
//! copy-and-prune is lossless unless someone deletes something, and the prune
//! list is [`crate::share_completeness::DECLARED`] — the one place that
//! decision is already written down, and until now a declaration with no
//! production caller at all.

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::pack::{Manifest, PackOptions};
use crate::share_completeness::{DECLARED, Disposition};
use crate::share_scrub::ShareDestination;
use crate::store::Store;

/// Tables the reconstruction must NOT carry, from the single declared source.
#[must_use]
pub fn excluded() -> Vec<&'static str> {
    DECLARED
        .iter()
        .filter(|(_, d)| matches!(d, Disposition::Excluded))
        .map(|(name, _)| *name)
        .collect()
}

/// Build a lossless whole-store pack at `out_path`.
///
/// # Errors
/// Refuses an outward destination: a full pack carries `events` and the
/// operational tables, and publishing one is a decision held for the operator,
/// not something this path may acquire by convenience. Also propagates SQLite
/// errors from the copy, the prune, or the manifest write.
pub fn pack_full(
    store: &Store,
    out_path: &str,
    opts: &PackOptions,
    timestamp: &str,
) -> Result<Manifest> {
    // THE ONE-WAY DOOR, as a locked door rather than a sign. A full pack
    // includes the event log and every operational table; publishing one is
    // held for the operator (aegis-9f899e). Refusing here means the full path
    // cannot acquire a publish route by convenience — which is the exact
    // wording of the constraint, and a comment would not have enforced it.
    if !opts.destination.is_internal() {
        return Err(Error::PolicyDenied(format!(
            "pack --full is INTERNAL ONLY and this pack is bound outward. A full \
             pack is lossless: it carries the event log and every operational \
             table, so publishing one exposes far more than a share does, and \
             that is a decision for the operator rather than for this command. \
             Pass {} if this pack is bound for a LAN-internal destination \
             (aegis-9f899e).",
            crate::share_scrub::INTERNAL_FLAG
        )));
    }

    let build_path = format!("{out_path}.building");
    for p in [&build_path, out_path] {
        if std::path::Path::new(p).exists() {
            std::fs::remove_file(p)
                .map_err(|e| Error::Store(format!("pack --full: cannot replace {p}: {e}")))?;
        }
    }

    let built = (|| -> Result<Manifest> {
        // VACUUM INTO, not a filesystem copy: the store runs in WAL mode, so
        // copying the file alone silently drops whatever is still in the -wal.
        // Same reason respace gives, and the same reason it is load-bearing
        // here: a pack missing the tail of the log is not lossless.
        store.conn.execute(
            &format!("VACUUM INTO '{}'", build_path.replace('\'', "''")),
            [],
        )?;

        let conn = Connection::open(&build_path)?;
        let present = live_tables(&conn)?;
        let mut pruned = Vec::new();
        for table in excluded() {
            // A declared exclusion that is ABSENT is normal, not an error:
            // `pack_loads` is created lazily by the unpack path, so a store
            // that never loaded a pack simply does not have it.
            if present.iter().any(|t| t == table) {
                conn.execute(&format!("DELETE FROM \"{table}\""), [])?;
                pruned.push(table);
            }
        }

        let manifest = manifest_for(&conn, opts, timestamp, &pruned)?;
        crate::pack::write_manifest(&conn, &manifest)?;
        drop(conn);
        Ok(manifest)
    })();

    if built.is_ok() {
        // VACUUM INTO again so the shipped file has no -wal/-shm siblings, for
        // the same reason `pack` does: an artifact that needs three files is not
        // a single attachable artifact.
        let conn = Connection::open(&build_path)?;
        conn.execute(
            &format!("VACUUM INTO '{}'", out_path.replace('\'', "''")),
            [],
        )?;
    }
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{build_path}{suffix}");
        if std::path::Path::new(&p).exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
    built
}

/// Every non-internal table in an open connection.
fn live_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(names)
}

/// Row count for every table in `conn`, so the lossless claim is checkable
/// from the artifact rather than only by re-running the producer.
fn row_counts(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut out = Vec::new();
    for name in live_tables(conn)? {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{name}\""), [], |r| {
            r.get(0)
        })?;
        out.push((name, n));
    }
    Ok(out)
}

fn manifest_for(
    out: &Connection,
    opts: &PackOptions,
    timestamp: &str,
    pruned: &[&str],
) -> Result<Manifest> {
    let counts = row_counts(out)?;
    let total: i64 = counts.iter().map(|(_, n)| *n).sum();
    Ok(Manifest {
        pack_format: "quipu-pack-full/1".to_string(),
        name: opts.name.clone().unwrap_or_else(|| "full".to_string()),
        version: opts.version.clone().unwrap_or_else(|| "0.1.0".to_string()),
        term_space: 0,
        content_hash: content_hash_of(out)?,
        created_at: timestamp.to_string(),
        source_graph: "urn:quipu:whole-store".to_string(),
        producer: serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "tool": "quipu pack --full",
        })
        .to_string(),
        counts: serde_json::json!({
            "tables": counts.len(),
            "rows": total,
            "per_table": counts
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            "pruned": pruned,
            "store_identity_is_lineage_not_identity": true,
        })
        .to_string(),
        destination: Some(ShareDestination::Internal),
    })
}

/// A content hash over every carried table's rows, in a deterministic order.
///
/// Not [`crate::pack::content_hash`]: that one hashes a canonical N-Triples
/// projection of CURRENT facts, which is precisely the lossy view this artifact
/// exists to replace. Hashing it here would make the full pack's identity blind
/// to the history it carries.
fn content_hash_of(conn: &Connection) -> Result<String> {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    for name in live_tables(conn)? {
        if name == "pack_manifest" {
            continue; // written after the hash; hashing it would be circular
        }
        ctx.update(b"## ");
        ctx.update(name.as_bytes());
        ctx.update(b"\n");
        // `quote(t.*)` is not valid SQLite, so the columns are enumerated and
        // quoted individually. Ordering by the concatenation rather than by
        // rowid keeps the hash insensitive to physical row order, which a
        // VACUUM is free to change.
        let cols: Vec<String> = {
            let mut c = conn.prepare(&format!("PRAGMA table_info(\"{name}\")"))?;
            c.query_map([], |r| r.get::<_, String>(1))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if cols.is_empty() {
            continue;
        }
        let expr = cols
            .iter()
            .map(|c| format!("quote(\"{c}\")"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let mut stmt = conn.prepare(&format!(
            "SELECT {expr} AS row_text FROM \"{name}\" ORDER BY row_text"
        ))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, Option<String>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for row in rows.into_iter().flatten() {
            ctx.update(row.as_bytes());
            ctx.update(b"\n");
        }
    }
    let digest = ctx.finish();
    let mut hex = String::from("sha256:");
    for b in digest.as_ref() {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok(hex)
}

#[cfg(test)]
#[path = "pack_full_tests.rs"]
mod tests;
