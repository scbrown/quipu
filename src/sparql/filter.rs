//! FILTER expression evaluation and literal-to-value conversion.

use oxrdf::{Literal, NamedNode};
use spargebra::algebra::{Expression, Function};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use super::TemporalContext;

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

use super::Bindings;

thread_local! {
    static QUERY_BASE_IRI: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(super) fn with_base_iri<T>(base_iri: Option<&str>, evaluate: impl FnOnce() -> T) -> T {
    QUERY_BASE_IRI.with(|slot| {
        let previous = slot.replace(base_iri.map(str::to_string));
        let result = evaluate();
        slot.replace(previous);
        result
    })
}

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
        Function::LangMatches => match (str_arg(0), str_arg(1)) {
            (Some(tag), Some(range)) => lang_matches(&tag, &range),
            _ => false,
        },
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
    match expr {
        Expression::Variable(var) => row.get(var.as_str()).cloned(),
        Expression::NamedNode(n) => store.lookup(n.as_str()).ok().flatten().map(Value::Ref),
        Expression::Literal(lit) => Some(literal_to_value(lit)),
        Expression::Equal(_, _)
        | Expression::SameTerm(_, _)
        | Expression::Greater(_, _)
        | Expression::GreaterOrEqual(_, _)
        | Expression::Less(_, _)
        | Expression::LessOrEqual(_, _)
        | Expression::And(_, _)
        | Expression::Or(_, _)
        | Expression::Not(_)
        | Expression::Bound(_)
        | Expression::FunctionCall(
            Function::LangMatches
            | Function::Contains
            | Function::StrStarts
            | Function::StrEnds
            | Function::IsIri
            | Function::IsBlank
            | Function::IsLiteral
            | Function::IsNumeric
            | Function::Regex,
            _,
        ) => eval_expression_boolean(store, expr, row).map(Value::Bool),
        Expression::Add(left, right) => {
            numeric_binary(store, left, right, row, i64::checked_add, |a, b| a + b)
        }
        Expression::Subtract(left, right) => {
            numeric_binary(store, left, right, row, i64::checked_sub, |a, b| a - b)
        }
        Expression::Multiply(left, right) => {
            numeric_binary(store, left, right, row, i64::checked_mul, |a, b| a * b)
        }
        Expression::Divide(left, right) => {
            let dividend = eval_expr(store, left, row)?;
            let divisor_value = eval_expr(store, right, row)?;
            let divisor = divisor_value.as_f64()?;
            if divisor == 0.0 {
                return None;
            }
            let quotient = dividend.as_f64()? / divisor;
            if matches!(dividend, Value::Int(_)) && matches!(divisor_value, Value::Int(_)) {
                Some(Value::Typed {
                    lexical: format_decimal(quotient),
                    datatype: namespace::XSD_DECIMAL.to_string(),
                })
            } else if dividend.datatype() == Some(namespace::XSD_DOUBLE)
                || divisor_value.datatype() == Some(namespace::XSD_DOUBLE)
            {
                Some(Value::Typed {
                    lexical: canonical_double(quotient),
                    datatype: namespace::XSD_DOUBLE.to_string(),
                })
            } else if dividend.datatype() == Some(namespace::XSD_DECIMAL)
                || divisor_value.datatype() == Some(namespace::XSD_DECIMAL)
            {
                Some(Value::Typed {
                    lexical: format_decimal(quotient),
                    datatype: namespace::XSD_DECIMAL.to_string(),
                })
            } else {
                Some(Value::Float(quotient))
            }
        }
        Expression::UnaryPlus(inner) => eval_expr(store, inner, row),
        Expression::UnaryMinus(inner) => match eval_expr(store, inner, row)? {
            Value::Int(value) => value.checked_neg().map(Value::Int),
            value => Some(Value::Float(-value.as_f64()?)),
        },
        Expression::If(condition, when_true, when_false) => {
            if eval_expression_boolean(store, condition, row)? {
                eval_expr(store, when_true, row)
            } else {
                eval_expr(store, when_false, row)
            }
        }
        Expression::Coalesce(expressions) => expressions
            .iter()
            .find_map(|expression| eval_expr(store, expression, row)),
        // String-valued builtins so nested calls like CONTAINS(LCASE(STR(?s)), ..)
        // resolve correctly (GH#12).
        Expression::FunctionCall(Function::Str, args) => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .map(|v| Value::Str(value_to_string(store, &v))),
        Expression::FunctionCall(Function::Lang, args) => {
            match eval_expr(store, args.first()?, row)? {
                Value::Lang { lang, .. } => Some(Value::Str(lang)),
                Value::Str(_) | Value::Typed { .. } => Some(Value::Str(String::new())),
                _ => None,
            }
        }
        Expression::FunctionCall(Function::Datatype, args) => {
            let datatype = match eval_expr(store, args.first()?, row)? {
                Value::Lang { .. } => namespace::RDF_LANG_STRING,
                Value::Str(_) => namespace::XSD_STRING,
                Value::Int(_) => namespace::XSD_INTEGER,
                Value::Float(_) => namespace::XSD_DOUBLE,
                Value::Bool(_) => namespace::XSD_BOOLEAN,
                Value::Typed { datatype, .. } => {
                    return store.intern(&datatype).ok().map(Value::Ref);
                }
                Value::Ref(_) | Value::Bytes(_) => return None,
            };
            store.intern(datatype).ok().map(Value::Ref)
        }
        Expression::FunctionCall(Function::Now, _) => Some(Value::Typed {
            lexical: crate::time::now_iso(),
            datatype: namespace::XSD_DATE_TIME.to_string(),
        }),
        Expression::FunctionCall(Function::Rand, _) => {
            let sample = next_nonce() & ((1_u64 << 53) - 1);
            Some(Value::Float(sample as f64 / (1_u64 << 53) as f64))
        }
        Expression::FunctionCall(Function::Uuid, _) => {
            let iri = format!("urn:uuid:{}", generated_uuid());
            store.intern(&iri).ok().map(Value::Ref)
        }
        Expression::FunctionCall(Function::StrUuid, _) => Some(Value::Str(generated_uuid())),
        Expression::FunctionCall(Function::Iri, args) => {
            let lexical = value_to_string(store, &eval_expr(store, args.first()?, row)?);
            let iri = resolve_query_iri(&lexical)?;
            store.intern(&iri).ok().map(Value::Ref)
        }
        Expression::FunctionCall(Function::Custom(function), args)
            if function.as_str() == namespace::XSD_DOUBLE =>
        {
            let lexical = value_to_string(store, &eval_expr(store, args.first()?, row)?);
            let value = lexical.parse::<f64>().ok()?;
            Some(Value::Typed {
                lexical: canonical_double(value),
                datatype: namespace::XSD_DOUBLE.to_string(),
            })
        }
        Expression::FunctionCall(Function::StrLang, args) => {
            let lexical = simple_string_literal(eval_expr(store, args.first()?, row)?)?;
            let (lang, _) = string_literal(eval_expr(store, args.get(1)?, row)?)?;
            Some(Value::Lang { lexical, lang })
        }
        Expression::FunctionCall(Function::StrDt, args) => {
            let lexical = simple_string_literal(eval_expr(store, args.first()?, row)?)?;
            let datatype = match args.get(1)? {
                Expression::NamedNode(datatype) => datatype.as_str().to_string(),
                other => {
                    let Value::Ref(datatype) = eval_expr(store, other, row)? else {
                        return None;
                    };
                    store.resolve(datatype).ok()?
                }
            };
            if datatype == namespace::XSD_STRING {
                Some(Value::Str(lexical))
            } else {
                Some(Value::Typed { lexical, datatype })
            }
        }
        Expression::FunctionCall(
            function @ (Function::Year
            | Function::Month
            | Function::Day
            | Function::Hours
            | Function::Minutes
            | Function::Seconds
            | Function::Timezone
            | Function::Tz),
            args,
        ) => {
            let Value::Typed { lexical, datatype } = eval_expr(store, args.first()?, row)? else {
                return None;
            };
            if datatype != namespace::XSD_DATE_TIME {
                return None;
            }
            date_time_component(function, &lexical)
        }
        Expression::FunctionCall(Function::LCase, args) => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .and_then(|v| map_string_literal(v, |s| s.to_lowercase())),
        Expression::FunctionCall(Function::UCase, args) => args
            .first()
            .and_then(|e| eval_expr(store, e, row))
            .and_then(|v| map_string_literal(v, |s| s.to_uppercase())),
        Expression::FunctionCall(Function::Concat, args) => concat(store, args, row),
        Expression::FunctionCall(Function::SubStr, args) => substring(store, args, row),
        Expression::FunctionCall(Function::StrLen, args) => {
            let value = eval_expr(store, args.first()?, row)?;
            let (lexical, _) = string_literal(value)?;
            i64::try_from(lexical.chars().count()).ok().map(Value::Int)
        }
        Expression::FunctionCall(Function::EncodeForUri, args) => {
            let value = eval_expr(store, args.first()?, row)?;
            let (lexical, _) = string_literal(value)?;
            Some(Value::Str(encode_for_uri(&lexical)))
        }
        Expression::FunctionCall(Function::StrBefore, args) => {
            string_partition(store, args, row, false)
        }
        Expression::FunctionCall(Function::StrAfter, args) => {
            string_partition(store, args, row, true)
        }
        Expression::FunctionCall(Function::Replace, args) => replace(store, args, row),
        Expression::FunctionCall(
            function @ (Function::Md5 | Function::Sha1 | Function::Sha256 | Function::Sha512),
            args,
        ) => hash_string(store, function, args, row),
        Expression::FunctionCall(Function::Abs, args) => {
            numeric_unary(store, args, row, i64::checked_abs, f64::abs)
        }
        Expression::FunctionCall(Function::Ceil, args) => {
            numeric_unary(store, args, row, Some, f64::ceil)
        }
        Expression::FunctionCall(Function::Floor, args) => {
            numeric_unary(store, args, row, Some, f64::floor)
        }
        // SPARQL ROUND follows XPath: a half-way value rounds toward positive
        // infinity. Rust's f64::round instead rounds halves away from zero.
        Expression::FunctionCall(Function::Round, args) => {
            numeric_unary(store, args, row, Some, |value| (value + 0.5).floor())
        }
        _ => None,
    }
}

