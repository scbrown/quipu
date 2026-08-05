//! Store context for write-time SHACL (aegis-fp17f).
//!
//! ## The defect this exists to remove
//!
//! SHACL here validated the SUBMITTED payload in isolation. That is defensible
//! for constraints about what a write ASSERTS, and wrong for constraints about
//! what a write REFERENCES: `sh:class` on an object property asks "is this value
//! a Foo?", and the answer lives in the graph, not in the request body. So a
//! correct write was refused whenever the referenced node's `rdf:type` triple
//! happened not to travel with it.
//!
//! The consequence is worse than strictness. It made the verdict a function of
//! how a caller PARTITIONED its triples: the same facts, submitted whole,
//! conformed; submitted split across two writes, the second was refused.
//! Measured at scale on hank's chunked promotion (aegis-sd5fj) — 2315 of 7638
//! symbols refused at chunk 2 of 71, every one of them a correct fact about a
//! module that was already typed in the store. A validator whose answer depends
//! on submission order is not reporting conformance, it is reporting an
//! accident of framing.
//!
//! ## What this does instead: REPAIR, never re-judge
//!
//! Validation runs against the payload ALONE first. That verdict — today's
//! verdict, unchanged — is the ceiling. Only if it refuses does a second pass
//! run with the store's type triples added, and the second pass may only
//! REMOVE violations from the first. The reported result is always a SUBSET of
//! what the payload alone produced.
//!
//! So "strictly more permissive" is not an argument about the code, it is the
//! shape of the code: there is no path on which a violation the old behaviour
//! did not report can be returned. A conforming payload takes the fast path and
//! costs exactly one validation, as before; only a payload that was already
//! being refused pays for the second pass.
//!
//! ## Why not the obvious implementation
//!
//! The first version of this module added the context and then dropped
//! violations whose focus node was not a payload SUBJECT — reasoning that a
//! write is answerable only for what it describes. That reasoning is wrong, and
//! the at-scale measurement caught it where the argument did not:
//! `partition_soak` found chunk 4 of 5 newly REFUSED on a real 65 MB
//! projection. A module can be a payload subject via an `imports` edge while
//! the payload does not describe it at all; adding its type from the store then
//! made it a target of `CodeModuleShape`, whose `sh:minCount filePath` had
//! nothing to satisfy it — a write refused for a shape violation on a node it
//! merely mentioned.
//!
//! That is precisely the "a tightening refuses writes that currently succeed"
//! hazard, arrived at while trying to avoid it. The subset property above is
//! the fix, and it is structural rather than reasoned, which is the difference
//! that matters: the earlier version was also carefully reasoned.

use std::collections::BTreeSet;

use oxrdf::{NamedOrBlankNode, Term as OxTerm};
use oxrdfio::{RdfFormat, RdfParser};

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;

/// `rdf:type`, spelled in full because that is how it is interned.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// What a payload asserts, split into the parts write-time validation needs.
#[derive(Debug, Default)]
pub struct PayloadScope {
    /// Subjects the payload DESCRIBES — the only nodes this write answers for.
    pub subjects: BTreeSet<String>,
    /// IRIs the payload references in object position without typing them here.
    /// These are the nodes whose type may need to come from the store.
    pub untyped_references: BTreeSet<String>,
}

/// Read a payload's subjects and its untyped outbound references.
///
/// Parses rather than scans: a chunk is Turtle, and a string search for
/// `a bobbin:Foo` would miss prefixed forms, full-IRI `rdf:type`, and predicate
/// lists, each of which changes the answer in the unsafe direction (a reference
/// wrongly believed to be typed here means no context is fetched and the write
/// is refused exactly as before).
pub fn scope_of(payload_turtle: &str) -> Result<PayloadScope> {
    let mut subjects = BTreeSet::new();
    let mut typed_here = BTreeSet::new();
    let mut referenced = BTreeSet::new();

    let parser = RdfParser::from_format(RdfFormat::Turtle);
    for quad in parser.for_reader(payload_turtle.as_bytes()) {
        let quad = quad.map_err(|e| Error::InvalidValue(format!("RDF parse error: {e}")))?;
        if let NamedOrBlankNode::NamedNode(s) = &quad.subject {
            subjects.insert(s.as_str().to_string());
            if quad.predicate.as_str() == RDF_TYPE {
                typed_here.insert(s.as_str().to_string());
            }
        }
        if let OxTerm::NamedNode(o) = &quad.object
            && quad.predicate.as_str() != RDF_TYPE
        {
            referenced.insert(o.as_str().to_string());
        }
    }

    // A node the payload types needs nothing from the store; a node the payload
    // merely mentions might.
    let untyped_references = referenced.difference(&typed_here).cloned().collect();
    Ok(PayloadScope {
        subjects,
        untyped_references,
    })
}

