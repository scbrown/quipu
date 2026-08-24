//! Deep-freeze IO: the full-history export, the canonical history form, and
//! the frozen-pack registry read that re-attaches archives on open.
//!
//! Split from [`super::freeze`] for the file-size ratchet; one feature, two
//! files. The lifecycle logic (freeze/thaw decisions, guards, registry
//! mutation) lives there — everything here is mechanical copying and hashing.

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::types::Value;

use super::Store;
use super::attach::Attachment;

/// The attachments a store's frozen-pack registry implies, for `init`.
///
/// Runs BEFORE migrations, so the table is probed via `sqlite_master`; a
/// store that never froze anything has no table and contributes nothing.
///
/// # Errors
/// A registered pack file that no longer exists is a HARD refusal naming the
/// path and the remedy — an archive graph silently absent from composition is
/// the silent-zero-rows failure this stack refuses everywhere.
pub(crate) fn frozen_attachments(conn: &Connection) -> Result<Vec<Attachment>> {
    let has_table: bool = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='frozen_packs'")?
        .exists([])?;
    if !has_table {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT alias, path, graph_iri FROM frozen_packs WHERE thawed_at IS NULL ORDER BY id",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (alias, path, graph_iri) in rows {
        if !Path::new(&path).exists() {
            return Err(Error::Store(format!(
                "frozen graph '{graph_iri}' is registered but its archive pack \
                 {path:?} is missing. Refusing to open with the archive silently \
                 absent — restore the file, or mark the freeze thawed in \
                 `frozen_packs` if the graph was re-imported by hand."
            )));
        }
        out.push(Attachment::read_only(&alias, &path));
    }
    Ok(out)
}

/// The canonical text of one graph's FULL history, for hashing.
///
/// One line per fact row: entity and attribute as IRIs, the value as
/// `ref:<iri>` or hex bytes, the writing transaction's timestamp/actor/source,
/// the bitemporal window, the op, and the retracting transaction's timestamp.
/// Lines are sorted, so the form is independent of rowid order AND of term
/// ids — which is what lets the pre-delete main store and the respaced pack
/// hash identically.
pub(crate) fn history_canonical(conn: &Connection, g: i64) -> Result<String> {
    let mut resolve_stmt = conn.prepare("SELECT iri FROM terms WHERE id = ?1")?;
    let mut resolve = |id: i64| -> Result<String> {
        resolve_stmt
            .query_row(params![id], |r| r.get(0))
            .map_err(|_| Error::UnknownTerm(id))
    };

    let mut stmt = conn.prepare(
        "SELECT f.e, f.a, f.v, tx.timestamp, tx.actor, tx.source, \
                f.valid_from, f.valid_to, f.op, rtx.timestamp \
         FROM facts f \
         JOIN transactions tx ON tx.id = f.tx \
         LEFT JOIN transactions rtx ON rtx.id = f.retracted_tx \
         WHERE f.g = ?1",
    )?;
    type Row = (
        i64,
        i64,
        Vec<u8>,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        i64,
        Option<String>,
    );
    let rows: Vec<Row> = stmt
        .query_map(params![g], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;

    let mut lines = Vec::with_capacity(rows.len());
    for (e, a, v, tx_ts, actor, source, from, to, op, retracted_ts) in rows {
        let value = match Value::from_bytes(&v)? {
            Value::Ref(id) => format!("ref:{}", resolve(id)?),
            other => format!("val:{}", hex::encode(other.to_bytes())),
        };
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            resolve(e)?,
            resolve(a)?,
            value,
            tx_ts,
            actor.as_deref().unwrap_or("-"),
            source.as_deref().unwrap_or("-"),
            from,
            to.as_deref().unwrap_or("-"),
            op,
            retracted_ts.as_deref().unwrap_or("-"),
        ));
    }
    lines.sort_unstable();
    Ok(format!("## history v2\n{}\n", lines.join("\n")))
}

