//! Joining the baseline's judgements to the verdicts the gate RECORDED
//! (aegis-xfuch4.2). `aegis:gatedTx` when a verdict carries it (ian's schema,
//! aegis-7pphox), otherwise the historical convention: the verdict-recording
//! transaction immediately after the gated one.

use std::collections::BTreeMap;

use rusqlite::params;

use super::VerdictJoin;
use crate::error::Result;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::store::Store;
use crate::types::Value;

/// Verdicts that carry `aegis:gatedTx`, keyed by the tx they judged.
pub(super) type GatedIndex = BTreeMap<i64, Vec<(String, String, String)>>;

pub(super) fn gated_tx_index(store: &Store) -> Result<GatedIndex> {
    let mut out = GatedIndex::new();
    let Some(gated) = store.lookup(&format!("{DEFAULT_BASE_NS}gatedTx"))? else {
        return Ok(out);
    };
    let mut stmt =
        store.prepare("SELECT e, v FROM facts WHERE a = ?1 AND op = 1 AND valid_to IS NULL")?;
    let rows = stmt
        .query_map(params![gated], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (verdict, raw) in rows {
        let tx = match Value::from_bytes(&raw)? {
            Value::Int(i) => i,
            Value::Str(s) => match s.parse() {
                Ok(i) => i,
                Err(_) => continue,
            },
            _ => continue,
        };
        if let Some(v) = verdict_fields(store, verdict)? {
            out.entry(tx).or_default().push(v);
        }
    }
    Ok(out)
}

/// (predicateId, targetRef, outcome) of one verdict entity, current values.
fn verdict_fields(store: &Store, verdict: i64) -> Result<Option<(String, String, String)>> {
    let field = |name: &str| -> Result<Option<String>> {
        let Some(a) = store.lookup(&format!("{DEFAULT_BASE_NS}{name}"))? else {
            return Ok(None);
        };
        let mut stmt = store.prepare(
            "SELECT v FROM facts WHERE e = ?1 AND a = ?2 AND op = 1 AND valid_to IS NULL LIMIT 1",
        )?;
        let mut rows = stmt.query(params![verdict, a])?;
        match rows.next()? {
            Some(r) => match Value::from_bytes(&r.get::<_, Vec<u8>>(0)?)? {
                Value::Str(s) => Ok(Some(s)),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    };
    Ok(
        match (
            field("predicateId")?,
            field("targetRef")?,
            field("outcome")?,
        ) {
            (Some(p), Some(t), Some(o)) => Some((p, t, o)),
            _ => None,
        },
    )
}

/// The verdicts the historical convention attributes to `tx`: those written
/// by the verdict-recording transaction immediately after it.
fn next_tx_verdicts(store: &Store, tx: i64) -> Result<Vec<(String, String, String)>> {
    let Some(meta) = store.get_transaction(tx + 1)? else {
        return Ok(Vec::new());
    };
    if meta.source.as_deref() != Some("write-gate verdict") {
        return Ok(Vec::new());
    }
    let Some(verdict_class) = store.lookup(&format!("{DEFAULT_BASE_NS}Verdict"))? else {
        return Ok(Vec::new());
    };
    let Some(rdf_type) = store.lookup(RDF_TYPE)? else {
        return Ok(Vec::new());
    };
    let mut stmt = store.prepare("SELECT e FROM facts WHERE tx = ?1 AND a = ?2 AND v = ?3")?;
    let ids = stmt
        .query_map(
            params![tx + 1, rdf_type, Value::Ref(verdict_class).to_bytes()],
            |r| r.get::<_, i64>(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(v) = verdict_fields(store, id)? {
            out.push(v);
        }
    }
    Ok(out)
}

pub(super) fn join_verdicts(
    store: &Store,
    tx: i64,
    enforced: &[(String, String, &'static str)],
    gated: &GatedIndex,
    join: &mut VerdictJoin,
) -> Result<()> {
    if enforced.is_empty() {
        return Ok(());
    }
    let (recorded, via_gated) = match gated.get(&tx) {
        Some(v) => (v.clone(), true),
        None => (next_tx_verdicts(store, tx)?, false),
    };
    for (p, t, o) in enforced {
        join.judged += 1;
        let same_pair: Vec<&(String, String, String)> = recorded
            .iter()
            .filter(|(rp, rt, _)| rp == p && rt == t)
            .collect();
        if same_pair.is_empty() {
            join.unmatched += 1;
            continue;
        }
        if via_gated {
            join.via_gated_tx += 1;
        }
        if same_pair.iter().any(|(_, _, ro)| ro == o) {
            join.matched += 1;
        } else {
            join.outcome_mismatch += 1;
        }
    }
    Ok(())
}
