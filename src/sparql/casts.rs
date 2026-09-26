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
static DATE_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^-?[0-9]{4,}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})?$",
    )
    .unwrap()
});

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
    /// A number or boolean, cast by value.
    Number(f64),
    Boolean(bool),
    DateTime(String),
    Iri(String),
}

fn source(store: &Store, value: Value) -> Option<Source> {
    Some(match value {
        Value::Str(s) => Source::Text(s),
        Value::Int(n) => Source::Number(n as f64),
        Value::Float(f) => Source::Number(f),
        Value::Bool(b) => Source::Boolean(b),
        Value::Ref(id) => Source::Iri(store.resolve(id).ok()?),
        Value::Typed { lexical, datatype } => match datatype.as_str() {
            namespace::XSD_STRING => Source::Text(lexical),
            namespace::XSD_DATE_TIME => Source::DateTime(lexical),
            namespace::XSD_BOOLEAN => Source::Boolean(parse_boolean(&lexical)?),
            other if namespace::is_numeric_datatype(other) => {
                Source::Number(parse_floating(&lexical)?)
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
            Source::Number(n) if n.fract() == 0.0 && n.is_finite() => format!("{}", n as i64),
            Source::Number(n) => n.to_string(),
            Source::Boolean(b) => b.to_string(),
        })),
        namespace::XSD_INTEGER => match source {
            Source::Text(s) if INTEGER.is_match(s.trim()) => s
                .trim()
                .trim_start_matches('+')
                .parse()
                .ok()
                .map(Value::Int),
            Source::Number(n) if n.is_finite() => Some(Value::Int(n.trunc() as i64)),
            Source::Boolean(b) => Some(Value::Int(i64::from(b))),
            _ => None,
        },
        namespace::XSD_DECIMAL => {
            let n = match source {
                Source::Text(s) if DECIMAL.is_match(s.trim()) => s.trim().parse().ok()?,
                Source::Number(n) if n.is_finite() => n,
                Source::Boolean(b) => f64::from(u8::from(b)),
                _ => return None,
            };
            Some(typed(format_decimal(n), namespace::XSD_DECIMAL))
        }
        namespace::XSD_FLOAT | namespace::XSD_DOUBLE => {
            let n = match source {
                Source::Text(s) => parse_floating(&s)?,
                Source::Number(n) => n,
                Source::Boolean(b) => f64::from(u8::from(b)),
                _ => return None,
            };
            if target == namespace::XSD_FLOAT {
                Some(typed(
                    canonical_double(f64::from(n as f32)),
                    namespace::XSD_FLOAT,
                ))
            } else {
                Some(typed(canonical_double(n), namespace::XSD_DOUBLE))
            }
        }
        namespace::XSD_BOOLEAN => match source {
            Source::Text(s) => parse_boolean(&s).map(Value::Bool),
            Source::Number(n) => Some(Value::Bool(n != 0.0 && !n.is_nan())),
            Source::Boolean(b) => Some(Value::Bool(b)),
            _ => None,
        },
        namespace::XSD_DATE_TIME => match source {
            Source::Text(s) | Source::DateTime(s) if DATE_TIME.is_match(s.trim()) => {
                Some(typed(s.trim().to_string(), namespace::XSD_DATE_TIME))
            }
            _ => None,
        },
        _ => None,
    }
}
