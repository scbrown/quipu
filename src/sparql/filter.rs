//! FILTER expression evaluation and literal-to-value conversion.

use oxrdf::Literal;
use spargebra::algebra::Expression;

use crate::store::Store;
use crate::types::Value;

use super::Bindings;

/// Evaluate a FILTER expression against a binding row.
pub fn eval_filter(store: &Store, expr: &Expression, row: &Bindings) -> bool {
    match expr {
        Expression::Equal(left, right) => {
            match (eval_expr(store, left, row), eval_expr(store, right, row)) {
                (Some(l), Some(r)) => l == r,
                _ => false,
            }
        }
        Expression::Greater(left, right) => compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Greater
        }),
        Expression::GreaterOrEqual(left, right) => compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Greater || o == std::cmp::Ordering::Equal
        }),
        Expression::Less(left, right) => {
            compare_values(store, left, right, row, |o| o == std::cmp::Ordering::Less)
        }
        Expression::LessOrEqual(left, right) => compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Less || o == std::cmp::Ordering::Equal
        }),
        Expression::And(left, right) => {
            eval_filter(store, left, row) && eval_filter(store, right, row)
        }
        Expression::Or(left, right) => {
            eval_filter(store, left, row) || eval_filter(store, right, row)
        }
        Expression::Not(inner) => !eval_filter(store, inner, row),
        Expression::Bound(var) => row.contains_key(var.as_str()),
        Expression::FunctionCall(spargebra::algebra::Function::Regex, args) => {
            if args.len() >= 2 {
                if let (Some(Value::Str(text)), Some(Value::Str(pattern))) = (
                    eval_expr(store, &args[0], row),
                    eval_expr(store, &args[1], row),
                ) {
                    // Simple regex: just check contains for now.
                    text.contains(&pattern)
                } else {
                    false
                }
            } else {
                false
            }
        }
        _ => true, // Unknown expressions pass through.
    }
}

/// Evaluate an expression to a Value.
pub fn eval_expr(store: &Store, expr: &Expression, row: &Bindings) -> Option<Value> {
    match expr {
        Expression::Variable(var) => row.get(var.as_str()).cloned(),
        Expression::NamedNode(n) => store.lookup(n.as_str()).ok().flatten().map(Value::Ref),
        Expression::Literal(lit) => Some(literal_to_value(lit)),
        _ => None,
    }
}

/// Compare two expressions with an ordering predicate.
pub fn compare_values(
    store: &Store,
    left: &Expression,
    right: &Expression,
    row: &Bindings,
    pred: impl Fn(std::cmp::Ordering) -> bool,
) -> bool {
    match (eval_expr(store, left, row), eval_expr(store, right, row)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => pred(a.cmp(&b)),
        (Some(Value::Float(a)), Some(Value::Float(b))) => a.partial_cmp(&b).is_some_and(&pred),
        (Some(Value::Int(a)), Some(Value::Float(b))) => {
            (a as f64).partial_cmp(&b).is_some_and(&pred)
        }
        (Some(Value::Float(a)), Some(Value::Int(b))) => {
            a.partial_cmp(&(b as f64)).is_some_and(&pred)
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => pred(a.cmp(&b)),
        _ => false,
    }
}

/// Convert an oxrdf Literal to a Value (same logic as rdf module).
pub fn literal_to_value(lit: &Literal) -> Value {
    let dt = lit.datatype().as_str();
    match dt {
        "http://www.w3.org/2001/XMLSchema#integer"
        | "http://www.w3.org/2001/XMLSchema#long"
        | "http://www.w3.org/2001/XMLSchema#int" => {
            if let Ok(n) = lit.value().parse::<i64>() {
                Value::Int(n)
            } else {
                Value::Str(lit.value().to_string())
            }
        }
        "http://www.w3.org/2001/XMLSchema#double"
        | "http://www.w3.org/2001/XMLSchema#float"
        | "http://www.w3.org/2001/XMLSchema#decimal" => {
            if let Ok(f) = lit.value().parse::<f64>() {
                Value::Float(f)
            } else {
                Value::Str(lit.value().to_string())
            }
        }
        "http://www.w3.org/2001/XMLSchema#boolean" => {
            Value::Bool(matches!(lit.value(), "true" | "1"))
        }
        _ => {
            if let Some(lang) = lit.language() {
                Value::Str(format!("{}@{}", lit.value(), lang))
            } else {
                Value::Str(lit.value().to_string())
            }
        }
    }
}