/// Export one graph's full history — every row, retracted included, with its
/// transactions — into a fresh store at `build_path`, and stamp a
/// `pack_format: "2"` manifest carrying `content_hash`.
///
/// The current-facts `pack()` is NOT sufficient for freezing (it re-asserts
/// live facts under one new transaction, discarding history), which is why
/// this exists. Terms and `Ref` payloads are re-interned by IRI through the
/// destination's own dictionary — correct by construction, the `pack_into`
/// discipline.
pub(crate) fn export_graph_history(
    store: &Store,
    graph_iri: &str,
    g: i64,
    build_path: &str,
    content_hash: &str,
    timestamp: &str,
) -> Result<(usize, usize)> {
    let out = Store::open(build_path)?;
    let out_g = out.graph_create(graph_iri)?;

    // Transactions referenced by this graph's rows, writing AND retracting,
    // copied with their identity fields so the pack replays honestly.
    let mut stmt = store.conn.prepare(
        "SELECT id, timestamp, actor, source FROM transactions WHERE id IN \
           (SELECT tx FROM facts WHERE g = ?1 \
            UNION SELECT retracted_tx FROM facts WHERE g = ?1 AND retracted_tx IS NOT NULL) \
         ORDER BY id",
    )?;
    let txs: Vec<(i64, String, Option<String>, Option<String>)> = stmt
        .query_map(params![g], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut tx_map = HashMap::with_capacity(txs.len());
    for (old, ts, actor, source) in &txs {
        out.conn.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![ts, actor, source],
        )?;
        tx_map.insert(*old, out.conn.last_insert_rowid());
    }

    let mut stmt = store.conn.prepare(
        "SELECT e, a, v, tx, valid_from, valid_to, op, retracted_tx \
         FROM facts WHERE g = ?1 ORDER BY rowid",
    )?;
    type FactRow = (
        i64,
        i64,
        Vec<u8>,
        i64,
        String,
        Option<String>,
        i64,
        Option<i64>,
    );
    let rows: Vec<FactRow> = stmt
        .query_map(params![g], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let fact_count = rows.len();
    for (e, a, v, tx, from, to, op, retracted) in rows {
        let e = out.intern(&store.resolve(e)?)?;
        let a = out.intern(&store.resolve(a)?)?;
        let v = match Value::from_bytes(&v)? {
            Value::Ref(id) => Value::Ref(out.intern(&store.resolve(id)?)?).to_bytes(),
            other => other.to_bytes(),
        };
        let tx = *tx_map
            .get(&tx)
            .ok_or_else(|| Error::Store(format!("freeze: unmapped transaction {tx}")))?;
        let retracted = match retracted {
            Some(old) => Some(*tx_map.get(&old).ok_or_else(|| {
                Error::Store(format!("freeze: unmapped retracting transaction {old}"))
            })?),
            None => None,
        };
        out.conn.execute(
            "INSERT INTO facts (e,a,v,g,tx,valid_from,valid_to,op,retracted_tx) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![e, a, v, out_g, tx, from, to, op, retracted],
        )?;
    }

    // The pack's own registry row carries the ARCHIVE label: kind=archive and
    // durability=backed describe what the pack IS, while freshness and policy
    // travel from the source graph. Trust is deliberately dropped — a trust
    // rank is anchored to a chain in the ORIGIN store's meta-graph and does
    // not survive relocation; the consumer's own floors decide.
    let l = store.label_of_id(g)?;
    let mut out = out;
    let label = super::labels::GraphLabel {
        freshness: l.freshness.value,
        durability: Some(crate::lattice::Durability::Backed),
        trust: None,
        policy: l.policy.value.clone(),
        kind: Some(crate::lattice_kind::DataKind::parse(
            crate::lattice_kind::KIND_ARCHIVE,
        )?),
    };
    out.set_graph_label(graph_iri, &label, timestamp, None)?;

    out.conn.execute_batch(crate::pack::MANIFEST_SQL)?;
    out.conn.execute(
        "INSERT OR REPLACE INTO pack_manifest \
         (id, pack_format, name, version, term_space, content_hash, created_at, \
          source_graph, producer, counts) \
         VALUES (1, '2', ?1, '0.1.0', 0, ?2, ?3, ?4, ?5, ?6)",
        params![
            crate::pack::local_name(graph_iri),
            content_hash,
            timestamp,
            graph_iri,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "tool": "quipu graph freeze",
            })
            .to_string(),
            serde_json::json!({
                "facts": fact_count,
                "transactions": txs.len(),
                "history": true,
            })
            .to_string(),
        ],
    )?;
    Ok((fact_count, txs.len()))
}

