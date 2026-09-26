//! Derived numeric values never define RDF term identity.
use crate::{Value, namespace};
use bigdecimal::BigDecimal;
use std::{cmp::Ordering, str::FromStr};

fn lexical(value: &Value) -> Option<String> {
    match value {
        Value::Int(n) => Some(n.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Typed { lexical, datatype } if namespace::is_numeric_datatype(datatype) => {
            Some(lexical.clone())
        }
        _ => None,
    }
}

pub(crate) fn decimal_result(value: &BigDecimal, datatype: &str) -> Value {
    let mut lexical = value.normalized().to_plain_string();
    if datatype == namespace::XSD_DECIMAL && !lexical.contains('.') {
        lexical.push_str(".0");
    }
    crate::literal_identity::literal_to_value(&oxrdf::Literal::new_typed_literal(
        lexical,
        oxrdf::NamedNode::new_unchecked(datatype),
    ))
}

fn is_floating(value: &Value) -> bool {
    matches!(
        value.datatype(),
        Some(namespace::XSD_FLOAT | namespace::XSD_DOUBLE)
    )
}

/// Parse integer/decimal grammar before `BigDecimal` (which also accepts an
/// exponent). No binary floating conversion on the exact comparison path.
pub(crate) fn decimal(value: &Value) -> Option<BigDecimal> {
    if is_floating(value) {
        return None;
    }
    let lexical = lexical(value)?;
    let unsigned = lexical.strip_prefix(['+', '-']).unwrap_or(&lexical);
    let decimal = value.datatype() == Some(namespace::XSD_DECIMAL);
    let mut dots = 0;
    let mut digits = 0;
    for ch in unsigned.bytes() {
        match ch {
            b'0'..=b'9' => digits += 1,
            b'.' if decimal => dots += 1,
            _ => return None,
        }
    }
    if digits == 0 || dots > 1 {
        return None;
    }
    BigDecimal::from_str(&lexical).ok()
}

fn float(value: &Value) -> Option<f64> {
    if !is_floating(value) {
        return decimal(value)?.to_string().parse().ok();
    }
    let text = lexical(value)?;
    // XSD special values, not Rust's broader case-insensitive spellings.
    match text.as_str() {
        "INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ if text
            .bytes()
            .all(|c| c.is_ascii_digit() || b"+-.eE".contains(&c)) =>
        {
            text.parse().ok()
        }
        _ => None,
    }
}

pub(crate) fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    if is_floating(a) || is_floating(b) {
        let (a_num, b_num) = (float(a)?, float(b)?);
        if a.datatype() != Some(namespace::XSD_DOUBLE)
            && b.datatype() != Some(namespace::XSD_DOUBLE)
        {
            return (a_num as f32).partial_cmp(&(b_num as f32));
        }
        return a_num.partial_cmp(&b_num);
    }
    Some(decimal(a)?.cmp(&decimal(b)?))
}

fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Typed { lexical, datatype } if datatype == namespace::XSD_BOOLEAN => {
            match lexical.as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(crate) fn equal(a: &Value, b: &Value) -> bool {
    if a.datatype().is_some_and(namespace::is_numeric_datatype)
        && b.datatype().is_some_and(namespace::is_numeric_datatype)
    {
        return compare(a, b) == Some(Ordering::Equal);
    }
    if let (Some(a), Some(b)) = (boolean(a), boolean(b)) {
        return a == b;
    }
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;
    fn typed(s: &str, dt: &str) -> Value {
        Value::Typed {
            lexical: s.into(),
            datatype: dt.into(),
        }
    }
    #[test]
    fn exact_comparisons_and_term_distinction() {
        let one = typed("01", namespace::XSD_INTEGER);
        assert!(equal(&one, &Value::Int(1)));
        assert_ne!(one, Value::Int(1));
        assert!(equal(
            &typed("+184467440737095516160", namespace::XSD_INTEGER),
            &typed("184467440737095516160.0", namespace::XSD_DECIMAL)
        ));
        assert!(!equal(
            &typed("9007199254740993", namespace::XSD_INTEGER),
            &Value::Int(9007199254740992)
        ));
        assert_eq!(
            compare(
                &typed("1.00000000000000000001", namespace::XSD_DECIMAL),
                &typed("1.00000000000000000002", namespace::XSD_DECIMAL)
            ),
            Some(Ordering::Less)
        );
        assert!(equal(
            &typed("1", namespace::XSD_BOOLEAN),
            &Value::Bool(true)
        ));
        assert!(!equal(
            &typed("NaN", namespace::XSD_DOUBLE),
            &typed("NaN", namespace::XSD_DOUBLE)
        ));
        assert!(equal(
            &typed("-0e0", namespace::XSD_DOUBLE),
            &typed("0e0", namespace::XSD_DOUBLE)
        ));
        assert_eq!(decimal(&typed("1e2", namespace::XSD_DECIMAL)), None);
    }
}
