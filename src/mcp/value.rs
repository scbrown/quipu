//! The [`Value`] ⇄ JSON converters for the MCP/REST surface. Split from
//! `mcp/mod.rs` for the file-size ratchet; the public path is unchanged
//! (`mcp::value_to_json` is re-exported).

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

/// Parse a JSON object-position value into a stored `Value`. Accepts a bare
/// string (treated as a plain literal) or a tagged object
/// `{iri|str|int|float|bool: ...}`. IRIs are interned to a `Value::Ref`.
pub(super) fn json_to_value(store: &Store, v: &JsonValue) -> Result<Value> {
    use crate::types::Value;
    if let Some(s) = v.as_str() {
        return Ok(Value::Str(s.to_string()));
    }
    if let Some(o) = v.as_object() {
        if let Some(iri) = o.get("iri").and_then(JsonValue::as_str) {
            return Ok(Value::Ref(store.intern(iri)?));
        }
        if let Some(s) = o.get("str").and_then(JsonValue::as_str) {
            return Ok(Value::Str(s.to_string()));
        }
        if let Some(n) = o.get("int").and_then(JsonValue::as_i64) {
            return Ok(Value::Int(n));
        }
        if let Some(f) = o.get("float").and_then(JsonValue::as_f64) {
            return Ok(Value::Float(f));
        }
        if let Some(b) = o.get("bool").and_then(JsonValue::as_bool) {
            return Ok(Value::Bool(b));
        }
        // A literal's tag/datatype rides in its own field. There is deliberately
        // no way to smuggle one into the lexical string (aegis-fmyi).
        if let Some(lexical) = o.get("value").and_then(JsonValue::as_str) {
            if let Some(lang) = o.get("lang").and_then(JsonValue::as_str) {
                return Ok(Value::Lang {
                    lexical: lexical.to_string(),
                    lang: lang.to_string(),
                });
            }
            if let Some(datatype) = o.get("datatype").and_then(JsonValue::as_str) {
                return Ok(Value::Typed {
                    lexical: lexical.to_string(),
                    datatype: datatype.to_string(),
                });
            }
        }
    }
    Err(Error::InvalidValue(
        "object must be a string literal, a tagged {iri|str|int|float|bool: ...}, \
         or {value, lang} / {value, datatype}"
            .into(),
    ))
}

pub fn value_to_json(store: &Store, val: &Value) -> JsonValue {
    value_to_json_mode(store, val, None)
}

/// Render an MCP value, compacting referenced IRIs and datatype IRIs by default.
pub fn value_to_json_compact(store: &Store, val: &Value) -> JsonValue {
    let prefixes = crate::compact::PrefixMap::from_store(store).ok();
    value_to_json_mode(store, val, prefixes.as_ref())
}

/// Render a value using a prefix table already loaded for the whole response.
pub fn value_to_json_with_prefixes(
    store: &Store,
    val: &Value,
    prefixes: &crate::compact::PrefixMap,
) -> JsonValue {
    value_to_json_mode(store, val, Some(prefixes))
}

fn value_to_json_mode(
    store: &Store,
    val: &Value,
    prefixes: Option<&crate::compact::PrefixMap>,
) -> JsonValue {
    let compact = |iri: &str| prefixes.map_or_else(|| iri.to_string(), |map| map.compact(iri));
    match val {
        Value::Ref(id) => {
            let iri = store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}"));
            JsonValue::String(compact(&iri))
        }
        Value::Str(s) => JsonValue::String(s.clone()),
        // Lang/Typed serialize as objects so the caller gets the CORRECT lexical
        // value AND can recover the tag/datatype. Emitting a bare "hello@en"
        // string here is what aegis-fmyi was filed against; a bare "hello" that
        // silently drops the tag is the same loss one step later.
        Value::Lang { lexical, lang } => serde_json::json!({"value": lexical, "lang": lang}),
        Value::Typed { lexical, datatype } => {
            serde_json::json!({"value": lexical, "datatype": compact(datatype)})
        }
        Value::Int(n) => serde_json::json!(n),
        Value::Float(f) => serde_json::json!(f),
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Bytes(b) => JsonValue::String(format!("<{} bytes>", b.len())),
    }
}
