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
    store_type_context_in_graph(store, iris, crate::schema::ROOT_GRAPH)
}

/// [`store_type_context`], scoped to the graph a write is landing in (quipu-080).
///
/// The context is the UNION of the target graph's types and ROOT's. That is the
/// semantics `docs/design/named-graphs.md` already sanctions for the shapes
/// half: ontology lives in ROOT and applies to every destination graph, so a
/// node ROOT types (an `aegis:` class, a shared entity) repairs a plane-routed
/// write exactly as it repairs a ROOT write. The graph's own half is the fix
/// for the aegis-fp17f/aegis-sd5fj defect class re-surfacing on `/knot`'s
/// `graph` param: a chunked write into a named committed graph whose earlier
/// chunks typed nodes IN THAT GRAPH must see those types too.
///
/// What is deliberately NOT unioned: any third graph. Planes are trust
/// boundaries — camayoc's ingress discipline quarantines model-inferred facts
/// in low-trust planes precisely so they cannot masquerade — and a quarantined
/// graph's `rdf:type` claims repairing a write into another plane would be
/// exactly that masquerade. Deduplicated because ROOT and the target graph may
/// both hold the same type triple, and `g = ROOT` reduces to the old behaviour.
pub fn store_type_context_in_graph(
    store: &Store,
    iris: &BTreeSet<String>,
    g: i64,
) -> Result<String> {
    let Some(type_attr) = store.lookup(RDF_TYPE)? else {
        // No rdf:type interned at all — an empty store. No context to add.
        return Ok(String::new());
    };
    let mut lines = BTreeSet::new();
    let mut graphs = vec![crate::schema::ROOT_GRAPH];
    if g != crate::schema::ROOT_GRAPH {
        graphs.push(g);
    }
    for iri in iris {
        let Some(entity) = store.lookup(iri)? else {
            continue;
        };
        for &scope in &graphs {
            for fact in store.entity_facts_in_graph(entity, scope)? {
                if fact.attribute != type_attr {
                    continue;
                }
                if let Value::Ref(type_id) = fact.value {
                    let type_iri = store.resolve(type_id)?;
                    lines.insert(format!("<{iri}> <{RDF_TYPE}> <{type_iri}> .\n"));
                }
            }
        }
    }
    Ok(lines.into_iter().collect())
}

/// SHACL paths that some property shape REQUIRES (`sh:minCount >= 1`).
///
/// Deliberately not per-`sh:targetClass`: the answer is only used to decide
/// which of a payload subject's EXISTING facts are worth fetching, and fetching
/// a fact that turns out to be irrelevant costs one row and cannot change a
/// verdict (see [`repaired`]). Resolving path-to-class properly would mean
/// implementing target selection a second time, beside the validator that
/// already does it — two implementations that must agree, and only one of them
/// exercised by the tests.
///
/// The set is small in practice: the shape files hold a label floor plus a
/// handful of emitter-proven scalars.
pub fn required_paths(shapes_turtle: &str) -> Result<BTreeSet<String>> {
    use oxttl::TurtleParser;
    use std::collections::BTreeMap;

    const SH_PATH: &str = "http://www.w3.org/ns/shacl#path";
    const SH_MIN_COUNT: &str = "http://www.w3.org/ns/shacl#minCount";

    let parser = TurtleParser::new()
        .with_base_iri("http://example.org/")
        .map_err(|e| Error::InvalidValue(format!("shapes base IRI: {e}")))?;

    // A property shape is usually a BLANK node (`sh:property [ ... ]`), so the
    // path and the count arrive as two triples about the same subject and have
    // to be joined rather than read off one.
    let mut path_of: BTreeMap<String, String> = BTreeMap::new();
    let mut required: BTreeSet<String> = BTreeSet::new();

    for triple in parser.for_reader(shapes_turtle.as_bytes()) {
        let triple = triple.map_err(|e| Error::InvalidValue(format!("shapes parse error: {e}")))?;
        let subject = triple.subject.to_string();
        match triple.predicate.as_str() {
            SH_PATH => {
                if let OxTerm::NamedNode(path) = &triple.object {
                    path_of.insert(subject, path.as_str().to_string());
                }
            }
            SH_MIN_COUNT => {
                if let OxTerm::Literal(lit) = &triple.object
                    && lit.value().parse::<i64>().is_ok_and(|n| n >= 1)
                {
                    required.insert(subject);
                }
            }
            _ => {}
        }
    }

    Ok(required
        .into_iter()
        .filter_map(|s| path_of.get(&s).cloned())
        .collect())
}