fn eval_expression_boolean(store: &Store, expr: &Expression, row: &Bindings) -> Option<bool> {
    match expr {
        Expression::Equal(left, right) | Expression::SameTerm(left, right) => {
            Some(expr_eq(store, left, right, row))
        }
        Expression::Greater(left, right) => {
            Some(compare_values(store, left, right, row, |order| {
                order == std::cmp::Ordering::Greater
            }))
        }
        Expression::GreaterOrEqual(left, right) => {
            Some(compare_values(store, left, right, row, |order| {
                order != std::cmp::Ordering::Less
            }))
        }
        Expression::Less(left, right) => Some(compare_values(store, left, right, row, |order| {
            order == std::cmp::Ordering::Less
        })),
        Expression::LessOrEqual(left, right) => {
            Some(compare_values(store, left, right, row, |order| {
                order != std::cmp::Ordering::Greater
            }))
        }
        Expression::And(left, right) => Some(
            eval_expression_boolean(store, left, row)?
                && eval_expression_boolean(store, right, row)?,
        ),
        Expression::Or(left, right) => Some(
            eval_expression_boolean(store, left, row)?
                || eval_expression_boolean(store, right, row)?,
        ),
        Expression::Not(inner) => Some(!eval_expression_boolean(store, inner, row)?),
        Expression::Bound(variable) => Some(row.contains_key(variable.as_str())),
        Expression::FunctionCall(function, args) => {
            eval_bool_function(store, function, args, row).ok()
        }
        _ => effective_boolean_value(&eval_expr(store, expr, row)?),
    }
}

