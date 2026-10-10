//! XSD constructor functions used as casts: `xsd:integer(?x)` and friends
//! (SPARQL 1.1 section 17.5, W3C sparql10 `cast` and `sort` tests,
//! aegis-soqv1r).
//!
//! A cast that is not allowed, or whose input is not a valid lexical form for
//! the target, is a type error and returns `None`, exactly like an unbound
//! value. Only `xsd:double` used to be implemented, so every other cast was
//! silently unbound, and `ORDER BY xsd:integer(?o)` sorted nothing.

use std::sync::LazyLock;

use regex::Regex;

use crate::namespace;
use crate::store::Store;
use crate::types::Value;

static INTEGER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?[0-9]+$").unwrap());
static DECIMAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)$").unwrap());
static FLOATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][+-]?[0-9]+)?|[+-]?INF|NaN)$").unwrap()
});
#[derive(Clone, Copy)]
enum NumericKind {
    Float,
    Double,
    Decimal,
}

/// The XSD cast targets this module implements.
pub(super) fn is_cast(function: &str) -> bool {
    matches!(
        function,
        namespace::XSD_STRING
            | namespace::XSD_INTEGER
            | namespace::XSD_DECIMAL
            | namespace::XSD_FLOAT
            | namespace::XSD_DOUBLE
            | namespace::XSD_BOOLEAN
            | namespace::XSD_DATE_TIME
    )
}

/// What a cast reads from its argument.
enum Source {
    /// A simple literal or `xsd:string`: cast from its lexical form.
    Text(String),
    Integer(i64),
    /// A number or boolean, cast by value.
    Number(f64, NumericKind),
    Boolean(bool),
    DateTime(String),
    Iri(String),
}

fn source(store: &Store, value: Value) -> Option<Source> {
    Some(match value {
        Value::Str(s) => Source::Text(s),
        Value::Int(n) => Source::Integer(n),
        Value::Float(f) => Source::Number(f, NumericKind::Double),
        Value::Bool(b) => Source::Boolean(b),
        Value::Ref(id) => Source::Iri(store.resolve(id).ok()?),
        Value::Typed { lexical, datatype } => match datatype.as_str() {
            namespace::XSD_STRING => Source::Text(lexical),
            namespace::XSD_DATE_TIME => Source::DateTime(
                lexical
                    .trim()
                    .parse::<oxsdatatypes::DateTime>()
                    .ok()?
                    .to_string(),
            ),
            namespace::XSD_BOOLEAN => Source::Boolean(parse_boolean(&lexical)?),
            other if namespace::is_integer_datatype(other) => {
                Source::Integer(lexical.trim().parse().ok()?)
            }
            namespace::XSD_FLOAT => Source::Number(
                f64::from(parse_floating(&lexical)? as f32),
                NumericKind::Float,
            ),
            namespace::XSD_DOUBLE => Source::Number(parse_floating(&lexical)?, NumericKind::Double),
            namespace::XSD_DECIMAL if DECIMAL.is_match(lexical.trim()) => {
                let n: f64 = lexical.trim().parse().ok()?;
                if !n.is_finite() {
                    return None;
                }
                Source::Number(n, NumericKind::Decimal)
            }
            _ => return None,
        },
        // A language-tagged string is not a cast source (SPARQL 17.5 table).
        Value::Lang { .. } | Value::Bytes(_) => return None,
    })
}

