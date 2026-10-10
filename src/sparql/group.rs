//! GROUP BY and aggregate evaluation that moves rows instead of copying them
//! (aegis-skj0lv).
//!
//! Every aggregate lowers to a `Group`. This used to clone each solution into
//! its group, so a `COUNT(*)` over a 773k-fact graph held two copies of every
//! row (+1.83 GB, 13 s on production data), and it found each row's group by
//! linear search, which is O(rows x groups) for a `GROUP BY ?s`.

use std::collections::HashMap;

use spargebra::algebra::AggregateExpression;
use spargebra::term::Variable;

use super::Bindings;
use super::aggregate::eval_aggregate;
use crate::store::Store;
use crate::types::Value;

type Group = (Vec<Option<Value>>, Vec<Bindings>);

/// Key comparisons made by [`partition`], for the regression test.
#[cfg(test)]
pub(super) static COMPARISONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Partition `rows` by the values of `keys`, moving each row exactly once.
/// Groups keep first-seen order. `Value` is only `PartialEq`, so candidates
/// are bucketed by a hash of their debug form and confirmed with `==`: the
/// same grouping as an exhaustive search, at hash-lookup cost.
pub(super) fn partition(rows: Vec<Bindings>, keys: &[String]) -> Vec<Group> {
    if keys.is_empty() {
        // With no GROUP BY every row is in the one group, even when there are
        // none: `SELECT (COUNT(*) AS ?n) WHERE { <no match> }` answers 0.
        return vec![(Vec::new(), rows)];
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for row in rows {
        let key: Vec<Option<Value>> = keys.iter().map(|k| row.get(k).cloned()).collect();
        let bucket = buckets.entry(format!("{key:?}")).or_default();
        let found = bucket.iter().copied().find(|&i| {
            #[cfg(test)]
            COMPARISONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            groups[i].0 == key
        });
        match found {
            Some(i) => groups[i].1.push(row),
            None => {
                bucket.push(groups.len());
                groups.push((key, vec![row]));
            }
        }
    }
    groups
}

/// Evaluate `GROUP BY variables` with `aggregates` over `rows`.
pub(super) fn evaluate(
    store: &Store,
    rows: Vec<Bindings>,
    variables: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
) -> (Vec<Bindings>, Vec<String>) {
    let keys: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();
    let agg_vars: Vec<String> = aggregates
        .iter()
        .map(|(v, _)| v.as_str().to_string())
        .collect();
    let mut result_rows = Vec::new();
    for (key, group_rows) in partition(rows, &keys) {
        let mut result_row = Bindings::new();
        for (var, value) in keys.iter().zip(key) {
            if let Some(value) = value {
                result_row.insert(var.clone(), value);
            }
        }
        for ((_, expr), var) in aggregates.iter().zip(&agg_vars) {
            if let Some(value) = eval_aggregate(store, expr, &group_rows) {
                result_row.insert(var.clone(), value);
            }
        }
        result_rows.push(result_row);
    }
    let mut vars = keys;
    vars.extend(agg_vars);
    (result_rows, vars)
}

#[cfg(test)]
#[path = "group_tests.rs"]
mod tests;
