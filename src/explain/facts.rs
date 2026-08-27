//! Fact-log reads used by the derivation walk.

use rusqlite::params;
use serde_json::{Value as Json, json};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

pub(super) fn fact_row(
    store: &Store,
    graphs: &[i64],
    e: i64,
    a: i64,
    value: &Value,
) -> Result<Option<(i64, Option<String>, String)>> {
    let mut stmt = store.conn.prepare(&format!(
        "SELECT f.tx, t.source, f.valid_from FROM facts f \
         JOIN transactions t ON f.tx = t.id \
         WHERE f.e = ?1 AND f.a = ?2 AND f.v = ?3 \
           AND f.op = 1 AND f.valid_to IS NULL AND f.g IN ({}) \
         ORDER BY f.tx DESC LIMIT 1",
        graph_list(graphs)
    ))?;
    let mut rows = stmt.query(params![e, a, value.to_bytes()])?;
    match rows.next()? {
        Some(row) => Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?))),
        None => Ok(None),
    }
}

fn graph_list(graphs: &[i64]) -> String {
    graphs
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn refs_for(store: &Store, graphs: &[i64], e: i64, a: i64) -> Result<Vec<i64>> {
    let mut stmt = store.conn.prepare(&format!(
        "SELECT v FROM facts WHERE e = ?1 AND a = ?2 \
         AND op = 1 AND valid_to IS NULL AND g IN ({})",
        graph_list(graphs)
    ))?;
    collect_refs(stmt.query(params![e, a])?)
}

pub(super) fn subjects_for(store: &Store, graphs: &[i64], a: i64, o: i64) -> Result<Vec<i64>> {
    let o_bytes = Value::Ref(o).to_bytes();
    let mut stmt = store.conn.prepare(&format!(
        "SELECT e FROM facts WHERE a = ?1 AND v = ?2 \
         AND op = 1 AND valid_to IS NULL AND g IN ({})",
        graph_list(graphs)
    ))?;
    let mut rows = stmt.query(params![a, o_bytes])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row.get(0)?);
    }
    Ok(out)
}

pub(super) fn exists_ref(store: &Store, graphs: &[i64], e: i64, a: i64, o: i64) -> Result<bool> {
    Ok(fact_row(store, graphs, e, a, &Value::Ref(o))?.is_some())
}

pub(super) fn ref_pairs_for(store: &Store, graphs: &[i64], a: i64) -> Result<Vec<(i64, i64)>> {
    let mut stmt = store.conn.prepare(&format!(
        "SELECT e, v FROM facts WHERE a = ?1 \
         AND op = 1 AND valid_to IS NULL AND g IN ({})",
        graph_list(graphs)
    ))?;
    let mut rows = stmt.query(params![a])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let e: i64 = row.get(0)?;
        if let Value::Ref(o) = Value::from_bytes(&row.get::<_, Vec<u8>>(1)?)? {
            out.push((e, o));
        }
    }
    Ok(out)
}

fn collect_refs(mut rows: rusqlite::Rows<'_>) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        if let Value::Ref(o) = Value::from_bytes(&row.get::<_, Vec<u8>>(0)?)? {
            out.push(o);
        }
    }
    Ok(out)
}

pub(super) fn display_value(store: &Store, value: &Value) -> Result<Json> {
    Ok(match value {
        Value::Ref(id) => json!(store.resolve(*id)?),
        other => json!(format!("{other:?}")),
    })
}
