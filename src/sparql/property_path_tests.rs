//! Property paths that END on a literal (aegis-sxlptn).
//!
//! The path engine worked on (node id, node id) pairs and dropped every
//! literal object, so `?s (p|p) ?v` returned 0 rows where `?s p ?v` returned 1,
//! and `(p1|p2)` lost exactly the literal-valued rows its UNION form keeps.
//! A constant literal object also resolved to "unbound", so `?s (p|q) "x"`
//! returned every subject with any node edge on the path.

use oxrdfio::RdfFormat;

use super::query;
use crate::rdf::ingest_rdf;
use crate::store::Store;

fn store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
ex:a ex:edge ex:b .
ex:b ex:edge ex:c .
ex:a ex:label "A" .
ex:b ex:label "B" .
ex:c ex:qlabel "C" .
ex:s ex:owner "crew" .
ex:t ex:qowner ex:y .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

fn rows(store: &Store, q: &str) -> usize {
    query(store, q).unwrap().rows().len()
}

const EX: &str = "PREFIX ex: <http://example.org/> ";

#[test]
fn same_predicate_alternation_equals_the_predicate_for_literal_objects() {
    let s = store();
    let plain = rows(&s, &format!("{EX} SELECT ?s ?v WHERE {{ ?s ex:label ?v }}"));
    assert_eq!(plain, 2, "control: two literal labels");
    let alt = rows(
        &s,
        &format!("{EX} SELECT ?s ?v WHERE {{ ?s (ex:label|ex:label) ?v }}"),
    );
    assert_eq!(alt, plain, "(p|p) must equal p for literal objects");
}

#[test]
fn alternation_equals_union_for_mixed_literal_and_node_objects() {
    let s = store();
    let union = rows(
        &s,
        &format!("{EX} SELECT ?s ?v WHERE {{ {{ ?s ex:owner ?v }} UNION {{ ?s ex:qowner ?v }} }}"),
    );
    assert_eq!(union, 2, "control: one literal owner, one node owner");
    let alt = rows(
        &s,
        &format!("{EX} SELECT ?s ?v WHERE {{ ?s (ex:owner|ex:qowner) ?v }}"),
    );
    assert_eq!(alt, union, "(p1|p2) must equal its UNION");
}

#[test]
fn a_closure_can_end_on_a_literal() {
    let s = store();
    // From ex:a: nodes b, c by edge/edge; literals "A" (a) and "B" (b) by label.
    let n = rows(
        &s,
        &format!("{EX} SELECT ?v WHERE {{ ex:a (ex:edge|ex:label)+ ?v }}"),
    );
    assert_eq!(n, 4, "b, c, \"A\", \"B\"");
}

#[test]
fn a_constant_literal_object_matches_only_that_literal() {
    let s = store();
    // Before: "B" resolved to unbound, and the node engine returned every
    // subject with an ex:edge (a and b), with "B" never checked.
    let n = rows(
        &s,
        &format!("{EX} SELECT ?s WHERE {{ ?s (ex:edge|ex:label) \"B\" }}"),
    );
    assert_eq!(n, 1, "only ex:b has the literal \"B\"");
}

#[test]
fn a_bound_literal_object_is_matched_not_ignored() {
    let s = store();
    let n = rows(
        &s,
        &format!("{EX} SELECT ?s WHERE {{ VALUES ?v {{ \"A\" }} ?s (ex:edge|ex:label) ?v }}"),
    );
    assert_eq!(n, 1, "only ex:a has the literal \"A\"");
}

#[test]
fn zero_or_more_to_a_constant_literal_is_only_its_own_zero_length_path() {
    let s = store();
    // Zero-length: ?s = "B". No ex:edge step ends on a literal.
    let n = rows(&s, &format!("{EX} SELECT ?s WHERE {{ ?s ex:edge* \"B\" }}"));
    assert_eq!(
        n, 1,
        "the identity row for \"B\" only, not one per literal in the store"
    );
}
