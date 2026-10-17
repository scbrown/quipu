//! Store-aware ORDER BY keys, evaluated once per solution.
use super::{Bindings, TemporalContext};
use crate::{
    error::{Error, Result},
    store::Store,
    types::Value,
};
use spargebra::algebra::OrderExpression;

#[cfg(not(target_arch = "wasm32"))]
mod spill;
mod top;
pub(super) use top::top_rows;

pub(super) fn sort_rows(
    store: &Store,
    rows: Vec<Bindings>,
    expression: &[OrderExpression],
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    // Evaluate each key once per solution, including volatile expressions.
    // Ref IDs encode insertion order; only resolved strings order IRIs.
    let mut iri_keys: std::collections::HashMap<i64, std::sync::Arc<str>> =
        std::collections::HashMap::new();
    check_deadline(ctx)?;
    // Bound retained key storage independently of the result-row cap. Include
    // slot storage, owned payload capacities and unique cached IRI strings.
    // A single evaluated expression is transient until it passes this budget.
    let entries = rows
        .len()
        .checked_mul(expression.len())
        .ok_or_else(budget_error)?;
    #[cfg(not(target_arch = "wasm32"))]
    if rows.len() > 4096 || entries > MAX_KEY_ENTRIES {
        return spill::sort(store, rows, expression, ctx);
    }
    if entries > MAX_KEY_ENTRIES {
        return Err(budget_error());
    }
    let mut bytes = entries
        .checked_mul(std::mem::size_of::<Key>())
        .and_then(|n| {
            rows.len()
                .checked_mul(
                    std::mem::size_of::<Vec<Key>>()
                        + std::mem::size_of::<Bindings>()
                        + 4 * std::mem::size_of::<usize>(),
                )
                .and_then(|overhead| n.checked_add(overhead))
        })
        .ok_or_else(budget_error)?;
    charge(&mut bytes, 0)?;
    let mut keyed = Vec::with_capacity(rows.len());
    for row in rows {
        check_deadline(ctx)?;
        let mut keys = Vec::with_capacity(expression.len());
        for order in expression {
            check_deadline(ctx)?;
            let (OrderExpression::Asc(expr) | OrderExpression::Desc(expr)) = order;
            let value = super::filter::eval_expr(store, expr, &row);
            check_deadline(ctx)?;
            charge(&mut bytes, payload_bytes(value.as_ref()))?;
            let iri = if let Some(Value::Ref(id)) = &value {
                Some(match iri_keys.entry(*id) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let lexical = store.resolve(*id)?;
                        // Conservatively include hash-node and Arc overhead.
                        charge(&mut bytes, lexical.len().saturating_add(128))?;
                        let lexical: std::sync::Arc<str> = lexical.into();
                        entry.insert(lexical.clone());
                        lexical
                    }
                })
            } else {
                None
            };
            keys.push((value, iri));
        }
        keyed.push((row, keys));
    }
    let compare = |a: &Vec<Key>, b: &Vec<Key>| {
        for ((order, (va, ia)), (vb, ib)) in expression.iter().zip(a).zip(b) {
            let cmp = match (ia, ib) {
                (Some(a), Some(b)) => a.cmp(b),
                _ => super::aggregate::compare_option_values(va, vb),
            };
            let cmp = if matches!(order, OrderExpression::Asc(_)) {
                cmp
            } else {
                cmp.reverse()
            };
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    };
    // Fallible stable merge sort: cancellation never changes comparator results
    // mid-sort. Only indices move; keys are evaluated once and rows not cloned.
    let mut indices: Vec<usize> = (0..keyed.len()).collect();
    let mut scratch = vec![0; indices.len()];
    let mut width = 1;
    while width < indices.len() {
        for start in (0..indices.len()).step_by(width * 2) {
            let mid = (start + width).min(indices.len());
            let end = (mid + width).min(indices.len());
            let (mut left, mut right) = (start, mid);
            for slot in &mut scratch[start..end] {
                check_deadline(ctx)?;
                if left < mid
                    && (right == end
                        || compare(&keyed[indices[left]].1, &keyed[indices[right]].1)
                            != std::cmp::Ordering::Greater)
                {
                    *slot = indices[left];
                    left += 1;
                } else {
                    *slot = indices[right];
                    right += 1;
                }
            }
        }
        std::mem::swap(&mut indices, &mut scratch);
        width *= 2;
    }
    let mut positions: Vec<usize> = (0..keyed.len()).collect();
    let mut originals = positions.clone();
    for (target, original) in indices.into_iter().enumerate() {
        check_deadline(ctx)?;
        let source = positions[original];
        let displaced = originals[target];
        keyed.swap(target, source);
        originals.swap(target, source);
        positions[original] = target;
        positions[displaced] = source;
    }
    Ok(keyed.into_iter().map(|(row, _)| row).collect())
}