/// The store's values for `paths` on the payload's own SUBJECTS, as N-Triples.
///
/// # Why subjects, when [`store_type_context_in_graph`] is about references
///
/// That function fixes `sh:class` on a node the payload MENTIONS. This one
/// fixes a different defect with the opposite shape (aegis-dixug): adding a
/// governed type to an EXISTING, fully conformant node is refused unless the
/// payload also restates every property that node's shape requires — because
/// `sh:targetClass` matching sees the type in the payload, while every OTHER
/// constraint on that shape is evaluated against the payload alone.
///
/// ```text
/// # store already holds: <guard> rdfs:label "guard"
/// POST /knot  "<guard> a aegis:OperationalRule ."
///   -> conforms:false, MinCount(1) not satisfied, path=rdfs:label
/// ```
///
/// The message names a label that is RIGHT THERE, so it sends the caller
/// looking for a missing fact, finding one, and concluding the store is
/// inconsistent. That is not a corner case: it is THE SHAPE OF EVERY
/// INCREMENTAL WRITE — adding a type, an edge, one property. The narrower and
/// more careful the write, the likelier it is refused.
///
/// # Why this is bounded, unlike the generalisation the module warns off
///
/// [`store_type_context_in_graph`] declines to pull each referenced node's full
/// description because that turns one write into an unbounded read. This is not
/// that: it is restricted to paths some shape REQUIRES, so it fetches the
/// handful of scalars a floor is made of, never a node's edge set. A payload
/// subject is also a node the write ANSWERS for, which a mere reference is not.
///
/// Safety is structural either way — [`repaired`] subtracts, so no context can
/// introduce a violation into the reported result. This can only turn a refusal
/// into a pass, never the reverse.
pub fn store_property_context_in_graph(
    store: &Store,
    subjects: &BTreeSet<String>,
    paths: &BTreeSet<String>,
    g: i64,
) -> Result<String> {
    if subjects.is_empty() || paths.is_empty() {
        return Ok(String::new());
    }
    // Resolve the required paths to term ids ONCE. A path the store has never
    // interned cannot be on any fact, so it drops out here rather than being
    // compared per fact.
    let mut wanted = BTreeSet::new();
    for path in paths {
        if let Some(id) = store.lookup(path)? {
            wanted.insert(id);
        }
    }
    if wanted.is_empty() {
        return Ok(String::new());
    }

    let mut graphs = vec![crate::schema::ROOT_GRAPH];
    if g != crate::schema::ROOT_GRAPH {
        graphs.push(g);
    }

    let mut lines = BTreeSet::new();
    for iri in subjects {
        let Some(entity) = store.lookup(iri)? else {
            continue;
        };
        for &scope in &graphs {
            for fact in store.entity_facts_in_graph(entity, scope)? {
                if !wanted.contains(&fact.attribute) {
                    continue;
                }
                let attr = store.resolve(fact.attribute)?;
                // A value this store holds but cannot render as an RDF term
                // (raw bytes) is skipped, never fatal: this is a repair pass
                // beside a write that has already been judged once.
                let Ok(term) = crate::rdf::value_to_term(store, &fact.value) else {
                    continue;
                };
                lines.insert(format!("<{iri}> <{attr}> {term} .\n"));
            }
        }
    }
    Ok(lines.into_iter().collect())
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
    validate_with_store_context_in_graph(
        store,
        shapes_turtle,
        data_turtle,
        crate::schema::ROOT_GRAPH,
    )
}

/// [`validate_with_store_context`] for a write landing in graph `g` (quipu-080):
/// the repair context is the target graph's type triples unioned with ROOT's —
/// see [`store_type_context_in_graph`] for why that union and no wider. `/knot`
/// threads its resolved destination graph here; `g = ROOT` is byte-identical to
/// the two-argument form.
#[cfg(feature = "shacl")]
pub fn validate_with_store_context_in_graph(
    store: &Store,
    shapes_turtle: &str,
    data_turtle: &str,
    g: i64,
) -> Result<crate::shacl::ValidationFeedback> {
    // Pass 1: the payload alone. This is the OLD behaviour and the ceiling on
    // what may be reported.
    let baseline = crate::shacl::validate_shapes(shapes_turtle, data_turtle)?;
    if baseline.conforms {
        // Fast path: nothing to repair, and no second validation is paid for.
        return Ok(baseline);
    }

    let scope = scope_of(data_turtle)?;
    // Two repairs, deliberately additive and deliberately different in kind:
    //   - TYPES for nodes the payload merely REFERENCES  (aegis-fp17f/sd5fj):
    //     fixes `sh:class` on a value whose type lives in the store.
    //   - REQUIRED PROPERTIES on the payload's own SUBJECTS (aegis-dixug):
    //     fixes an incremental write being refused for a floor the store
    //     already satisfies.
    // Both are bounded, and neither can add a violation — `repaired` subtracts.
    let mut context = store_type_context_in_graph(store, &scope.untyped_references, g)?;
    context.push_str(&store_property_context_in_graph(
        store,
        &scope.subjects,
        &required_paths(shapes_turtle)?,
        g,
    )?);
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
