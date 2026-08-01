//! FILTER expression evaluation and literal-to-value conversion.

use oxrdf::Literal;
use spargebra::algebra::Expression;

use super::TemporalContext;

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
pub fn eval_filter(
    store: &Store,
    expr: &Expression,
    row: &Bindings,
    ctx: &TemporalContext,
) -> Result<bool> {
    match expr {
        Expression::Equal(left, right) => Ok(expr_eq(store, left, right, row)),
        // quipu #52: `?x IN (a, b)` is defined by SPARQL 1.1 as the disjunction
        // `?x = a || ?x = b`, so it desugars here rather than needing its own
        // comparison logic — it shares `expr_eq` with the `=` arm above so the
        // two can never drift. `NOT IN` is parsed as `Not(In(…))`, which the
        // `Not` arm below already handles. An EMPTY candidate list is `false`
        // (and `NOT IN ()` therefore `true`), per spec.
        Expression::In(lhs, candidates) => Ok(candidates
            .iter()
            .any(|candidate| expr_eq(store, lhs, candidate, row))),
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
            Ok(eval_filter(store, left, row, ctx)? && eval_filter(store, right, row, ctx)?)
        }
        Expression::Or(left, right) => {
            Ok(eval_filter(store, left, row, ctx)? || eval_filter(store, right, row, ctx)?)
        }
        Expression::Not(inner) => Ok(!eval_filter(store, inner, row, ctx)?),
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
        // EXISTS { pattern } (and NOT EXISTS via the Not arm above). The inner
        // graph pattern is evaluated through the full pattern engine — so
        // property paths, OPTIONAL, nested FILTERs etc. all work inside it
        // (paths under NOT EXISTS were the specific gap this closes). SPARQL
        // semantics: EXISTS is true for the current row iff some solution of
        // the inner pattern is COMPATIBLE with it — agrees on every shared
        // variable. We evaluate the inner pattern fresh and test compatibility
        // rather than substituting, which is the standard definition and keeps
        // path/join evaluation untouched.
        Expression::Exists(inner) => {
            // Seed the inner pattern with the current row so it is CONSTRAINED
            // by the outer bindings (SPARQL substitution semantics) — a bound
            // ?s makes a path traverse only from that ?s. The seeded result is
            // already exactly the compatible solutions, so EXISTS is simply
            // "did it produce a row" (replaces the unseeded
            // per-row full re-eval that held the store mutex O(n x inner)).
            let (inner_rows, _) = super::pattern::eval_pattern_seeded(store, inner, ctx, row)?;
            Ok(!inner_rows.is_empty())
        }
        other => Err(Error::InvalidValue(format!(
            "unsupported FILTER expression: {other:?}"
        ))),
    }
}

/// Term equality for `=` and `IN`. An operand that cannot be evaluated (an
/// unbound variable, an IRI absent from the dictionary) is not equal to
/// anything rather than an error — matching what `=` has always done.
fn expr_eq(store: &Store, left: &Expression, right: &Expression, row: &Bindings) -> bool {
    match (eval_expr(store, left, row), eval_expr(store, right, row)) {
        (Some(l), Some(r)) => l == r,
        _ => false,
    }
}

