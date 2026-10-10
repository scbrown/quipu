//! Keep strict SHACL feedback separate from a locally chosen admission policy.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::{Quad, Term};
use oxrdfio::{RdfFormat, RdfParser, RdfSerializer};

use crate::error::{Error, Result};
use crate::shacl::{ValidationFeedback, ValidationIssue};

const ON_VIOLATION: &str = "http://quipu.dev/ns#onViolation";

pub(crate) struct PolicyValidation {
    pub(crate) full: ValidationFeedback,
    pub(crate) blocking: bool,
    pub(crate) advisory: Vec<ValidationIssue>,
    rejecting_violations: usize,
}

impl PolicyValidation {
    pub(crate) fn report(&self) -> Result<serde_json::Value> {
        let mut report = serde_json::to_value(&self.full)
            .map_err(|error| Error::Serialization(format!("SHACL report: {error}")))?;
        report["blocking"] = self.blocking.into();
        report["rejecting_violations"] = self.rejecting_violations.into();
        report["advisory_count"] = self.advisory.len().into();
        report["advisory_results"] = serde_json::to_value(&self.advisory)
            .map_err(|error| Error::Serialization(format!("SHACL advisory report: {error}")))?;
        Ok(report)
    }
}

/// Parse policy by RDF subject, independent of Turtle whitespace/prefixes.
/// Both validation graphs keep every constraint and shared/list dependency;
/// only independent targets of inactive shapes are removed.
struct PolicyGraphs {
    full: String,
    reject: String,
    emit: String,
    has_emit: bool,
}

fn policy_graphs(shapes: &str) -> Result<PolicyGraphs> {
    let quads: Vec<Quad> = RdfParser::from_format(RdfFormat::Turtle)
        .for_reader(shapes.as_bytes())
        .map(|quad| quad.map_err(|e| Error::InvalidValue(format!("shape policy RDF: {e}"))))
        .collect::<Result<_>>()?;
    let mut policies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut node_shapes = BTreeSet::new();
    for quad in &quads {
        if quad.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
            && matches!(&quad.object, Term::NamedNode(n) if n.as_str() == "http://www.w3.org/ns/shacl#NodeShape")
        {
            node_shapes.insert(quad.subject.to_string());
        }
        if !matches!(
            quad.predicate.as_str(),
            ON_VIOLATION | "http://quipu.dev/ontology/onViolation"
        ) {
            continue;
        }
        let Term::Literal(value) = &quad.object else {
            return Err(Error::InvalidValue(
                "onViolation must be emit or reject".into(),
            ));
        };
        if !matches!(value.value(), "emit" | "reject") {
            return Err(Error::InvalidValue(format!(
                "unknown onViolation policy: {}",
                value.value()
            )));
        }
        policies
            .entry(quad.subject.to_string())
            .or_default()
            .insert(value.value().into());
    }
    if policies.values().any(|values| values.len() != 1) {
        return Err(Error::InvalidValue(
            "conflicting onViolation policies".into(),
        ));
    }
    let emit_subjects: BTreeSet<String> = policies
        .into_iter()
        .filter(|(_, values)| values.contains("emit"))
        .map(|(subject, _)| subject)
        .collect();
    if !emit_subjects.is_empty()
        && quads
            .iter()
            .any(|q| q.predicate.as_str() == "http://www.w3.org/ns/shacl#target")
    {
        return Err(Error::InvalidValue(
            "onViolation admission does not support custom SHACL targets".into(),
        ));
    }
    let partition = |emit: bool| -> Result<String> {
        serialize_policy(quads.iter().filter(|quad| {
            let active = emit_subjects.contains(&quad.subject.to_string()) == emit;
            if active { return true; }
            match quad.predicate.as_str() {
                "http://www.w3.org/ns/shacl#targetClass" |
                "http://www.w3.org/ns/shacl#targetNode" |
                "http://www.w3.org/ns/shacl#targetSubjectsOf" |
                "http://www.w3.org/ns/shacl#targetObjectsOf" => false,
                // Disable implicit class targets on inactive NodeShapes.
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type" =>
                    !(node_shapes.contains(&quad.subject.to_string())
                    && matches!(&quad.object, Term::NamedNode(n) if n.as_str() == "http://www.w3.org/2000/01/rdf-schema#Class")),
                _ => true,
            }
        }))
    };
    Ok(PolicyGraphs {
        full: serialize_policy(quads.iter())?,
        reject: partition(false)?,
        emit: partition(true)?,
        has_emit: !emit_subjects.is_empty(),
    })
}

fn serialize_policy<'a>(quads: impl Iterator<Item = &'a Quad>) -> Result<String> {
    let mut writer = RdfSerializer::from_format(RdfFormat::Turtle).for_writer(Vec::new());
    for quad in quads {
        writer
            .serialize_quad(quad)
            .map_err(|e| Error::InvalidValue(format!("shape policy serialize: {e}")))?;
    }
    let bytes = writer
        .finish()
        .map_err(|e| Error::InvalidValue(format!("shape policy finish: {e}")))?;
    String::from_utf8(bytes).map_err(|e| Error::InvalidValue(format!("shape policy UTF8: {e}")))
}

/// Validate full diagnostics and reject gates under caller-authoritative policy.
pub(crate) fn validate<F>(
    shapes: &str,
    data: &str,
    emit_authorized: bool,
    mut validator: F,
) -> Result<PolicyValidation>
where
    F: FnMut(&str, &str) -> Result<ValidationFeedback>,
{
    let split = policy_graphs(shapes)?;
    let mut full = validator(&split.full, data)?;
    // Context repair historically used violations==0; this report promises
    // SHACL's strict meaning, including Warning and Info results.
    full.conforms = full.results.is_empty();
    let (blocking, advisory, rejecting_violations) = if emit_authorized && split.has_emit {
        let rejecting = validator(&split.reject, data)?;
        let advisory = validator(&split.emit, data)?.results;
        (rejecting.blocks(), advisory, rejecting.violations)
    } else {
        (full.blocks(), Vec::new(), full.violations)
    };
    Ok(PolicyValidation {
        full,
        blocking,
        advisory,
        rejecting_violations,
    })
}

#[cfg(test)]
#[path = "shacl_admission_tests.rs"]
mod tests;