static NONCE: AtomicU64 = AtomicU64::new(0);

fn next_nonce() -> u64 {
    let counter = NONCE.fetch_add(1, AtomicOrdering::Relaxed);
    crate::time::epoch_secs()
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(counter.wrapping_mul(0xbf58_476d_1ce4_e5b9))
}

fn generated_uuid() -> String {
    let first = next_nonce();
    let second = next_nonce().rotate_left(29);
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        first >> 32,
        (first >> 16) & 0xffff,
        first & 0x0fff,
        0x8000 | ((second >> 48) & 0x3fff),
        second & 0xffff_ffff_ffff
    )
}

fn canonical_double(value: f64) -> String {
    let rendered = format!("{value:E}");
    let (mantissa, exponent) = rendered.split_once('E').unwrap_or((&rendered, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.to_string()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{}", exponent.parse::<i32>().unwrap_or(0))
}

fn resolve_query_iri(value: &str) -> Option<String> {
    if NamedNode::new(value).is_ok() {
        return Some(value.to_string());
    }
    QUERY_BASE_IRI.with(|slot| {
        let base = slot.borrow();
        let base = base.as_deref()?;
        if value.starts_with('/') {
            let scheme = base.find("://")? + 3;
            let authority_end = base[scheme..]
                .find('/')
                .map_or(base.len(), |offset| scheme + offset);
            Some(format!("{}{}", &base[..authority_end], value))
        } else {
            let directory_end = base.rfind('/').map_or(base.len(), |index| index + 1);
            Some(format!("{}{}", &base[..directory_end], value))
        }
    })
}

fn string_literal(value: Value) -> Option<(String, Option<String>)> {
    match value {
        Value::Str(lexical) => Some((lexical, None)),
        Value::Lang { lexical, lang } => Some((lexical, Some(lang))),
        Value::Typed { lexical, datatype } if datatype == namespace::XSD_STRING => {
            Some((lexical, None))
        }
        _ => None,
    }
}

fn simple_string_literal(value: Value) -> Option<String> {
    match value {
        Value::Str(lexical) => Some(lexical),
        Value::Typed { lexical, datatype } if datatype == namespace::XSD_STRING => Some(lexical),
        _ => None,
    }
}

fn date_time_component(function: &Function, lexical: &str) -> Option<Value> {
    let (date, time) = lexical.split_once('T')?;
    let mut date_parts = date.rsplitn(3, '-');
    let day = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<i64>().ok()?;
    let year = date_parts.next()?.parse::<i64>().ok()?;

    let (clock, timezone) = if let Some(clock) = time.strip_suffix('Z') {
        (clock, Some("Z"))
    } else if let Some(offset_at) = time
        .char_indices()
        .skip(1)
        .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index))
    {
        (&time[..offset_at], Some(&time[offset_at..]))
    } else {
        (time, None)
    };
    let mut clock_parts = clock.split(':');
    let hours = clock_parts.next()?.parse::<i64>().ok()?;
    let minutes = clock_parts.next()?.parse::<i64>().ok()?;
    let seconds = clock_parts.next()?;

    match function {
        Function::Year => Some(Value::Int(year)),
        Function::Month => Some(Value::Int(month)),
        Function::Day => Some(Value::Int(day)),
        Function::Hours => Some(Value::Int(hours)),
        Function::Minutes => Some(Value::Int(minutes)),
        Function::Seconds => Some(Value::Typed {
            lexical: canonical_seconds(seconds)?,
            datatype: namespace::XSD_DECIMAL.to_string(),
        }),
        Function::Tz => Some(Value::Str(timezone.unwrap_or_default().to_string())),
        Function::Timezone => timezone.map(|timezone| Value::Typed {
            lexical: timezone_duration(timezone),
            datatype: namespace::XSD_DAY_TIME_DURATION.to_string(),
        }),
        _ => None,
    }
}

