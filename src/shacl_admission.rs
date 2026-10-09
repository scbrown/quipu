//! Keep strict SHACL feedback separate from a locally chosen admission policy.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::Term;
use oxrdfio::{RdfFormat, RdfParser};

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

/// Unknown or conflicting policy declarations refuse rather than guessing.
fn check_policy(shapes: &str) -> Result<()> {
    let mut policies = BTreeMap::<String, BTreeSet<String>>::new();
    for quad in RdfParser::from_format(RdfFormat::Turtle).for_reader(shapes.as_bytes()) {
        let quad =
            quad.map_err(|error| Error::InvalidValue(format!("shape policy RDF: {error}")))?;
        if quad.predicate.as_str() != ON_VIOLATION {
            continue;
        }
        let Term::Literal(value) = quad.object else {
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
    Ok(())
}

/// `shapes` must be the caller's authoritative policy, never an incoming override.
/// Validate the complete document for diagnostics, then the reject subset for
/// admission. The emit subset contributes advisory diagnostics only.
pub(crate) fn validate<F>(
    shapes: &str,
    data: &str,
    emit_authorized: bool,
    mut validator: F,
) -> Result<PolicyValidation>
where
    F: FnMut(&str, &str) -> Result<ValidationFeedback>,
{
    check_policy(shapes)?;
    let mut full = validator(shapes, data)?;
    // Context repair historically used violations==0; this report promises
    // SHACL's strict meaning, including Warning and Info results.
    full.conforms = full.results.is_empty();
    let split = crate::shacl::split_shapes_by_policy(shapes);
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
