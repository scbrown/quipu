//! Internal helpers for OWL Turtle parsing and axiom extraction.

use std::collections::{HashMap, HashSet};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

use super::{
    Axioms, OWL_DISJOINT_WITH, OWL_EQUIVALENT_CLASS, OWL_EQUIVALENT_PROPERTY,
    OWL_FUNCTIONAL_PROPERTY, OWL_INVERSE_OF, OWL_SYMMETRIC_PROPERTY, OWL_TRANSITIVE_PROPERTY,
    RDF_TYPE, RDFS_DOMAIN, RDFS_RANGE, RDFS_SUB_CLASS_OF, RDFS_SUB_PROPERTY_OF,
};

pub(super) fn parse_turtle_triples(turtle: &str) -> Result<Vec<(String, String, String)>> {
    use oxttl::TurtleParser;

    let parser = TurtleParser::new()
        .with_base_iri("http://example.org/")
        .map_err(|e| Error::InvalidValue(format!("base IRI error: {e}")))?;
    let mut triples = Vec::new();

    for result in parser.for_reader(turtle.as_bytes()) {
        let triple = result.map_err(|e| Error::InvalidValue(format!("Turtle parse error: {e}")))?;
        let s = triple.subject.to_string();
        let p = triple.predicate.to_string();
        let o = triple.object.to_string();
        let s = strip_angles(&s);
        let p = strip_angles(&p);
        let o = strip_angles(&o);
        triples.push((s, p, o));
    }
    Ok(triples)
}

fn strip_angles(s: &str) -> String {
    if s.starts_with('<') && s.ends_with('>') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

pub(super) fn extract_axioms(triples: &[(String, String, String)]) -> Axioms {
    let mut axioms = Axioms::default();

    for (s, p, o) in triples {
        match p.as_str() {
            RDFS_SUB_CLASS_OF => {
                axioms.subclass_of.push((s.clone(), o.clone()));
            }
            RDFS_SUB_PROPERTY_OF => {
                axioms.subproperty_of.push((s.clone(), o.clone()));
            }
            OWL_DISJOINT_WITH => {
                axioms.disjoint_with.insert((s.clone(), o.clone()));
                axioms.disjoint_with.insert((o.clone(), s.clone()));
            }
            OWL_INVERSE_OF => {
                axioms.inverse_of.push((s.clone(), o.clone()));
                axioms.inverse_of.push((o.clone(), s.clone()));
            }
            OWL_EQUIVALENT_CLASS => {
                axioms.equivalent_classes.push((s.clone(), o.clone()));
            }
            OWL_EQUIVALENT_PROPERTY => {
                axioms.equivalent_properties.push((s.clone(), o.clone()));
            }
            RDFS_DOMAIN => {
                axioms.domains.push((s.clone(), o.clone()));
            }
            RDFS_RANGE => {
                axioms.ranges.push((s.clone(), o.clone()));
            }
            RDF_TYPE => match o.as_str() {
                OWL_FUNCTIONAL_PROPERTY => {
                    axioms.functional_properties.insert(s.clone());
                }
                OWL_SYMMETRIC_PROPERTY => {
                    axioms.symmetric_properties.insert(s.clone());
                }
                OWL_TRANSITIVE_PROPERTY => {
                    axioms.transitive_properties.insert(s.clone());
                }
                _ => {}
            },
            _ => {}
        }
    }

    axioms
}

/// Compute transitive closure of a relation.
/// Returns a map: class -> set of all transitive superclasses.
pub(super) fn transitive_closure(pairs: &[(String, String)]) -> HashMap<String, HashSet<String>> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (sub, sup) in pairs {
        children.entry(sub.as_str()).or_default().push(sup.as_str());
    }

    let mut result: HashMap<String, HashSet<String>> = HashMap::new();
    for (sub, _) in pairs {
        if result.contains_key(sub) {
            continue;
        }
        let mut visited = HashSet::new();
        let mut stack = vec![sub.as_str()];
        while let Some(current) = stack.pop() {
            if let Some(parents) = children.get(current) {
                for &parent in parents {
                    if visited.insert(parent.to_string()) {
                        stack.push(parent);
                    }
                }
            }
        }
        visited.remove(sub.as_str());
        if !visited.is_empty() {
            result.insert(sub.clone(), visited);
        }
    }
    result
}

/// Collect all (entity, class_id) pairs from rdf:type facts.
pub(super) fn collect_type_facts(store: &Store, rdf_type_id: i64) -> Result<Vec<(i64, i64)>> {
    let facts = store.current_facts()?;
    let mut type_facts = Vec::new();
    for f in &facts {
        if f.attribute == rdf_type_id
            && let Value::Ref(class_id) = &f.value
        {
            type_facts.push((f.entity, *class_id));
        }
    }
    Ok(type_facts)
}

/// Collect all (subject, object) pairs for a given predicate.
pub(super) fn collect_predicate_facts(
    store: &Store,
    predicate_id: i64,
) -> Result<Vec<(i64, Value)>> {
    let facts = store.current_facts()?;
    let mut pred_facts = Vec::new();
    for f in &facts {
        if f.attribute == predicate_id {
            pred_facts.push((f.entity, f.value.clone()));
        }
    }
    Ok(pred_facts)
}
