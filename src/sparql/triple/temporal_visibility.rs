//! Fact visibility at independent valid-time and transaction-time cuts.

use rusqlite::types::ToSql;

use crate::sparql::TemporalContext;

pub(super) fn add_conditions(
    ctx: &TemporalContext,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn ToSql>>,
) {
    let tx_parameter = ctx.as_of_tx.map(|tx| {
        params.push(Box::new(tx) as Box<dyn ToSql>);
        params.len()
    });
    if let Some(at) = &ctx.valid_at {
        params.push(Box::new(at.clone()));
        let date_parameter = params.len();
        conditions.push(format!("valid_from <= ?{date_parameter}"));
        // A later closure must not rewrite the interval that was known at
        // the pinned transaction. close_assertion only closes open rows:
        // a future retracted_tx therefore certifies an originally open end.
        // Explicit finite ends (no retracted_tx) retain their original bound.
        let future_close = tx_parameter
            .map(|p| format!(" OR retracted_tx > ?{p}"))
            .unwrap_or_default();
        conditions.push(format!(
            "(valid_to IS NULL OR valid_to > ?{date_parameter}{future_close})"
        ));
    } else if let Some(p) = tx_parameter {
        // Legacy closures without a recorded retracting transaction remain
        // unknown: never guess an interval or make a closed row current.
        conditions.push(format!("(valid_to IS NULL OR retracted_tx > ?{p})"));
    } else {
        conditions.push("valid_to IS NULL".into());
    }
    if let Some(p) = tx_parameter {
        conditions.push(format!("tx <= ?{p}"));
    }
}