fn parse_boolean(lexical: &str) -> Option<bool> {
    match lexical.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_floating(lexical: &str) -> Option<f64> {
    let lexical = lexical.trim();
    if !FLOATING.is_match(lexical) {
        return None;
    }
    match lexical {
        "INF" | "+INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        other => other.parse().ok(),
    }
}

fn typed(lexical: String, datatype: &str) -> Value {
    Value::Typed {
        lexical,
        datatype: datatype.to_string(),
    }
}

fn floating_string(n: f64, kind: NumericKind, canonical_double: fn(f64) -> String) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n.is_sign_negative() { "-INF" } else { "INF" }.into();
    }
    if n == 0.0 {
        return if n.is_sign_negative() { "-0" } else { "0" }.into();
    }
    if (0.000001..1000000.0).contains(&n.abs()) {
        return match kind {
            NumericKind::Float => (n as f32).to_string(),
            _ => n.to_string(),
        };
    }
    if matches!(kind, NumericKind::Float) {
        let rendered = format!("{:E}", n as f32);
        let (mantissa, exponent) = rendered.split_once('E').unwrap_or((&rendered, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        format!("{mantissa}E{exponent}")
    } else {
        canonical_double(n)
    }
}

fn floating_lexical(n: f64, canonical_double: fn(f64) -> String) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n == f64::INFINITY {
        "INF".into()
    } else if n == f64::NEG_INFINITY {
        "-INF".into()
    } else {
        canonical_double(n)
    }
}

/// Apply the XSD constructor `target` to `value`. `None` is a type error.
pub(super) fn cast(
    store: &Store,
    target: &str,
    value: Value,
    canonical_double: fn(f64) -> String,
    format_decimal: fn(f64) -> String,
) -> Option<Value> {
    let source = source(store, value)?;
    match target {
        namespace::XSD_STRING => Some(Value::Str(match source {
            Source::Text(s) | Source::DateTime(s) | Source::Iri(s) => s,
            Source::Integer(n) => n.to_string(),
            Source::Number(n, NumericKind::Decimal) => n.to_string(),
            Source::Number(n, kind) => floating_string(n, kind, canonical_double),
            Source::Boolean(b) => b.to_string(),
        })),
        namespace::XSD_INTEGER => match source {
            Source::Integer(n) => Some(Value::Int(n)),
            Source::Text(s) if INTEGER.is_match(s.trim()) => s
                .trim()
                .trim_start_matches('+')
                .parse()
                .ok()
                .map(Value::Int),
            Source::Number(n, _)
                if n.is_finite()
                    && n.trunc() >= i64::MIN as f64
                    && n.trunc() < -(i64::MIN as f64) =>
            {
                Some(Value::Int(n.trunc() as i64))
            }
            Source::Boolean(b) => Some(Value::Int(i64::from(b))),
            _ => None,
        },
        namespace::XSD_DECIMAL => {
            if let Source::Integer(n) = source {
                return Some(typed(n.to_string(), namespace::XSD_DECIMAL));
            }
            let n = match source {
                Source::Text(s) if DECIMAL.is_match(s.trim()) => s.trim().parse().ok()?,
                Source::Number(n, _) if n.is_finite() => n,
                Source::Boolean(b) => f64::from(u8::from(b)),
                _ => return None,
            };
            if !n.is_finite() {
                return None;
            }
            Some(typed(format_decimal(n), namespace::XSD_DECIMAL))
        }
        namespace::XSD_FLOAT | namespace::XSD_DOUBLE => {
            let n = match source {
                Source::Text(s) => parse_floating(&s)?,
                Source::Integer(n) => n as f64,
                Source::Number(n, _) => n,
                Source::Boolean(b) => f64::from(u8::from(b)),
                _ => return None,
            };
            if target == namespace::XSD_FLOAT {
                Some(typed(
                    floating_lexical(f64::from(n as f32), canonical_double),
                    namespace::XSD_FLOAT,
                ))
            } else {
                Some(typed(
                    floating_lexical(n, canonical_double),
                    namespace::XSD_DOUBLE,
                ))
            }
        }
        namespace::XSD_BOOLEAN => match source {
            Source::Integer(n) => Some(Value::Bool(n != 0)),
            Source::Text(s) => parse_boolean(&s).map(Value::Bool),
            Source::Number(n, _) => Some(Value::Bool(n != 0.0 && !n.is_nan())),
            Source::Boolean(b) => Some(Value::Bool(b)),
            _ => None,
        },
        namespace::XSD_DATE_TIME => match source {
            Source::Text(s) | Source::DateTime(s) => Some(typed(
                s.trim().parse::<oxsdatatypes::DateTime>().ok()?.to_string(),
                namespace::XSD_DATE_TIME,
            )),
            _ => None,
        },
        _ => None,
    }
}
