//! SQL `IN`-clause builders for the triple-pattern compiler — graph ids, term
//! ids, and `Value::Ref` blobs, each pushing bound params. Split from
//! `sparql/triple.rs` for the file-size ratchet.

use crate::types::Value;

/// Build a graph-membership SQL condition over `facts.g`, pushing the ids as
/// bound params (quipu #36). An EMPTY set yields `0 = 1` — match nothing, never
/// a silent fall-through (e.g. a `FROM NAMED` with no `FROM` has an empty
/// default graph). A single id yields `g = ?N`; several yield `g IN (…)`.
pub(super) fn sql_graph_in(
    gids: &[i64],
    sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> String {
    if gids.is_empty() {
        return "0 = 1".to_string();
    }
    let placeholders: Vec<String> = gids
        .iter()
        .map(|gid| {
            sql_params.push(Box::new(*gid));
            format!("?{}", sql_params.len())
        })
        .collect();
    if placeholders.len() == 1 {
        format!("g = {}", placeholders[0])
    } else {
        format!("g IN ({})", placeholders.join(", "))
    }
}

/// Build an equality/IN predicate for a term-id column. The one-id spelling is
/// intentionally the pre-#76 plan; only composed stores widen it.
pub(super) fn sql_id_in(
    column: &str,
    ids: &[i64],
    sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> String {
    if ids.is_empty() {
        return "0 = 1".to_string();
    }
    let placeholders: Vec<String> = ids
        .iter()
        .map(|id| {
            sql_params.push(Box::new(*id));
            format!("?{}", sql_params.len())
        })
        .collect();
    if placeholders.len() == 1 {
        format!("{column} = {}", placeholders[0])
    } else {
        format!("{column} IN ({})", placeholders.join(", "))
    }
}

pub(super) fn sql_ref_in(
    ids: &[i64],
    sql_params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> String {
    if ids.is_empty() {
        return "0 = 1".to_string();
    }
    let placeholders: Vec<String> = ids
        .iter()
        .map(|id| {
            sql_params.push(Box::new(Value::Ref(*id).to_bytes()));
            format!("?{}", sql_params.len())
        })
        .collect();
    if placeholders.len() == 1 {
        format!("v = {}", placeholders[0])
    } else {
        format!("v IN ({})", placeholders.join(", "))
    }
}
