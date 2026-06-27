//! FILTER expression evaluation and literal-to-value conversion.

use oxrdf::Literal;
use spargebra::algebra::Expression;

use crate::namespace;
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
        Expression::FunctionCall(func, args) => eval_bool_function(store, func, args, row),
        _ => true, // Unknown expressions pass through.
    }
}

/// Evaluate a boolean-returning FILTER builtin (CONTAINS, isIRI, …).
///
/// Previously every FunctionCall except Regex fell through to `true`, so
/// `FILTER(CONTAINS(...))`, `FILTER(isIRI(?o))`, etc. were silent no-ops
/// (GH#12). Implemented builtins now filter correctly; genuinely unsupported
/// functions still pass through (lenient — no regression for those).
fn eval_bool_function(
    store: &Store,
    func: &spargebra::algebra::Function,
    args: &[Expression],
    row: &Bindings,
) -> bool {
    use spargebra::algebra::Function;
    let str_arg = |i: usize| -> Option<String> {
        args.get(i)
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| value_to_string(store, &v))
    };
    match func {
        Function::Contains => match (str_arg(0), str_arg(1)) {
            (Some(h), Some(n)) => h.contains(&n),
            _ => false,
        },
        Function::StrStarts => match (str_arg(0), str_arg(1)) {
            (Some(h), Some(n)) => h.starts_with(&n),
            _ => false,
        },
        Function::StrEnds => match (str_arg(0), str_arg(1)) {
            (Some(h), Some(n)) => h.ends_with(&n),
            _ => false,
        },
        Function::Regex => match (str_arg(0), str_arg(1)) {
            // Best-effort: substring match (full regex not yet wired).
            (Some(text), Some(pat)) => text.contains(&pat),
            _ => false,
        },
        Function::IsIri | Function::IsBlank => {
            matches!(args.first().and_then(|e| eval_expr(store, e, row)), Some(Value::Ref(_)))
        }
        Function::IsLiteral => matches!(
            args.first().and_then(|e| eval_expr(store, e, row)),
            Some(Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Bytes(_))
        ),
        Function::IsNumeric => matches!(
            args.first().and_then(|e| eval_expr(store, e, row)),
            Some(Value::Int(_) | Value::Float(_))
        ),
        _ => true, // Genuinely unsupported function — pass through (no regression).
    }
}

/// Render a Value as a string for string builtins (STR/CONTAINS/LCASE/…).
/// Refs resolve to their IRI string.
fn value_to_string(store: &Store, v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Ref(id) => store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}")),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

/// Evaluate an expression to a Value.
pub fn eval_expr(store: &Store, expr: &Expression, row: &Bindings) -> Option<Value> {
    use spargebra::algebra::Function;
    match expr {
        Expression::Variable(var) => row.get(var.as_str()).cloned(),
        Expression::NamedNode(n) => store.lookup(n.as_str()).ok().flatten().map(Value::Ref),
        Expression::Literal(lit) => Some(literal_to_value(lit)),
        // String-valued builtins so nested calls like CONTAINS(LCASE(STR(?s)), ..)
        // resolve correctly (GH#12).
        Expression::FunctionCall(Function::Str, args) => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| Value::Str(value_to_string(store, &v))),
        Expression::FunctionCall(Function::LCase, args) => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| Value::Str(value_to_string(store, &v).to_lowercase())),
        Expression::FunctionCall(Function::UCase, args) => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| Value::Str(value_to_string(store, &v).to_uppercase())),
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
        namespace::XSD_INTEGER | namespace::XSD_LONG | namespace::XSD_INT => {
            if let Ok(n) = lit.value().parse::<i64>() {
                Value::Int(n)
            } else {
                Value::Str(lit.value().to_string())
            }
        }
        namespace::XSD_DOUBLE | namespace::XSD_FLOAT | namespace::XSD_DECIMAL => {
            if let Ok(f) = lit.value().parse::<f64>() {
                Value::Float(f)
            } else {
                Value::Str(lit.value().to_string())
            }
        }
        namespace::XSD_BOOLEAN => Value::Bool(matches!(lit.value(), "true" | "1")),
        _ => {
            if let Some(lang) = lit.language() {
                Value::Str(format!("{}@{}", lit.value(), lang))
            } else {
                Value::Str(lit.value().to_string())
            }
        }
    }
}
