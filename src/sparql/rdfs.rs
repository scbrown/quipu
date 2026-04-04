//! RDFS type-hierarchy helpers — subclass inference for rdf:type queries.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

use super::Bindings;
use super::TemporalContext;
use super::pattern::bind_var;

pub const RDF_TYPE: &str = namespace::RDF_TYPE;
const RDFS_SUBCLASS_OF: &str = namespace::RDFS_SUBCLASS_OF;

/// Check if a triple pattern has rdf:type as predicate and a concrete class as object.
pub fn is_rdf_type_pattern(tp: &TriplePattern) -> bool {
    matches!(&tp.predicate, NamedNodePattern::NamedNode(n) if n.as_str() == RDF_TYPE)
        && matches!(&tp.object, TermPattern::NamedNode(_))
}

/// Collect a class and all its subclasses (transitive) from the fact log.
///
/// Uses rdfs:subClassOf triples: `SubClass rdfs:subClassOf SuperClass`.
/// Returns the term IDs of the class and all subclasses.
pub fn collect_class_and_subclasses(store: &Store, class_iri: &str) -> Result<Vec<i64>> {
    let Some(class_id) = store.lookup(class_iri)? else {
        return Ok(vec![]);
    };

    let Some(subclass_pred) = store.lookup(RDFS_SUBCLASS_OF)? else {
        return Ok(vec![class_id]); // No subClassOf pred -> just the class itself
    };

    // BFS to find all subclasses.
    let mut result = vec![class_id];
    let mut frontier = vec![class_id];

    while !frontier.is_empty() {
        let mut next_frontier = Vec::new();
        for super_id in &frontier {
            // Find all X where X rdfs:subClassOf super_id (as a Ref value)
            let target_bytes = Value::Ref(*super_id).to_bytes();
            let mut stmt = store.prepare(
                "SELECT e FROM facts WHERE a = ?1 AND v = ?2 AND op = 1 AND valid_to IS NULL",
            )?;
            let mut rows = stmt.query(rusqlite::params![subclass_pred, target_bytes])?;
            while let Some(row) = rows.next()? {
                let sub_id: i64 = row.get(0)?;
                if !result.contains(&sub_id) {
                    result.push(sub_id);
                    next_frontier.push(sub_id);
                }
            }
        }
        frontier = next_frontier;
    }

    Ok(result)
}

/// Evaluate a `?x a <Class>` pattern with subclass expansion.
pub fn eval_type_pattern_with_subclasses(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    class_ids: &[i64],
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    let Some(type_pred_id) = store.lookup(RDF_TYPE)? else {
        return Ok(vec![]);
    };

    // Subject filter (if bound).
    let subject_id =
        if let Some(iri) = super::pattern::resolve_subject_pattern(&tp.subject, bindings) {
            match store.lookup(&iri)? {
                Some(id) => Some(id),
                None => return Ok(vec![]),
            }
        } else {
            None
        };

    let mut results = Vec::new();

    // For each class in the hierarchy, find instances.
    for class_id in class_ids {
        let v_bytes = Value::Ref(*class_id).to_bytes();

        // Build SQL and params dynamically.
        let mut conditions = vec!["a = ?1".to_string(), "v = ?2".to_string()];
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(type_pred_id));
        params_vec.push(Box::new(v_bytes.clone()));

        if let Some(sid) = subject_id {
            conditions.push(format!("e = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(sid));
        }
        conditions.push("op = 1".to_string());
        if let Some(vt) = &ctx.valid_at {
            conditions.push(format!("valid_from <= ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(vt.clone()));
            conditions.push(format!(
                "(valid_to IS NULL OR valid_to > ?{})",
                params_vec.len()
            ));
        } else {
            conditions.push("valid_to IS NULL".to_string());
        }
        if let Some(tx) = ctx.as_of_tx {
            conditions.push(format!("tx <= ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(tx));
        }

        let sql = format!("SELECT e, v FROM facts WHERE {}", conditions.join(" AND "));
        let mut stmt = store.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(std::convert::AsRef::as_ref).collect();
        let mut rows = stmt.query(param_refs.as_slice())?;

        while let Some(row) = rows.next()? {
            let e_id: i64 = row.get(0)?;
            let v_bytes_row: Vec<u8> = row.get(1)?;
            let v = Value::from_bytes(&v_bytes_row)?;

            let mut new_bindings = bindings.clone();
            let mut compatible = true;

            // Bind subject variable.
            if let TermPattern::Variable(var) = &tp.subject {
                let e_iri = store.resolve(e_id)?;
                let e_val = if let Some(term_id) = store.lookup(&e_iri)? {
                    Value::Ref(term_id)
                } else {
                    Value::Str(e_iri)
                };
                if !bind_var(&mut new_bindings, var.as_str(), e_val, &mut compatible) {
                    continue;
                }
            }

            // Bind object variable (always the matched class).
            if let TermPattern::Variable(var) = &tp.object
                && !bind_var(&mut new_bindings, var.as_str(), v, &mut compatible)
            {
                continue;
            }

            if compatible {
                results.push(new_bindings);
            }
        }
    }

    Ok(results)
}
