//! CONSTRUCT / DESCRIBE result building (split from `sparql/mod.rs`, quipu-sd1).

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::{Bindings, Triple};

/// Instantiate a CONSTRUCT template with each row of bindings.
pub(super) fn eval_construct(
    store: &Store,
    template: &[spargebra::term::TriplePattern],
    rows: &[Bindings],
) -> Result<Vec<Triple>> {
    use spargebra::term::{NamedNodePattern, TermPattern};

    let mut triples = Vec::new();

    for row in rows {
        for tp in template {
            let subject = match &tp.subject {
                TermPattern::NamedNode(n) => n.as_str().to_string(),
                TermPattern::Variable(v) => match row.get(v.as_str()) {
                    Some(Value::Ref(id)) => store.resolve(*id)?,
                    Some(Value::Str(s)) => s.clone(),
                    _ => continue,
                },
                TermPattern::BlankNode(b) => format!("_:{}", b.as_str()),
                TermPattern::Literal(_) => continue,
                #[cfg(feature = "shacl")]
                TermPattern::Triple(_) => continue,
            };

            let predicate = match &tp.predicate {
                NamedNodePattern::NamedNode(n) => n.as_str().to_string(),
                NamedNodePattern::Variable(v) => match row.get(v.as_str()) {
                    Some(Value::Ref(id)) => store.resolve(*id)?,
                    Some(Value::Str(s)) => s.clone(),
                    _ => continue,
                },
            };

            let object = match &tp.object {
                TermPattern::NamedNode(n) => {
                    if let Some(id) = store.lookup(n.as_str())? {
                        Value::Ref(id)
                    } else {
                        Value::Str(n.as_str().to_string())
                    }
                }
                TermPattern::Literal(lit) => super::filter::literal_to_value(lit),
                TermPattern::Variable(v) => match row.get(v.as_str()) {
                    Some(val) => val.clone(),
                    None => continue,
                },
                TermPattern::BlankNode(b) => Value::Str(format!("_:{}", b.as_str())),
                #[cfg(feature = "shacl")]
                TermPattern::Triple(_) => continue,
            };

            let triple = Triple {
                subject,
                predicate,
                object,
            };
            if !triples.contains(&triple) {
                triples.push(triple);
            }
        }
    }

    Ok(triples)
}

/// Gather all triples for each entity mentioned in the result rows.
pub(super) fn eval_describe(store: &Store, rows: &[Bindings]) -> Result<Vec<Triple>> {
    let mut entity_ids = Vec::new();

    // Collect all Ref values from all bindings.
    for row in rows {
        for val in row.values() {
            if let Value::Ref(id) = val
                && !entity_ids.contains(id)
            {
                entity_ids.push(*id);
            }
        }
    }

    let mut triples = Vec::new();
    for eid in &entity_ids {
        let facts = store.entity_facts(*eid)?;
        let subject_iri = store.resolve(*eid)?;
        for fact in &facts {
            let predicate = store.resolve(fact.attribute)?;
            let object = match &fact.value {
                Value::Ref(id) => Value::Ref(*id),
                other => other.clone(),
            };
            let triple = Triple {
                subject: subject_iri.clone(),
                predicate,
                object,
            };
            if !triples.contains(&triple) {
                triples.push(triple);
            }
        }
    }

    Ok(triples)
}
