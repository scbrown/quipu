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
