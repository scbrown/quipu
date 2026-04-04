//! Aggregate expression evaluation (COUNT, SUM, AVG, MIN, MAX, SAMPLE, `GROUP_CONCAT`).

use spargebra::algebra::{AggregateExpression, AggregateFunction};

use crate::store::Store;
use crate::types::Value;

use super::Bindings;
use super::filter::eval_expr;

/// Evaluate an aggregate expression over a group of rows.
pub fn eval_aggregate(store: &Store, agg: &AggregateExpression, rows: &[Bindings]) -> Value {
    match agg {
        AggregateExpression::CountSolutions { distinct } => {
            if *distinct {
                let mut seen: Vec<&Bindings> = Vec::new();
                let count = rows
                    .iter()
                    .filter(|r| {
                        if seen.contains(r) {
                            false
                        } else {
                            seen.push(r);
                            true
                        }
                    })
                    .count();
                Value::Int(count as i64)
            } else {
                Value::Int(rows.len() as i64)
            }
        }
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => {
            let mut values: Vec<Value> = rows
                .iter()
                .filter_map(|row| eval_expr(store, expr, row))
                .collect();
            if *distinct {
                let mut deduped = Vec::new();
                for v in values {
                    if !deduped.contains(&v) {
                        deduped.push(v);
                    }
                }
                values = deduped;
            }
            match name {
                AggregateFunction::Count => Value::Int(values.len() as i64),
                AggregateFunction::Sum => {
                    let mut sum = 0.0f64;
                    let mut all_int = true;
                    for v in &values {
                        match v {
                            Value::Int(n) => sum += *n as f64,
                            Value::Float(f) => {
                                sum += f;
                                all_int = false;
                            }
                            _ => {}
                        }
                    }
                    if all_int {
                        Value::Int(sum as i64)
                    } else {
                        Value::Float(sum)
                    }
                }
                AggregateFunction::Avg => {
                    if values.is_empty() {
                        return Value::Int(0);
                    }
                    let mut sum = 0.0f64;
                    let mut count = 0usize;
                    for v in &values {
                        match v {
                            Value::Int(n) => {
                                sum += *n as f64;
                                count += 1;
                            }
                            Value::Float(f) => {
                                sum += f;
                                count += 1;
                            }
                            _ => {}
                        }
                    }
                    if count == 0 {
                        Value::Int(0)
                    } else {
                        Value::Float(sum / count as f64)
                    }
                }
                AggregateFunction::Min => values
                    .into_iter()
                    .reduce(|a, b| {
                        if compare_option_values(&Some(a.clone()), &Some(b.clone()))
                            == std::cmp::Ordering::Less
                        {
                            a
                        } else {
                            b
                        }
                    })
                    .unwrap_or(Value::Int(0)),
                AggregateFunction::Max => values
                    .into_iter()
                    .reduce(|a, b| {
                        if compare_option_values(&Some(a.clone()), &Some(b.clone()))
                            == std::cmp::Ordering::Greater
                        {
                            a
                        } else {
                            b
                        }
                    })
                    .unwrap_or(Value::Int(0)),
                AggregateFunction::Sample => values.into_iter().next().unwrap_or(Value::Int(0)),
                AggregateFunction::GroupConcat { separator } => {
                    let sep = separator.as_deref().unwrap_or(" ");
                    let strs: Vec<String> = values
                        .iter()
                        .map(|v| match v {
                            Value::Str(s) => s.clone(),
                            Value::Int(n) => n.to_string(),
                            Value::Float(f) => f.to_string(),
                            Value::Bool(b) => b.to_string(),
                            _ => String::new(),
                        })
                        .collect();
                    Value::Str(strs.join(sep))
                }
                AggregateFunction::Custom(_) => Value::Int(0),
            }
        }
    }
}

/// Compare two optional Values for ordering (used by ORDER BY).
pub fn compare_option_values(a: &Option<Value>, b: &Option<Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(va), Some(vb)) => match (va, vb) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::Int(a), Value::Float(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Ref(a), Value::Ref(b)) => a.cmp(b),
            _ => std::cmp::Ordering::Equal,
        },
    }
}
