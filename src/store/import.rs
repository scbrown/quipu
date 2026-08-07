//! Offline import-with-remap into one local named graph (quipu #85).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::types::Value;

/// Counts produced by [`import_graph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub terms: usize,
    pub transactions: usize,
    pub facts: usize,
    pub graph: i64,
}

/// Import every fact from `src` into the local named graph `graph_iri` in `dst`.
///
/// The source is opened read-only. Term identity is resolved by IRI into the
/// destination dictionary, so an IRI present in both files has exactly one
/// local id. The live source schema is classified before the destination is
/// touched; an unknown column refuses the operation instead of being skipped.
pub fn import_graph(dst: &Path, src: &Path, graph_iri: &str) -> Result<ImportReport> {
    if dst == src {
        return Err(Error::Store(
            "graph import refuses: source and destination are the same file".into(),
        ));
    }
    if !src.exists() {
        return Err(Error::Store(format!("no such store: {}", src.display())));
    }

    // Migrate both views with the current binary. The source itself stays
    // read-only: migrate a VACUUM snapshot when an old schema needs upgrading.
    let dst_path = dst.to_string_lossy().to_string();
    drop(super::Store::open(&dst_path)?);
    let source = Connection::open_with_flags(
        src,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    super::respace::classify_live_schema(&source)?;

    let mut dest = Connection::open(&dst_path)?;
    dest.execute_batch("PRAGMA foreign_keys = ON;")?;
    let tx = dest.transaction()?;

    let source_terms: Vec<(i64, String)> = source
        .prepare("SELECT id, iri FROM terms ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut ids = HashMap::with_capacity(source_terms.len());
    for (foreign, iri) in &source_terms {
        ids.insert(*foreign, super::intern_in_space(&tx, iri)?);
    }
    let graph = super::intern_in_space(&tx, graph_iri)?;
    tx.execute(
        "INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at, source) \
         VALUES (?1, 'committed', NULL, '1970-01-01T00:00:00Z', NULL)",
        params![graph],
    )?;

    let source_txs: Vec<(i64, String, Option<String>, Option<String>)> = source
        .prepare("SELECT id, timestamp, actor, source FROM transactions ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut tx_ids = HashMap::with_capacity(source_txs.len());
    for (old, timestamp, actor, origin) in source_txs {
        tx.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![timestamp, actor, origin],
        )?;
        tx_ids.insert(old, tx.last_insert_rowid());
    }

    let mut stmt = source.prepare(
        "SELECT e, a, v, tx, valid_from, valid_to, op, retracted_tx FROM facts ORDER BY rowid",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, Option<i64>>(7)?,
        ))
    })?;
    let mut facts = 0;
    let mut referenced = HashSet::new();
    for row in rows {
        let (e, a, blob, old_tx, from, to, op, old_retracted) = row?;
        let e = mapped(&ids, e)?;
        let a = mapped(&ids, a)?;
        let value = match Value::from_bytes(&blob)? {
            Value::Ref(id) => {
                let id = mapped(&ids, id)?;
                referenced.insert(id);
                Value::Ref(id).to_bytes()
            }
            other => other.to_bytes(),
        };
        let local_tx = mapped(&tx_ids, old_tx)?;
        let retracted = old_retracted.map(|id| mapped(&tx_ids, id)).transpose()?;
        tx.execute(
            "INSERT INTO facts (e,a,v,g,tx,valid_from,valid_to,op,retracted_tx) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![e, a, value, graph, local_tx, from, to, op, retracted],
        )?;
        facts += 1;
    }

    for id in referenced {
        if tx
            .query_row("SELECT 1 FROM terms WHERE id=?1", params![id], |_| Ok(()))
            .optional()?
            .is_none()
        {
            return Err(Error::Store(format!(
                "graph import post-condition failed: Ref({id}) has no local term"
            )));
        }
    }
    tx.commit()?;
    Ok(ImportReport {
        terms: source_terms.len(),
        transactions: tx_ids.len(),
        facts,
        graph,
    })
}

fn mapped(map: &HashMap<i64, i64>, id: i64) -> Result<i64> {
    map.get(&id).copied().ok_or_else(|| {
        Error::Store(format!(
            "graph import refuses: term/transaction id {id} has no source dictionary entry"
        ))
    })
}

#[cfg(test)]
mod tests;
