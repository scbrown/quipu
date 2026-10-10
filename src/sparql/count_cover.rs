//! Current singleton-graph COUNT projection from existing covering indexes.
use super::{GraphScope, TemporalContext};
use crate::{Result, Store};

pub(super) fn projection(
    store: &Store,
    ctx: &TemporalContext,
    conditions: &[String],
) -> Result<Option<String>> {
    if store.has_attachments() || ctx.valid_at.is_some() || ctx.as_of_tx.is_some() {
        return Ok(None);
    }
    let (GraphScope::Default(graphs) | GraphScope::Named(graphs)) = &ctx.graph else {
        return Ok(None);
    };
    let [graph] = graphs.as_slice() else {
        return Ok(None);
    };
    let present: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='facts'
         AND name IN ('idx_geav','idx_current_g')",
        [],
        |row| row.get(0),
    )?;
    if present != 2 {
        return Ok(None);
    }
    // Both projections refer to the SAME physical row, not just its subject.
    // The partial current-g index proves currentness; geav supplies graph and
    // terms without reading wide facts-table pages. DISTINCT is still required
    // for repeated assertions of a current triple across transactions.
    let mut predicates: Vec<String> = conditions
        .iter()
        .filter(|condition| !matches!(condition.as_str(), "op = 1" | "valid_to IS NULL"))
        .cloned()
        .collect();
    predicates.push(format!(
        "EXISTS (SELECT 1 FROM facts AS live INDEXED BY idx_current_g
         WHERE live.g={graph} AND live.rowid=facts.rowid
         AND live.op=1 AND live.valid_to IS NULL)"
    ));
    Ok(Some(format!(
        "SELECT DISTINCT e,a,v FROM facts INDEXED BY idx_geav WHERE {}",
        predicates.join(" AND ")
    )))
}