fn canonical_seconds(seconds: &str) -> Option<String> {
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let whole = whole.parse::<u64>().ok()?;
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        Some(whole.to_string())
    } else {
        Some(format!("{whole}.{fraction}"))
    }
}

fn format_decimal(value: f64) -> String {
    let rendered = value.to_string();
    if rendered.contains('.') {
        rendered
    } else {
        format!("{rendered}.0")
    }
}

fn timezone_duration(timezone: &str) -> String {
    if timezone == "Z" || timezone == "+00:00" || timezone == "-00:00" {
        return "PT0S".to_string();
    }
    let sign = if timezone.starts_with('-') { "-" } else { "" };
    let mut parts = timezone[1..].split(':');
    let hours = parts.next().unwrap_or("0").trim_start_matches('0');
    let minutes = parts.next().unwrap_or("0").trim_start_matches('0');
    let mut duration = format!("{sign}PT");
    if !hours.is_empty() {
        duration.push_str(hours);
        duration.push('H');
    }
    if !minutes.is_empty() {
        duration.push_str(minutes);
        duration.push('M');
    }
    duration
}

fn lang_matches(tag: &str, range: &str) -> bool {
    if range == "*" {
        return !tag.is_empty();
    }
    let tag = tag.to_ascii_lowercase();
    let range = range.to_ascii_lowercase();
    tag == range
        || tag
            .strip_prefix(&range)
            .is_some_and(|rest| rest.starts_with('-'))
}

