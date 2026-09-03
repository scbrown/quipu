//! Aggregate expression evaluation (COUNT, SUM, AVG, MIN, MAX, SAMPLE, `GROUP_CONCAT`).

use spargebra::algebra::{AggregateExpression, AggregateFunction};

use crate::store::Store;
use crate::types::Value;

use super::Bindings;
use super::filter::eval_expr;

/// Evaluate an aggregate expression over a group of rows.
pub fn eval_aggregate(
    store: &Store,
    agg: &AggregateExpression,
    rows: &[Bindings],
) -> Option<Value> {
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
                Some(Value::Int(count as i64))
            } else {
                Some(Value::Int(rows.len() as i64))
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
                AggregateFunction::Count => Some(Value::Int(values.len() as i64)),
                AggregateFunction::Sum => {
                    if values.iter().any(|value| value.as_f64().is_none()) {
                        return None;
                    }
                    let mut sum = 0.0f64;
                    let mut all_int = true;
                    let mut any_double = false;
                    for v in &values {
                        // as_f64 also sees numeric Typed literals (xsd:long,
                        // xsd:decimal, …), which keep their datatype rather
                        // than collapsing into Int/Float at parse (aegis-fmyi).
                        if let Some(n) = v.as_f64() {
                            sum += n;
                            all_int &= is_integral(v);
                            any_double |= v.datatype() == Some(crate::namespace::XSD_DOUBLE);
                        }
                    }
                    if all_int {
                        Some(Value::Int(sum as i64))
                    } else if any_double {
                        Some(typed_number(sum, crate::namespace::XSD_DOUBLE))
                    } else {
                        Some(typed_number(sum, crate::namespace::XSD_DECIMAL))
                    }
                }
                AggregateFunction::Avg => {
                    if values.is_empty() {
                        return None;
                    }
                    if values.iter().any(|value| value.as_f64().is_none()) {
                        return None;
                    }
                    let mut sum = 0.0f64;
                    let mut count = 0usize;
                    let mut any_double = false;
                    for v in &values {
                        if let Some(n) = v.as_f64() {
                            sum += n;
                            count += 1;
                            any_double |= v.datatype() == Some(crate::namespace::XSD_DOUBLE);
                        }
                    }
                    if count == 0 {
                        None
                    } else {
                        Some(typed_number(
                            sum / count as f64,
                            if any_double {
                                crate::namespace::XSD_DOUBLE
                            } else {
                                crate::namespace::XSD_DECIMAL
                            },
                        ))
                    }
                }
                AggregateFunction::Min => comparable_values(&values).then(|| {
                    values
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
                        .unwrap_or(Value::Int(0))
                }),
                AggregateFunction::Max => comparable_values(&values).then(|| {
                    values
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
                        .unwrap_or(Value::Int(0))
                }),
                AggregateFunction::Sample => values.into_iter().next(),
                AggregateFunction::GroupConcat { separator } => {
                    let sep = separator.as_deref().unwrap_or(" ");
                    let strs: Vec<String> = values
                        .iter()
                        .map(|v| match v {
                            Value::Str(s) => s.clone(),
                            // GROUP_CONCAT concatenates lexical forms, so a
                            // lang literal contributes "hello", not "hello@en".
                            Value::Lang { lexical, .. } | Value::Typed { lexical, .. } => {
                                lexical.clone()
                            }
                            Value::Int(n) => n.to_string(),
                            Value::Float(f) => f.to_string(),
                            Value::Bool(b) => b.to_string(),
                            _ => String::new(),
                        })
                        .collect();
                    Some(Value::Str(strs.join(sep)))
                }
                AggregateFunction::Custom(_) => None,
            }
        }
    }
}

fn comparable_values(values: &[Value]) -> bool {
    values.is_empty()
        || values.iter().all(|value| value.as_f64().is_some())
        || values.iter().all(|value| value.as_lexical().is_some())
}

fn typed_number(value: f64, datatype: &str) -> Value {
    let lexical = if datatype == crate::namespace::XSD_DOUBLE {
        let rendered = format!("{value:E}");
        let (mantissa, exponent) = rendered.split_once('E').unwrap_or((&rendered, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        format!("{mantissa}E{}", exponent.parse::<i32>().unwrap_or(0))
    } else {
        let mut rendered = format!("{value:.12}");
        while rendered.ends_with('0') && !rendered.ends_with(".0") {
            rendered.pop();
        }
        rendered
    };
    Value::Typed {
        lexical,
        datatype: datatype.to_string(),
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
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Ref(a), Value::Ref(b)) => a.cmp(b),
            _ => match (va.as_f64(), vb.as_f64()) {
                // Any numeric pair, including Typed numerics that kept their
                // datatype (xsd:long, xsd:decimal, …), orders numerically.
                (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
                // Otherwise fall back to lexical order for literals that have
                // a lexical form at all; anything else is incomparable.
                _ => match (va.as_lexical(), vb.as_lexical()) {
                    (Some(a), Some(b)) => a.cmp(b),
                    _ => std::cmp::Ordering::Equal,
                },
            },
        },
    }
}

/// Does this value carry an integer-valued datatype? Used by SUM to decide
/// whether the total stays an integer.
fn is_integral(v: &Value) -> bool {
    match v {
        Value::Int(_) => true,
        Value::Typed { datatype, .. } => matches!(
            datatype.as_str(),
            crate::namespace::XSD_INTEGER
                | crate::namespace::XSD_LONG
                | crate::namespace::XSD_INT
                | crate::namespace::XSD_SHORT
                | crate::namespace::XSD_BYTE
                | crate::namespace::XSD_NON_NEGATIVE_INTEGER
                | crate::namespace::XSD_POSITIVE_INTEGER
                | crate::namespace::XSD_UNSIGNED_LONG
                | crate::namespace::XSD_UNSIGNED_INT
        ),
        _ => false,
    }
}
