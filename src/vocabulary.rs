//! The sanctioned class vocabulary, and what a write is about to add to it.
//!
//! # Why this module exists
//!
//! SHACL fires through `sh:targetClass`. An entity typed with a class that no
//! shape targets is therefore **not validated by anything** — there is no shape
//! to violate, so it is untargeted and vacuously conformant. That makes the one
//! error agents actually make — minting a plausible-sounding class that does not
//! exist — the single error shapes structurally cannot catch (aegis-7n1ya).
//!
//! It is not a corner case. It was measured at 405 entities across 5 invented
//! classes, every one written behind an HTTP 200 with a healthy `count`. And it
//! is not only a hand-written-episode problem: an integration publishing a
//! governed snapshot through `/knot` hits it exactly the same way, at machine
//! scale, while its own `conforms != false` guard reports success (aegis-6noan).
//!
//! So this is deliberately an ADVISORY, not a gate. It reports what a write
//! ADDED to the vocabulary without declaring it; it never refuses. Two reasons,
//! both learned here the expensive way:
//!
//!   1. A gate on an undeclared type would refuse the 232 entities already
//!      carrying one, on every subsequent touch, for a defect they did not
//!      cause. Advise before enforce.
//!   2. The reported failure mode is never "the write was rejected". It is "the
//!      author read a 200 as confirmation that the type was fine." A hint in the
//!      response is aimed at exactly that reader, at exactly that moment.
//!
//! Retiring the advisory in favour of a gate is a real option later — but only
//! once the tail is empty, and never by deleting this: the hint is what makes
//! the tail measurable in the first place.

use std::collections::BTreeSet;

use crate::error::{Error, Result};
use crate::store::Store;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const TARGET_CLASS: &str = "http://www.w3.org/ns/shacl#targetClass";
const SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

/// The class IRIs sanctioned by the shape sets currently loaded in the store.
///
/// `sh:targetClass` plus declared `rdfs:subClassOf` OBJECTS. The second half is
/// load-bearing and easy to drop: abstract parents such as `Service` and `Host`
/// are query-only classes with no target shape of their own, but they are
/// legitimate IRIs, and a vocabulary that omitted them would report every
/// correct `Service` write as ungoverned — the advisory would then be loudest
/// about the cases that are right.
pub fn sanctioned(store: &Store) -> Result<BTreeSet<String>> {
    use oxttl::TurtleParser;

    let mut classes = BTreeSet::new();
    for (name, turtle, _) in store.list_shapes()? {
        let parser = TurtleParser::new()
            .with_base_iri("http://example.org/")
            .map_err(|e| Error::InvalidValue(format!("shape set '{name}' base IRI: {e}")))?;
        for result in parser.for_reader(turtle.as_bytes()) {
            let triple = result.map_err(|e| {
                Error::InvalidValue(format!("shape set '{name}' Turtle parse error: {e}"))
            })?;
            let predicate = triple.predicate.as_str();
            if (predicate == TARGET_CLASS || predicate == SUBCLASS_OF)
                && let oxrdf::Term::NamedNode(class) = triple.object
            {
                classes.insert(class.as_str().to_owned());
            }
        }
    }
    Ok(classes)
}

/// Class IRIs asserted as an `rdf:type` in `turtle` that no loaded shape set
/// sanctions, sorted and de-duplicated.
///
/// A parse error yields an EMPTY hint list rather than an error: this runs
/// beside a write that has already parsed and committed, and an advisory must
/// never be able to fail a write that succeeded. The caller has already
/// reported any real parse problem.
pub fn ungoverned_types_in_turtle(turtle: &str, sanctioned: &BTreeSet<String>) -> Vec<String> {
    use oxttl::TurtleParser;

    let Ok(parser) = TurtleParser::new().with_base_iri("http://example.org/") else {
        return Vec::new();
    };
    let mut found = BTreeSet::new();
    for result in parser.for_reader(turtle.as_bytes()) {
        let Ok(triple) = result else {
            // Partial results are still worth reporting: the write itself
            // parsed, so a failure here is this parser's own base-IRI handling,
            // not the payload's. Keep what we resolved and stop.
            break;
        };
        if triple.predicate.as_str() == RDF_TYPE
            && let oxrdf::Term::NamedNode(class) = triple.object
            && !sanctioned.contains(class.as_str())
        {
            found.insert(class.as_str().to_owned());
        }
    }
    found.into_iter().collect()
}