/// Verify a frozen pack: recompute the canonical history from the PACK's own
/// rows and compare against `expected_hash`. Opened read-only — verification
/// must not be able to repair what it is checking.
pub(crate) fn verify_history_pack(path: &str, graph_iri: &str, expected_hash: &str) -> Result<()> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let g: Option<i64> = conn
        .query_row(
            "SELECT id FROM terms WHERE iri = ?1",
            params![graph_iri],
            |r| r.get(0),
        )
        .optional()?;
    let Some(g) = g else {
        return Err(Error::Store(format!(
            "pack {path:?} does not intern graph '{graph_iri}'"
        )));
    };
    let canonical = history_canonical(&conn, g)?;
    let actual = crate::pack::content_hash(&canonical);
    if actual != expected_hash {
        return Err(Error::Store(format!(
            "frozen pack {path:?} fails verification: content hash {actual} != \
             expected {expected_hash}. The pack file is left in place for \
             inspection; nothing was deleted."
        )));
    }
    Ok(())
}

/// Import one graph's full history back from a frozen pack into the local
/// store, under the same IRI — the thaw copy. Graph-FILTERED, unlike
/// `import::import_graph`: the pack also holds its own meta-graph label
/// facts, which must not be imported as data.
pub(crate) fn import_graph_history(
    store: &Store,
    pack_path: &str,
    graph_iri: &str,
) -> Result<usize> {
    let src = Connection::open_with_flags(
        pack_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let src_g: i64 = src
        .query_row(
            "SELECT id FROM terms WHERE iri = ?1",
            params![graph_iri],
            |r| r.get(0),
        )
        .map_err(|_| Error::Store(format!("pack {pack_path:?} does not intern '{graph_iri}'")))?;
    let local_g = store.intern(graph_iri)?;

    let mut stmt = src.prepare(
        "SELECT id, timestamp, actor, source FROM transactions WHERE id IN \
           (SELECT tx FROM facts WHERE g = ?1 \
            UNION SELECT retracted_tx FROM facts WHERE g = ?1 AND retracted_tx IS NOT NULL) \
         ORDER BY id",
    )?;
    let txs: Vec<(i64, String, Option<String>, Option<String>)> = stmt
        .query_map(params![src_g], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut tx_map = HashMap::with_capacity(txs.len());
    for (old, ts, actor, source) in txs {
        store.conn.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![ts, actor, source],
        )?;
        tx_map.insert(old, store.conn.last_insert_rowid());
    }

    let mut resolve_stmt = src.prepare("SELECT iri FROM terms WHERE id = ?1")?;
    let mut src_resolve = |id: i64| -> Result<String> {
        resolve_stmt
            .query_row(params![id], |r| r.get(0))
            .map_err(|_| Error::UnknownTerm(id))
    };

    let mut stmt = src.prepare(
        "SELECT e, a, v, tx, valid_from, valid_to, op, retracted_tx \
         FROM facts WHERE g = ?1 ORDER BY rowid",
    )?;
    type FactRow = (
        i64,
        i64,
        Vec<u8>,
        i64,
        String,
        Option<String>,
        i64,
        Option<i64>,
    );
    let rows: Vec<FactRow> = stmt
        .query_map(params![src_g], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut facts = 0usize;
    for (e, a, v, tx, from, to, op, retracted) in rows {
        let e = store.intern(&src_resolve(e)?)?;
        let a = store.intern(&src_resolve(a)?)?;
        let v = match Value::from_bytes(&v)? {
            Value::Ref(id) => Value::Ref(store.intern(&src_resolve(id)?)?).to_bytes(),
            other => other.to_bytes(),
        };
        let tx = *tx_map
            .get(&tx)
            .ok_or_else(|| Error::Store(format!("thaw: unmapped transaction {tx}")))?;
        let retracted = match retracted {
            Some(old) => Some(*tx_map.get(&old).ok_or_else(|| {
                Error::Store(format!("thaw: unmapped retracting transaction {old}"))
            })?),
            None => None,
        };
        store.conn.execute(
            "INSERT INTO main.facts (e,a,v,g,tx,valid_from,valid_to,op,retracted_tx) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![e, a, v, local_g, tx, from, to, op, retracted],
        )?;
        facts += 1;
    }
    Ok(facts)
}