fn build_string_literal(lexical: String, lang: Option<String>) -> Value {
    lang.map_or(Value::Str(lexical.clone()), |lang| Value::Lang {
        lexical,
        lang,
    })
}

fn map_string_literal(value: Value, f: impl FnOnce(String) -> String) -> Option<Value> {
    let (lexical, lang) = string_literal(value)?;
    Some(build_string_literal(f(lexical), lang))
}

fn concat(store: &Store, args: &[Expression], row: &Bindings) -> Option<Value> {
    let mut result = String::new();
    let mut common_lang: Option<Option<String>> = None;
    for arg in args {
        let (lexical, lang) = string_literal(eval_expr(store, arg, row)?)?;
        result.push_str(&lexical);
        common_lang = Some(match common_lang {
            None => lang,
            Some(current) if current == lang => current,
            Some(_) => None,
        });
    }
    Some(build_string_literal(result, common_lang.flatten()))
}

fn substring(store: &Store, args: &[Expression], row: &Bindings) -> Option<Value> {
    let (source, lang) = string_literal(eval_expr(store, args.first()?, row)?)?;
    let start = eval_expr(store, args.get(1)?, row)?.as_f64()?.round() as i64;
    let length = if let Some(arg) = args.get(2) {
        Some(eval_expr(store, arg, row)?.as_f64()?.round() as i64)
    } else {
        None
    };
    let skip = usize::try_from(start.saturating_sub(1).max(0)).ok()?;
    let take = length.map_or(usize::MAX, |value| {
        usize::try_from(value.max(0)).unwrap_or(0)
    });
    let lexical = source.chars().skip(skip).take(take).collect();
    Some(build_string_literal(lexical, lang))
}

fn string_partition(
    store: &Store,
    args: &[Expression],
    row: &Bindings,
    after: bool,
) -> Option<Value> {
    let (source, lang) = string_literal(eval_expr(store, args.first()?, row)?)?;
    let (needle, needle_lang) = string_literal(eval_expr(store, args.get(1)?, row)?)?;
    if needle_lang.is_some() && needle_lang != lang {
        return None;
    }
    let Some(position) = source.find(&needle) else {
        return Some(Value::Str(String::new()));
    };
    let lexical = if after {
        source[position + needle.len()..].to_string()
    } else {
        source[..position].to_string()
    };
    Some(build_string_literal(lexical, lang))
}