/// SPARQL effective boolean value for a bound value used directly as a FILTER.
fn effective_boolean_value(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Str(s) => Some(!s.is_empty()),
        Value::Int(i) => Some(*i != 0),
        Value::Float(f) => Some(*f != 0.0),
        // SPARQL EBV: numeric literals test against zero, plain/lang strings
        // against emptiness. Other datatypes have no EBV.
        Value::Typed { lexical, datatype } if namespace::is_numeric_datatype(datatype) => {
            lexical.parse::<f64>().ok().map(|f| f != 0.0)
        }
        Value::Typed { lexical, datatype } if datatype == namespace::XSD_BOOLEAN => {
            Some(matches!(lexical.as_str(), "true" | "1"))
        }
        Value::Lang { lexical, .. } => Some(!lexical.is_empty()),
        Value::Typed { .. } | Value::Ref(_) | Value::Bytes(_) => None,
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
        Function::IsIri => {
            matches!(
                args.first().and_then(|e| eval_expr(store, e, row)),
                Some(Value::Ref(_))
            )
        }
        // A `Value` has no blank-node variant: blank nodes are interned as
        // `Ref`s to an IRI carrying a "_:" prefix, which this cannot see from
        // here. So this is false for every possible value rather than merely
        // unimplemented. It was previously aliased to IsIri through a shared
        // match arm, which made FILTER(isBlank(?s)) match every IRI in the store
        // (aegis-t2jh — still open; aegis-fmyi's Lang/Typed work does not
        // resolve it).
        Function::IsBlank => false,
        // Lang and Typed are literals too. Listing the non-literal variants and
        // negating keeps this correct the next time a variant is added, instead
        // of silently answering "false" for it.
        Function::IsLiteral => matches!(
            args.first().and_then(|e| eval_expr(store, e, row)),
            Some(v) if !matches!(v, Value::Ref(_))
        ),
        // Numeric Typed literals (xsd:long, xsd:decimal, …) are numeric — they
        // only became `Typed` instead of `Int`/`Float` so their datatype IRI
        // would survive. `as_f64` is the one place that knows which they are.
        Function::IsNumeric => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .is_some_and(|v| v.as_f64().is_some()),
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
        // SPARQL STR() yields the LEXICAL form — without the tag, without the
        // datatype. STR("hello"@en) is "hello", never "hello@en".
        Value::Lang { lexical, .. } | Value::Typed { lexical, .. } => lexical.clone(),
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
    let (Some(a), Some(b)) = (eval_expr(store, left, row), eval_expr(store, right, row)) else {
        return false;
    };
    // Integers compare exactly; any other numeric pair — including Typed
    // numerics that kept their datatype (xsd:long, xsd:decimal, …) rather than
    // collapsing into Int/Float at parse — compares as f64.
    if let (Value::Int(a), Value::Int(b)) = (&a, &b) {
        return pred(a.cmp(b));
    }
    if let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) {
        return a.partial_cmp(&b).is_some_and(&pred);
    }
    // String comparison uses the LEXICAL form, so "hello"@en compares as
    // "hello" (it used to compare as "hello@en") and an xsd:date still orders
    // by its ISO-8601 lexeme as it did when it was silently a plain string.
    match (a.as_lexical(), b.as_lexical()) {
        (Some(a), Some(b)) => pred(a.cmp(b)),
        _ => false,
    }
}

/// Convert an oxrdf Literal to a Value (same logic as rdf module).
pub fn literal_to_value(lit: &Literal) -> Value {
    // A language tag must be checked FIRST: its datatype is rdf:langString.
    // Never fold the tag into the lexical form — that is irreversible (the
    // plain string "hello@en" would become indistinguishable). aegis-fmyi.
    if let Some(lang) = lit.language() {
        return Value::Lang {
            lexical: lit.value().to_string(),
            lang: lang.to_string(),
        };
    }
    let dt = lit.datatype().as_str();
    let typed = || Value::Typed {
        lexical: lit.value().to_string(),
        datatype: dt.to_string(),
    };
    match dt {
        namespace::XSD_INTEGER => lit
            .value()
            .parse::<i64>()
            .map_or_else(|_| typed(), Value::Int),
        namespace::XSD_DOUBLE => lit
            .value()
            .parse::<f64>()
            .map_or_else(|_| typed(), Value::Float),
        namespace::XSD_BOOLEAN => Value::Bool(matches!(lit.value(), "true" | "1")),
        // RDF 1.1: a plain literal's datatype IS xsd:string, so Str is lossless.
        namespace::XSD_STRING => Value::Str(lit.value().to_string()),
        // Every other datatype — xsd:date, xsd:decimal, integer subtypes,
        // customs — keeps its IRI verbatim instead of being destroyed.
        _ => typed(),
    }
}
