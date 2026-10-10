//! aegis-skj0lv: grouping moves rows and finds groups in O(1).

use std::sync::atomic::Ordering;

use super::{COMPARISONS, partition};
use crate::sparql::Bindings;
use crate::types::Value;

fn row(s: &str, n: i64) -> Bindings {
    let mut b = Bindings::new();
    b.insert("s".into(), Value::Str(s.into()));
    b.insert("n".into(), Value::Int(n));
    b
}

#[test]
fn group_by_many_keys_is_linear_not_quadratic() {
    // 20_000 distinct subjects, each seen twice. A linear group search makes
    // ~n^2/2 = 2e8 comparisons here; bucketed lookup makes one per repeat.
    let n = 20_000;
    let rows: Vec<Bindings> = (0..2 * n)
        .map(|i| row(&format!("s{}", i % n), i as i64))
        .collect();
    COMPARISONS.store(0, Ordering::Relaxed);
    let groups = partition(rows, &["s".to_string()]);
    let comparisons = COMPARISONS.load(Ordering::Relaxed);
    assert_eq!(groups.len(), n);
    assert!(groups.iter().all(|(_, rows)| rows.len() == 2));
    assert!(
        comparisons <= 2 * n,
        "comparisons {comparisons} must be O(rows)"
    );
}

#[test]
fn groups_keep_first_seen_order_and_unbound_keys() {
    let mut unbound = Bindings::new();
    unbound.insert("n".into(), Value::Int(9));
    let rows = vec![row("b", 1), row("a", 2), unbound, row("b", 3)];
    let groups = partition(rows, &["s".to_string()]);
    let keys: Vec<_> = groups.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(
        keys,
        vec![
            vec![Some(Value::Str("b".into()))],
            vec![Some(Value::Str("a".into()))],
            vec![None],
        ]
    );
    assert_eq!(groups[0].1.len(), 2);
}

#[test]
fn no_group_keys_is_one_group_even_when_empty() {
    assert_eq!(partition(Vec::new(), &[]).len(), 1);
    assert_eq!(partition(vec![row("a", 1), row("b", 2)], &[])[0].1.len(), 2);
    assert!(partition(Vec::new(), &["s".to_string()]).is_empty());
}