fn replace(store: &Store, args: &[Expression], row: &Bindings) -> Option<Value> {
    let value = eval_expr(store, args.first()?, row)?;
    let (source, lang) = string_literal(value)?;
    let (pattern, _) = string_literal(eval_expr(store, args.get(1)?, row)?)?;
    let (replacement, _) = string_literal(eval_expr(store, args.get(2)?, row)?)?;
    let flags = if let Some(arg) = args.get(3) {
        string_literal(eval_expr(store, arg, row)?)?.0
    } else {
        String::new()
    };
    let regex = build_regex(&pattern, &flags).ok()?;
    Some(build_string_literal(
        regex
            .replace_all(&source, replacement.as_str())
            .into_owned(),
        lang,
    ))
}

fn encode_for_uri(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(result, "%{byte:02X}");
            }
        }
    }
    result
}

fn hash_string(
    store: &Store,
    function: &spargebra::algebra::Function,
    args: &[Expression],
    row: &Bindings,
) -> Option<Value> {
    use md5::Digest as _;

    let (value, _) = string_literal(eval_expr(store, args.first()?, row)?)?;
    let bytes = value.as_bytes();
    let digest = match function {
        Function::Md5 => hex::encode(md5::Md5::digest(bytes)),
        Function::Sha1 => hex::encode(sha1::Sha1::digest(bytes)),
        Function::Sha256 => hex::encode(sha2::Sha256::digest(bytes)),
        Function::Sha512 => hex::encode(sha2::Sha512::digest(bytes)),
        _ => return None,
    };
    Some(Value::Str(digest))
}

fn numeric_unary(
    store: &Store,
    args: &[Expression],
    row: &Bindings,
    integer: impl FnOnce(i64) -> Option<i64>,
    float: impl FnOnce(f64) -> f64,
) -> Option<Value> {
    let value = eval_expr(store, args.first()?, row)?;
    match value {
        Value::Int(value) => integer(value).map(Value::Int),
        Value::Float(value) => Some(Value::Float(float(value))),
        Value::Typed { lexical, datatype } if namespace::is_numeric_datatype(&datatype) => {
            let result = float(lexical.parse::<f64>().ok()?);
            Some(Value::Typed {
                lexical: result.to_string(),
                datatype,
            })
        }
        _ => None,
    }
}

fn numeric_binary(
    store: &Store,
    left: &Expression,
    right: &Expression,
    row: &Bindings,
    integer: impl FnOnce(i64, i64) -> Option<i64>,
    float: impl FnOnce(f64, f64) -> f64,
) -> Option<Value> {
    let left = eval_expr(store, left, row)?;
    let right = eval_expr(store, right, row)?;
    match (&left, &right) {
        (Value::Int(left), Value::Int(right)) => integer(*left, *right).map(Value::Int),
        _ => {
            let value = float(left.as_f64()?, right.as_f64()?);
            if left.datatype() == Some(namespace::XSD_DOUBLE)
                || right.datatype() == Some(namespace::XSD_DOUBLE)
            {
                Some(Value::Typed {
                    lexical: canonical_double(value),
                    datatype: namespace::XSD_DOUBLE.to_string(),
                })
            } else if left.datatype() == Some(namespace::XSD_DECIMAL)
                || right.datatype() == Some(namespace::XSD_DECIMAL)
            {
                Some(Value::Typed {
                    lexical: format_decimal(value),
                    datatype: namespace::XSD_DECIMAL.to_string(),
                })
            } else {
                Some(Value::Float(value))
            }
        }
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
        // Preserve the lexical form and datatype. Numeric comparison and
        // arithmetic use `Value::as_f64`, while MIN/MAX and result formats
        // must still return the original RDF term (e.g. `1.0E2`^^xsd:double).
        namespace::XSD_DOUBLE => typed(),
        namespace::XSD_BOOLEAN => Value::Bool(matches!(lit.value(), "true" | "1")),
        // RDF 1.1: a plain literal's datatype IS xsd:string, so Str is lossless.
        namespace::XSD_STRING => Value::Str(lit.value().to_string()),
        // Every other datatype — xsd:date, xsd:decimal, integer subtypes,
        // customs — keeps its IRI verbatim instead of being destroyed.
        _ => typed(),
    }
}
