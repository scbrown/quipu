//! FILTER expression evaluation and literal-to-value conversion.

use oxrdf::Literal;
use spargebra::algebra::Expression;

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

use super::Bindings;

/// Evaluate a FILTER expression against a binding row.
///
/// Returns an error rather than a value for genuinely unsupported constructs.
/// Silently passing unknown expressions/builtins (the old `_ => true`) produced
/// wrong results with no signal — a SPARQL `FILTER` is meant to constrain, so a
/// construct we cannot evaluate must fail loudly, never match everything (hq-9hs).
pub fn eval_filter(store: &Store, expr: &Expression, row: &Bindings) -> Result<bool> {
    match expr {
        Expression::Equal(left, right) => Ok(
            match (eval_expr(store, left, row), eval_expr(store, right, row)) {
                (Some(l), Some(r)) => l == r,
                _ => false,
            },
        ),
        Expression::Greater(left, right) => Ok(compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Greater
        })),
        Expression::GreaterOrEqual(left, right) => {
            Ok(compare_values(store, left, right, row, |o| {
                o == std::cmp::Ordering::Greater || o == std::cmp::Ordering::Equal
            }))
        }
        Expression::Less(left, right) => Ok(compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Less
        })),
        Expression::LessOrEqual(left, right) => Ok(compare_values(store, left, right, row, |o| {
            o == std::cmp::Ordering::Less || o == std::cmp::Ordering::Equal
        })),
        Expression::And(left, right) => {
            Ok(eval_filter(store, left, row)? && eval_filter(store, right, row)?)
        }
        Expression::Or(left, right) => {
            Ok(eval_filter(store, left, row)? || eval_filter(store, right, row)?)
        }
        Expression::Not(inner) => Ok(!eval_filter(store, inner, row)?),
        Expression::Bound(var) => Ok(row.contains_key(var.as_str())),
        Expression::FunctionCall(func, args) => eval_bool_function(store, func, args, row),
        // A bare variable/literal used directly as a FILTER takes its effective
        // boolean value, e.g. `FILTER(?flag)` or `FILTER("x")`.
        Expression::Variable(_) | Expression::Literal(_) => {
            match eval_expr(store, expr, row)
                .as_ref()
                .and_then(effective_boolean_value)
            {
                Some(b) => Ok(b),
                None => Err(Error::InvalidValue(format!(
                    "FILTER expression has no effective boolean value: {expr:?}"
                ))),
            }
        }
        other => Err(Error::InvalidValue(format!(
            "unsupported FILTER expression: {other:?}"
        ))),
    }
}

/// SPARQL effective boolean value for a bound value used directly as a FILTER.
fn effective_boolean_value(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Str(s) => Some(!s.is_empty()),
        Value::Int(i) => Some(*i != 0),
        Value::Float(f) => Some(*f != 0.0),
        Value::Ref(_) | Value::Bytes(_) => None,
    }
}

/// Evaluate a boolean-returning FILTER builtin (CONTAINS, REGEX, isIRI, …).
///
/// Implemented builtins filter correctly; a genuinely unsupported function now
/// returns an error instead of passing through to `true`. Silently matching
/// every row for an unrecognised predicate corrupts results invisibly (hq-9hs).
fn eval_bool_function(
    store: &Store,
    func: &spargebra::algebra::Function,
    args: &[Expression],
    row: &Bindings,
) -> Result<bool> {
    use spargebra::algebra::Function;
    let str_arg = |i: usize| -> Option<String> {
        args.get(i)
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| value_to_string(store, &v))
    };
    Ok(match func {
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
        Function::Regex => return eval_regex(store, args, row),
        Function::IsIri | Function::IsBlank => {
            matches!(
                args.first().and_then(|e| eval_expr(store, e, row)),
                Some(Value::Ref(_))
            )
        }
        Function::IsLiteral => matches!(
            args.first().and_then(|e| eval_expr(store, e, row)),
            Some(
                Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Bytes(_)
            )
        ),
        Function::IsNumeric => matches!(
            args.first().and_then(|e| eval_expr(store, e, row)),
            Some(Value::Int(_) | Value::Float(_))
        ),
        other => {
            return Err(Error::InvalidValue(format!(
                "unsupported FILTER function: {other:?}"
            )));
        }
    })
}

/// Evaluate `REGEX(text, pattern [, flags])` with a real regex engine.
///
/// Replaces the old substring-only stub. An invalid pattern or unsupported flag
/// is an error (fail loud), never a silent partial match. SPARQL flags i/s/m/x
/// map to the corresponding inline regex flags.
fn eval_regex(store: &Store, args: &[Expression], row: &Bindings) -> Result<bool> {
    let arg_str = |i: usize| -> Option<String> {
        args.get(i)
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| value_to_string(store, &v))
    };
    // Unbound text or pattern → no match (cannot evaluate, but not an error).
    let (Some(text), Some(pattern)) = (arg_str(0), arg_str(1)) else {
        return Ok(false);
    };
    let flags = arg_str(2).unwrap_or_default();
    let re = build_regex(&pattern, &flags)?;
    Ok(re.is_match(&text))
}

/// Compile a SPARQL REGEX pattern + flag string into a `regex::Regex`.
fn build_regex(pattern: &str, flags: &str) -> Result<regex::Regex> {
    let mut inline = String::new();
    for f in flags.chars() {
        match f {
            'i' | 's' | 'm' | 'x' => inline.push(f),
            other => {
                return Err(Error::InvalidValue(format!(
                    "unsupported REGEX flag: {other:?}"
                )));
            }
        }
    }
    let full = if inline.is_empty() {
        pattern.to_string()
    } else {
        format!("(?{inline}){pattern}")
    };
    regex::Regex::new(&full)
        .map_err(|e| Error::InvalidValue(format!("invalid REGEX pattern {pattern:?}: {e}")))
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