type Key = (Option<Value>, Option<std::sync::Arc<str>>);
const MAX_KEY_BYTES: usize = 8 * 1024 * 1024;
const MAX_KEY_ENTRIES: usize = 65_536;

fn budget_error() -> Error {
    Error::InvalidValue(format!(
        "ORDER BY retained key budget exceeded ({MAX_KEY_ENTRIES} entries / {MAX_KEY_BYTES} bytes)"
    ))
}

fn charge(bytes: &mut usize, additional: usize) -> Result<()> {
    *bytes = bytes.checked_add(additional).ok_or_else(budget_error)?;
    if *bytes > MAX_KEY_BYTES {
        return Err(budget_error());
    }
    Ok(())
}

fn payload_bytes(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Str(s)) => s.capacity(),
        Some(Value::Bytes(s)) => s.capacity(),
        Some(Value::Lang { lexical, lang }) => lexical.capacity().saturating_add(lang.capacity()),
        Some(Value::Typed { lexical, datatype }) => {
            lexical.capacity().saturating_add(datatype.capacity())
        }
        _ => 0,
    }
}

fn check_deadline(ctx: &TemporalContext) -> Result<()> {
    if ctx.deadline.is_some_and(|dl| dl.passed()) {
        return Err(Error::QueryTimeout {
            elapsed_ms: 0,
            limit_ms: 0,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_preserves_provenance_header_from_a_nonwinning_row() {
        let store = Store::open_in_memory().unwrap();
        let result = crate::sparql::query(&store,
            "SELECT ?x WHERE { VALUES (?x ?_provider) { (1 UNDEF) (2 \"source\") } } ORDER BY ?x LIMIT 1").unwrap();
        assert_eq!(
            result.variables(),
            &["x".to_owned(), "_provider".to_owned()]
        );
        assert_eq!(result.rows().len(), 1);
        assert_eq!(result.rows()[0]["x"], Value::Int(1));
        assert!(!result.rows()[0].contains_key("_provider"));
    }

    #[test]
    fn mixed_incomparable_keys_and_computed_keys_keep_full_sort_prefix() {
        let store = Store::open_in_memory().unwrap();
        for keys in ["10 \"x\" 1", "\"x\" 10 1", "true 3 false 1", "2.0 1.0 3.0"] {
            for order in ["?x", "DESC(?x)", "(?x + 1)"] {
                let query =
                    format!("SELECT ?x WHERE {{ VALUES ?x {{ {keys} }} }} ORDER BY {order}");
                let all = crate::sparql::query(&store, &query).unwrap();
                let limited = crate::sparql::query(&store, &format!("{query} LIMIT 2")).unwrap();
                assert_eq!(limited.rows(), &all.rows()[..2], "{query}");
            }
        }
    }

    #[test]
    fn limit_retains_only_winners_and_offset_preserves_stable_ties() {
        let store = Store::open_in_memory().unwrap();
        let values = (0..2000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let query = format!(
            "SELECT ?x WHERE {{ VALUES ?x {{ {values} }} }} ORDER BY (\"{}\") DESC(?x) LIMIT 2 OFFSET 1",
            "k".repeat(16384)
        );
        let result = crate::sparql::query(&store, &query).unwrap();
        assert_eq!(
            result
                .rows()
                .iter()
                .map(|r| r["x"].clone())
                .collect::<Vec<_>>(),
            vec![Value::Int(1998), Value::Int(1997)]
        );
        let result = crate::sparql::query(&store,
            "SELECT ?id WHERE { VALUES (?key ?id) { (2 9) (1 4) (1 3) (2 8) } } ORDER BY ?key LIMIT 2 OFFSET 1").unwrap();
        assert_eq!(
            result
                .rows()
                .iter()
                .map(|r| r["id"].clone())
                .collect::<Vec<_>>(),
            vec![Value::Int(3), Value::Int(9)]
        );
    }

    #[test]
    fn limit_does_not_cross_distinct_or_constrain_nested_sort() {
        let store = Store::open_in_memory().unwrap();
        let result = crate::sparql::query(
            &store,
            "SELECT DISTINCT ?x WHERE { VALUES ?x { 3 3 2 1 } } ORDER BY DESC(?x) LIMIT 2",
        )
        .unwrap();
        assert_eq!(
            result
                .rows()
                .iter()
                .map(|r| r["x"].clone())
                .collect::<Vec<_>>(),
            vec![Value::Int(3), Value::Int(2)]
        );
        let result = crate::sparql::query(&store,
            "SELECT ?x WHERE { { SELECT ?x WHERE { VALUES ?x { 1 2 3 4 } } ORDER BY DESC(?x) } } ORDER BY ?x LIMIT 1").unwrap();
        assert_eq!(result.rows()[0]["x"], Value::Int(1));
    }

    #[test]
    fn complete_order_exceeds_key_slots_without_raising_retained_budget() {
        let store = Store::open_in_memory().unwrap();
        let rows = (0..70_000)
            .rev()
            .map(|n| Bindings::from([("x".to_owned(), Value::Int(n))]))
            .collect();
        let expression = OrderExpression::Asc(spargebra::algebra::Expression::Variable(
            spargebra::term::Variable::new("x").unwrap(),
        ));
        let sorted = sort_rows(&store, rows, &[expression], &TemporalContext::default()).unwrap();
        assert_eq!(sorted.len(), 70_000);
        for (n, row) in sorted.iter().enumerate() {
            assert_eq!(row["x"], Value::Int(n as i64));
        }
    }

    #[test]
    fn payload_capacities_and_exact_byte_boundary_are_charged() {
        let mut bytes = MAX_KEY_BYTES - 1;
        charge(&mut bytes, 1).unwrap();
        assert!(charge(&mut bytes, 1).is_err());
        let mut overflow = usize::MAX;
        assert!(charge(&mut overflow, 1).is_err());
        let mut value = String::with_capacity(1024);
        value.push('x');
        assert_eq!(payload_bytes(Some(&Value::Str(value))), 1024);
    }

    #[test]
    fn expired_sort_refuses_even_with_no_rows() {
        let store = Store::open_in_memory().unwrap();
        let ctx = TemporalContext {
            deadline: Some(crate::time::Deadline::after_millis(0)),
            ..Default::default()
        };
        assert!(matches!(
            sort_rows(&store, vec![], &[], &ctx),
            Err(Error::QueryTimeout { .. })
        ));
    }

    #[test]
    fn many_entries_spill_natively_or_refuse_without_filesystem() {
        let store = Store::open_in_memory().unwrap();
        let rows = vec![Bindings::new(); MAX_KEY_ENTRIES + 1];
        let expr = OrderExpression::Asc(spargebra::algebra::Expression::Variable(
            spargebra::term::Variable::new("missing").unwrap(),
        ));
        let result = sort_rows(&store, rows, &[expr], &TemporalContext::default());
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(result.unwrap().len(), MAX_KEY_ENTRIES + 1);
        #[cfg(target_arch = "wasm32")]
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("retained key budget")
        );
    }

    #[test]
    fn equal_keys_preserve_input_order() {
        let store = Store::open_in_memory().unwrap();
        let result = crate::sparql::query(
            &store,
            "SELECT ?id WHERE { VALUES (?key ?id) { (2 9) (1 4) (1 3) (2 8) } } ORDER BY ?key",
        )
        .unwrap();
        let ids: Vec<_> = result.rows().iter().map(|row| row["id"].clone()).collect();
        assert_eq!(
            ids,
            vec![Value::Int(4), Value::Int(3), Value::Int(9), Value::Int(8)]
        );
    }

    #[test]
    fn large_literal_keys_refuse_without_retaining_rows_times_key_bytes() {
        let store = Store::open_in_memory().unwrap();
        let values = (0..2000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let keys = (0..4)
            .map(|n| format!("(\"{}{}\")", "x".repeat(16384), n))
            .collect::<Vec<_>>()
            .join(" ");
        let query = format!("SELECT ?x WHERE {{ VALUES ?x {{ {values} }} }} ORDER BY {keys}");
        let err = crate::sparql::query(&store, &query).unwrap_err();
        println!("ORDER BY 2000 rows x 4 keys x 16385 bytes: {err}");
        assert!(
            err.to_string()
                .contains("ORDER BY retained key budget exceeded")
        );
        let result = crate::sparql::query(
            &store,
            "SELECT ?x WHERE { VALUES ?x { 3 1 2 1 } } ORDER BY ?x",
        )
        .unwrap();
        let values: Vec<_> = result.rows().iter().map(|row| row["x"].clone()).collect();
        assert_eq!(
            values,
            vec![Value::Int(1), Value::Int(1), Value::Int(2), Value::Int(3)]
        );
        println!("ORDER BY ordinary numeric control: {values:?}");
    }
}

#[cfg(test)]
mod numeric_tests;