/// The `rdf:type` triples the STORE holds for `iris`, as N-Triples.
///
/// Types ONLY. Pulling each referenced node's full description would be the
/// obvious generalisation and is deliberately not done: it turns one write into
/// an unbounded read, and every extra triple is another chance to make a node
/// the payload does not own into a validation target. `sh:class` needs the type
/// and nothing else.
///
/// Unknown IRIs are skipped silently — a reference to something the store has
/// never seen is not an error here, it is simply a constraint that will fail on
/// its own merits.
pub fn store_type_context(store: &Store, iris: &BTreeSet<String>) -> Result<String> {
    let Some(type_attr) = store.lookup(RDF_TYPE)? else {
        // No rdf:type interned at all — an empty store. No context to add.
        return Ok(String::new());
    };
    let mut out = String::new();
    for iri in iris {
        let Some(entity) = store.lookup(iri)? else {
            continue;
        };
        for fact in store.entity_facts(entity)? {
            if fact.attribute != type_attr {
                continue;
            }
            if let Value::Ref(type_id) = fact.value {
                let type_iri = store.resolve(type_id)?;
                out.push_str(&format!("<{iri}> <{RDF_TYPE}> <{type_iri}> .\n"));
            }
        }
    }
    Ok(out)
}

/// Validate `turtle` against `shapes`, using the store to REPAIR violations
/// caused by context the payload does not carry.
///
/// This is the write-time entry point. `shacl::validate_shapes` remains the
/// context-free form and is still correct for callers that genuinely have no
/// store — `/validate` with caller-supplied shapes, and the shape-authoring
/// tests.
#[cfg(feature = "shacl")]
pub fn validate_with_store_context(
    store: &Store,
    shapes_turtle: &str,
    data_turtle: &str,
) -> Result<crate::shacl::ValidationFeedback> {
    // Pass 1: the payload alone. This is the OLD behaviour and the ceiling on
    // what may be reported.
    let baseline = crate::shacl::validate_shapes(shapes_turtle, data_turtle)?;
    if baseline.conforms {
        // Fast path: nothing to repair, and no second validation is paid for.
        return Ok(baseline);
    }

    let scope = scope_of(data_turtle)?;
    let context = store_type_context(store, &scope.untyped_references)?;
    if context.is_empty() {
        return Ok(baseline);
    }

    // Pass 2: the same payload with the store's type triples. Turtle and
    // N-Triples concatenate cleanly, and the context is written in full-IRI
    // form precisely so it cannot depend on the payload's prefixes.
    let augmented =
        crate::shacl::validate_shapes(shapes_turtle, &format!("{data_turtle}\n{context}"))?;
    Ok(repaired(baseline, &augmented))
}

/// The baseline's violations MINUS the ones the store context resolved.
///
/// Subtraction, never union: an issue present only in the augmented run is
/// something the CONTEXT introduced (a referenced node becoming a validation
/// target), and reporting it would refuse a write for a fact it does not
/// assert. Those are dropped by construction here — not by a filter that has to
/// be right about targeting.
///
/// Recounted from the kept issues rather than carried over from either report:
/// a result that said `conforms: false, violations: 0` would be a refusal with
/// nothing to show for it.
fn repaired(
    baseline: crate::shacl::ValidationFeedback,
    augmented: &crate::shacl::ValidationFeedback,
) -> crate::shacl::ValidationFeedback {
    let still: BTreeSet<String> = augmented.results.iter().map(issue_key).collect();
    let results: Vec<_> = baseline
        .results
        .into_iter()
        .filter(|issue| still.contains(&issue_key(issue)))
        .collect();
    let violations = results
        .iter()
        .filter(|r| r.severity.to_ascii_lowercase().contains("violation"))
        .count();
    let warnings = results
        .iter()
        .filter(|r| r.severity.to_ascii_lowercase().contains("warning"))
        .count();
    crate::shacl::ValidationFeedback {
        conforms: violations == 0,
        violations,
        warnings,
        results,
        resolution_candidates: baseline.resolution_candidates,
    }
}

/// Identity of a validation issue across two runs of the same shapes.
///
/// Deliberately excludes `source_shape`: rudof labels an anonymous property
/// shape with a fresh blank-node id per run (`_:ad3f1b9a…`), so including it
/// would make every issue unique and the subtraction a no-op — the fix would
/// silently stop repairing anything while every test that checks REFUSALS kept
/// passing.
fn issue_key(issue: &crate::shacl::ValidationIssue) -> String {
    format!(
        "{}|{}|{}|{}",
        issue.focus_node,
        issue.component,
        issue.path.as_deref().unwrap_or(""),
        issue.value.as_deref().unwrap_or(""),
    )
}

#[cfg(test)]
#[path = "shacl_context_tests.rs"]
mod shacl_context_tests;