/// Build the advisory payload for a response, or `None` when everything the
/// write typed is governed.
///
/// `None` rather than an empty list ON PURPOSE, so the field is ABSENT on a
/// clean write. A field that is always present teaches readers to skip it, and
/// this one has exactly one job: to be noticed the one time it appears.
pub fn hint_json(ungoverned: Vec<String>) -> Option<serde_json::Value> {
    if ungoverned.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "ungoverned_types": ungoverned,
        "meaning": "these rdf:type IRIs are targeted by NO loaded shape, so SHACL \
                    could not validate them and `conforms: true` says nothing about \
                    them. The write SUCCEEDED and was not refused.",
        "next": "if the type is intended, declare it (add an sh:targetClass shape and \
                 load it) — otherwise re-type the nodes to a sanctioned class. \
                 POST /shapes {\"action\":\"vocabulary\"} lists what is sanctioned.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn reports_a_type_no_shape_targets() {
        let v = vocab(&["http://ex.org/Known"]);
        let ttl = r#"<http://ex.org/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex.org/Invented> ."#;
        assert_eq!(
            ungoverned_types_in_turtle(ttl, &v),
            vec!["http://ex.org/Invented".to_string()]
        );
    }

    #[test]
    fn stays_silent_when_every_type_is_sanctioned() {
        let v = vocab(&["http://ex.org/Known"]);
        let ttl = r#"<http://ex.org/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex.org/Known> ."#;
        assert!(ungoverned_types_in_turtle(ttl, &v).is_empty());
        assert!(hint_json(Vec::new()).is_none());
    }

    /// The aegis-6noan case: a dual-typed node where ONE of the two types is
    /// governed. `conforms: true` is truthful — the governed half validated —
    /// and the undeclared half is exactly what nothing else would report.
    #[test]
    fn reports_the_undeclared_half_of_a_dual_typed_node() {
        let v = vocab(&["http://aegis.gastown.local/ontology/CodeSymbol"]);
        let ttl = r#"<http://ex.org/c1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://aegis.gastown.local/ontology/Chunk> , <http://aegis.gastown.local/ontology/CodeSymbol> ."#;
        assert_eq!(
            ungoverned_types_in_turtle(ttl, &v),
            vec!["http://aegis.gastown.local/ontology/Chunk".to_string()]
        );
    }

    #[test]
    fn deduplicates_across_many_nodes() {
        let v = vocab(&[]);
        let ttl = r#"
            <http://ex.org/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex.org/X> .
            <http://ex.org/b> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex.org/X> .
            <http://ex.org/c> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex.org/Y> .
        "#;
        assert_eq!(
            ungoverned_types_in_turtle(ttl, &v),
            vec!["http://ex.org/X".to_string(), "http://ex.org/Y".to_string()]
        );
    }

    /// Only rdf:type is inspected. A non-type predicate pointing at an
    /// undeclared IRI is not a vocabulary problem and must not be reported —
    /// an advisory that cries wolf gets filtered out by its readers.
    #[test]
    fn ignores_predicates_other_than_rdf_type() {
        let v = vocab(&[]);
        let ttl = r#"<http://ex.org/a> <http://ex.org/mentions> <http://ex.org/NotAType> ."#;
        assert!(ungoverned_types_in_turtle(ttl, &v).is_empty());
    }

    #[test]
    fn hint_names_the_types_and_says_the_write_succeeded() {
        let hint = hint_json(vec!["http://ex.org/Invented".to_string()]).expect("hint");
        assert_eq!(hint["ungoverned_types"][0], "http://ex.org/Invented");
        assert!(hint["meaning"].as_str().unwrap().contains("SUCCEEDED"));
    }
}
