//! Entity content negotiation — JSON-LD, Turtle, and preview cards.

use serde_json::{Value as JsonValue, json};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

use super::short_name;

/// Build a JSON-LD object for an entity.
pub fn entity_json_ld(store: &Store, iri: &str) -> Result<JsonValue> {
    let sparql = format!("SELECT ?p ?o WHERE {{ <{iri}> ?p ?o }}");
    let result = crate::sparql_query(store, &sparql)?;

    let mut props = serde_json::Map::new();
    props.insert("@context".to_string(), json!("https://schema.org"));
    props.insert("@id".to_string(), json!(iri));

    for row in result.rows() {
        if let (Some(Value::Ref(p_id)), Some(val)) = (row.get("p"), row.get("o")) {
            let p_name = store.resolve(*p_id).unwrap_or_else(|_| format!("{p_id}"));
            let short_p = short_name(&p_name);
            let json_val = match val {
                Value::Ref(id) => {
                    let n = store.resolve(*id).unwrap_or_else(|_| format!("{id}"));
                    json!({"@id": n})
                }
                Value::Str(s) => json!(s),
                Value::Int(i) => json!(i),
                Value::Float(f) => json!(f),
                Value::Bool(b) => json!(b),
                Value::Bytes(_) => json!("[binary]"),
            };
            match props.get_mut(&short_p) {
                Some(serde_json::Value::Array(arr)) => arr.push(json_val),
                Some(existing) => {
                    let prev = existing.clone();
                    *existing = json!(vec![prev, json_val]);
                }
                None => {
                    props.insert(short_p, json_val);
                }
            }
        }
    }
    Ok(serde_json::Value::Object(props))
}

/// Serialize an entity's facts as Turtle RDF.
pub fn entity_turtle(store: &Store, iri: &str) -> Result<Vec<u8>> {
    let eid = store
        .lookup(iri)?
        .ok_or_else(|| Error::InvalidValue(format!("entity not found: {iri}")))?;
    let facts = store.entity_facts(eid)?;
    let mut buf = Vec::new();
    let mut ser =
        oxrdfio::RdfSerializer::from_format(oxrdfio::RdfFormat::Turtle).for_writer(&mut buf);
    for fact in &facts {
        let subject = oxrdf::NamedOrBlankNode::NamedNode(
            oxrdf::NamedNode::new(iri).map_err(|e| Error::InvalidValue(format!("{e}")))?,
        );
        let pred_iri = store.resolve(fact.attribute)?;
        let predicate =
            oxrdf::NamedNode::new(&pred_iri).map_err(|e| Error::InvalidValue(format!("{e}")))?;
        let object = value_to_rdf_term(store, &fact.value)?;
        ser.serialize_triple(&oxrdf::Triple {
            subject,
            predicate,
            object,
        })
        .map_err(|e| Error::InvalidValue(format!("{e}")))?;
    }
    ser.finish()
        .map_err(|e| Error::InvalidValue(format!("{e}")))?;
    Ok(buf)
}

/// Convert a quipu `Value` to an oxrdf `Term`.
fn value_to_rdf_term(store: &Store, value: &Value) -> Result<oxrdf::Term> {
    Ok(match value {
        Value::Ref(id) => {
            let iri = store.resolve(*id)?;
            if let Some(bnode) = iri.strip_prefix("_:") {
                oxrdf::Term::BlankNode(
                    oxrdf::BlankNode::new(bnode)
                        .map_err(|e| Error::InvalidValue(format!("{e}")))?,
                )
            } else {
                oxrdf::Term::NamedNode(
                    oxrdf::NamedNode::new(&iri).map_err(|e| Error::InvalidValue(format!("{e}")))?,
                )
            }
        }
        Value::Str(s) => oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal(s)),
        Value::Int(n) => oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            n.to_string(),
            oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )),
        Value::Float(f) => oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            f.to_string(),
            oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#double"),
        )),
        Value::Bool(b) => oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            b.to_string(),
            oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#boolean"),
        )),
        Value::Bytes(_) => return Err(Error::InvalidValue("cannot serialize bytes to RDF".into())),
    })
}

/// Render a self-contained HTML preview card for an entity.
pub fn preview_card(store: &Store, iri: &str) -> Result<String> {
    let sparql = format!("SELECT ?p ?o WHERE {{ <{iri}> ?p ?o }}");
    let result = crate::sparql_query(store, &sparql)?;

    let label = short_name(iri);
    let mut entity_type = "Thing".to_string();
    let mut properties = Vec::new();

    for row in result.rows() {
        let pred = match row.get("p") {
            Some(Value::Ref(id)) => store.resolve(*id).unwrap_or_default(),
            _ => continue,
        };
        let obj = match row.get("o") {
            Some(val) => val.clone(),
            _ => continue,
        };

        let pred_short = short_name(&pred);
        if pred.contains("type") || pred.ends_with("#type") {
            if let Value::Ref(id) = &obj {
                entity_type = short_name(&store.resolve(*id).unwrap_or_default());
            }
            continue;
        }

        let val_str = match &obj {
            Value::Ref(id) => {
                let n = store.resolve(*id).unwrap_or_default();
                format!(
                    "<a href=\"/entity/{}\">{}</a>",
                    super::html_escape(&n),
                    super::html_escape(&short_name(&n))
                )
            }
            Value::Str(s) => super::html_escape(s),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => format!("{f:.2}"),
            Value::Bool(b) => b.to_string(),
            Value::Bytes(_) => "[binary]".to_string(),
        };
        properties.push((pred_short, val_str));
    }

    let props_html: String = properties
        .iter()
        .take(8)
        .map(|(p, v)| {
            format!(
                "<tr><td style=\"color:#888;padding:2px 8px 2px 0;font-size:12px\">\
                 {p}</td><td style=\"padding:2px 0;font-size:12px\">{v}</td></tr>"
            )
        })
        .collect();

    Ok(format!(
        "<!DOCTYPE html>\n\
         <html><head><meta charset=\"utf-8\">\n\
         <style>\n\
         body {{ font-family: system-ui, sans-serif; background: #1a1a2e; \
         color: #e0e0e0; margin: 0; padding: 12px; }}\n\
         .card {{ border: 1px solid #333; border-radius: 6px; padding: 12px; \
         max-width: 380px; }}\n\
         .label {{ font-size: 16px; font-weight: 600; margin-bottom: 4px; }}\n\
         .type-badge {{ display: inline-block; background: #2d2d44; color: #8be9fd; \
         font-size: 11px; padding: 2px 6px; border-radius: 3px; margin-bottom: 8px; }}\n\
         a {{ color: #8be9fd; text-decoration: none; }}\n\
         a:hover {{ text-decoration: underline; }}\n\
         </style></head>\n\
         <body>\n\
         <div class=\"card\">\n\
         <div class=\"label\">{label}</div>\n\
         <span class=\"type-badge\">{entity_type}</span>\n\
         <table style=\"width:100%;border-collapse:collapse\">{props_html}</table>\n\
         </div>\n\
         </body></html>",
        label = super::html_escape(&label),
        entity_type = super::html_escape(&entity_type),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_unknown_entity() {
        let store = Store::open_in_memory().unwrap();
        let result = preview_card(&store, "http://example.org/nonexistent");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("<!DOCTYPE html>"));
    }
}
