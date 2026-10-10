//! Tests for the `VALUES` bind join (aegis-roth88).
//!
//! Two kinds: the bind join must ENGAGE where it is meant to (otherwise the
//! semantic tests below pass in both worlds), and every engaged shape must
//! answer exactly what the hash join answers.

use oxrdfio::RdfFormat;
use spargebra::algebra::GraphPattern;
use spargebra::{Query, SparqlParser};

use super::bind_join::{BIND_JOIN_MAX_ROWS, seed_safe, try_values_bind_join};
use super::pattern::eval_pattern_seeded;
use super::pattern_util::join_rows;
use super::{Bindings, TemporalContext, query};
use crate::rdf::{ingest_rdf, ingest_rdf_to_graph};
use crate::store::Store;

const G: &str = "http://example.org/g";

fn store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let root = r#"@prefix ex: <http://example.org/> .
ex:alice ex:name "Alice" ; ex:age 30 .
ex:bob ex:name "Bob" ; ex:age 25 .
ex:carol ex:name "Carol" .
"#;
    ingest_rdf(
        &mut store,
        root.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-07T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let g = store.graph_create(G).unwrap();
    let named = r#"@prefix ex: <http://example.org/> .
ex:alice ex:role "admin" ; ex:team ex:red .
ex:bob ex:role "user" .
ex:dave ex:role "user" .
"#;
    ingest_rdf_to_graph(
        &mut store,
        named.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-07T00:00:00Z",
        None,
        None,
        g,
    )
    .unwrap();
    store
}

/// The pattern under the top-level projection of a SELECT.
fn where_clause(sparql: &str) -> GraphPattern {
    let Query::Select { pattern, .. } = SparqlParser::new().parse_query(sparql).unwrap() else {
        panic!("not a SELECT");
    };
    let mut p = pattern;
    loop {
        p = match p {
            GraphPattern::Project { inner, .. } | GraphPattern::Distinct { inner } => *inner,
            other => return other,
        };
    }
}

fn join_operands(p: &GraphPattern) -> (&GraphPattern, &GraphPattern) {
    match p {
        GraphPattern::Join { left, right } => (left, right),
        GraphPattern::Graph { inner, .. } => join_operands(inner),
        other => panic!("expected a Join, got {other:?}"),
    }
}

/// Rows as sorted strings, so bag equality ignores row order but keeps counts.
fn bag(rows: &[Bindings]) -> Vec<String> {
    let mut v: Vec<String> = rows
        .iter()
        .map(|r| {
            let mut kv: Vec<String> = r.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
            kv.sort();
            kv.join(",")
        })
        .collect();
    v.sort();
    v
}

/// The bind join's rows next to the hash join's for the same Join node.
fn both_ways(store: &Store, sparql: &str, ctx: &TemporalContext) -> (Vec<String>, Vec<String>) {
    let p = where_clause(sparql);
    let (left, right) = join_operands(&p);
    let seed = Bindings::new();
    let (bind_rows, _) = try_values_bind_join(store, left, right, ctx, &seed)
        .unwrap()
        .expect("the bind join must engage for this shape");
    let (l, _) = eval_pattern_seeded(store, left, ctx, &seed).unwrap();
    let (r, _) = eval_pattern_seeded(store, right, ctx, &seed).unwrap();
    let hash_rows = join_rows(&l, &r, ctx).unwrap();
    (bag(&bind_rows), bag(&hash_rows))
}

#[test]
fn engages_for_values_then_bgp_and_matches_the_hash_join() {
    let store = store();
    let (bind, hash) = both_ways(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { VALUES ?s { ex:alice ex:carol ex:nobody } ?s ?p ?o }"#,
        &TemporalContext::default(),
    );
    assert_eq!(bind, hash);
    assert_eq!(bind.len(), 3, "alice has 2 facts, carol 1, nobody 0");
}

#[test]
fn engages_for_bgp_then_values() {
    let store = store();
    let (bind, hash) = both_ways(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { ?s ex:name ?n VALUES ?s { ex:bob } }"#,
        &TemporalContext::default(),
    );
    assert_eq!(bind, hash);
    assert_eq!(bind.len(), 1);
}

#[test]
fn the_seeds_shape_values_outside_graph_matches_end_to_end() {
    // The exact seeds read_subjects shape, against the UNION it must equal.
    let store = store();
    let values = query(
        &store,
        &format!(
            "PREFIX ex: <http://example.org/>
             SELECT ?s ?p ?o WHERE {{ VALUES ?s {{ ex:alice ex:dave ex:carol }} GRAPH <{G}> {{ ?s ?p ?o }} }}"
        ),
    )
    .unwrap();
    let union = query(
        &store,
        &format!(
            "PREFIX ex: <http://example.org/>
             SELECT ?s ?p ?o WHERE {{
               {{ BIND(ex:alice AS ?s) GRAPH <{G}> {{ ex:alice ?p ?o }} }} UNION
               {{ BIND(ex:dave AS ?s) GRAPH <{G}> {{ ex:dave ?p ?o }} }} UNION
               {{ BIND(ex:carol AS ?s) GRAPH <{G}> {{ ex:carol ?p ?o }} }} }}"
        ),
    )
    .unwrap();
    assert_eq!(bag(values.rows()), bag(union.rows()));
    assert_eq!(
        values.rows().len(),
        3,
        "alice 2 + dave 1; carol has none in G"
    );
}

#[test]
fn values_inside_graph_matches_values_outside() {
    let store = store();
    let inside = query(
        &store,
        &format!(
            "PREFIX ex: <http://example.org/>
             SELECT ?s ?p ?o WHERE {{ GRAPH <{G}> {{ VALUES ?s {{ ex:alice ex:bob }} ?s ?p ?o }} }}"
        ),
    )
    .unwrap();
    let outside = query(
        &store,
        &format!(
            "PREFIX ex: <http://example.org/>
             SELECT ?s ?p ?o WHERE {{ VALUES ?s {{ ex:alice ex:bob }} GRAPH <{G}> {{ ?s ?p ?o }} }}"
        ),
    )
    .unwrap();
    assert_eq!(bag(inside.rows()), bag(outside.rows()));
    assert_eq!(inside.rows().len(), 3);
}

#[test]
fn undef_rows_and_duplicate_rows_keep_join_multiplicity() {
    // UNDEF leaves ?s unbound for that row (an unconstrained match), and a
    // duplicated row must duplicate its solutions, exactly as the hash join.
    let store = store();
    let (bind, hash) = both_ways(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { VALUES (?s ?n) { (ex:bob UNDEF) (ex:bob UNDEF) (UNDEF "Carol") } ?s ex:name ?n }"#,
        &TemporalContext::default(),
    );
    assert_eq!(bind, hash);
    assert_eq!(bind.len(), 3, "bob twice, carol once");
}

#[test]
fn a_seed_that_conflicts_with_a_values_row_drops_it() {
    let store = store();
    let p = where_clause(
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { VALUES ?s { ex:alice ex:bob } ?s ex:name ?n }"#,
    );
    let (left, right) = join_operands(&p);
    let mut seed = Bindings::new();
    let bob = store.lookup("http://example.org/bob").unwrap().unwrap();
    seed.insert("s".into(), crate::types::Value::Ref(bob));
    let (rows, _) = try_values_bind_join(&store, left, right, &TemporalContext::default(), &seed)
        .unwrap()
        .unwrap();
    assert_eq!(rows.len(), 1, "only the bob row agrees with the seed");
}

#[test]
fn empty_values_yields_no_rows_and_the_same_header() {
    let store = store();
    let p = where_clause(
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { VALUES ?s { } ?s ex:name ?n }"#,
    );
    let (left, right) = join_operands(&p);
    let (rows, vars) = try_values_bind_join(
        &store,
        left,
        right,
        &TemporalContext::default(),
        &Bindings::new(),
    )
    .unwrap()
    .unwrap();
    assert!(rows.is_empty());
    assert_eq!(vars, vec!["s".to_string(), "n".to_string()]);
}

#[test]
fn declines_when_the_other_side_could_observe_substitution() {
    // An OPTIONAL whose FILTER reads the VALUES variable answers differently
    // under substitution (?x bound) than bottom-up (?x unbound inside the
    // group), so the bind join must NOT engage. The inner group makes the
    // algebra Join(Values, LeftJoin), the shape the guard has to refuse.
    let p = where_clause(
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { VALUES ?x { 26 } { ?s ex:age ?a OPTIONAL { ?s ex:name ?n FILTER(?a > ?x) } } }"#,
    );
    let (left, right) = join_operands(&p);
    assert!(matches!(left, GraphPattern::Values { .. }), "{left:?}");
    assert!(matches!(right, GraphPattern::LeftJoin { .. }), "{right:?}");
    let store = store();
    assert!(
        try_values_bind_join(
            &store,
            left,
            right,
            &TemporalContext::default(),
            &Bindings::new()
        )
        .unwrap()
        .is_none()
    );
    // And end to end the answer stays the bottom-up one: inside the group ?x
    // is unbound, so the FILTER errors and no ?n is ever attached.
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT * WHERE { VALUES ?x { 26 } { ?s ex:age ?a OPTIONAL { ?s ex:name ?n FILTER(?a > ?x) } } }"#,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 2, "alice and bob have an age");
    assert!(
        result.rows().iter().all(|r| !r.contains_key("n")),
        "{:?}",
        result.rows()
    );
}

#[test]
fn seed_safe_admits_only_bgp_graph_and_join() {
    for (q, want) in [
        ("SELECT * WHERE { ?s ?p ?o }", true),
        ("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }", true),
        ("SELECT * WHERE { ?s ?p ?o FILTER(?o = 1) }", false),
        ("SELECT * WHERE { ?s ?p ?o OPTIONAL { ?o ?q ?r } }", false),
        ("SELECT * WHERE { ?s ?p ?o MINUS { ?s ?q ?r } }", false),
        ("SELECT * WHERE { { ?s ?p ?o } UNION { ?o ?p ?s } }", false),
    ] {
        assert_eq!(seed_safe(&where_clause(q)), want, "{q}");
    }
}

#[test]
fn declines_above_the_row_ceiling() {
    let store = store();
    let rows: String = (0..=BIND_JOIN_MAX_ROWS)
        .map(|i| format!("<http://example.org/n{i}> "))
        .collect();
    let p = where_clause(&format!(
        "SELECT * WHERE {{ VALUES ?s {{ {rows} }} ?s ?p ?o }}"
    ));
    let (left, right) = join_operands(&p);
    assert!(
        try_values_bind_join(
            &store,
            left,
            right,
            &TemporalContext::default(),
            &Bindings::new()
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn a_values_graph_iri_the_store_lacks_matches_no_graph() {
    // wu's quipu#429 review: an unknown graph IRI binds ?g as a Str, and the
    // GRAPH ?g arm used to treat that as unbound, scanning every graph and
    // relabelling ?g. Under the bind join that doubled the rows.
    let store = store();
    let both = query(
        &store,
        &format!(
            "SELECT * WHERE {{ VALUES ?g {{ <{G}> <http://example.org/nog> }} GRAPH ?g {{ ?s ?p ?o }} }}"
        ),
    )
    .unwrap();
    assert_eq!(
        both.rows().len(),
        4,
        "only the 4 facts of G: {:?}",
        both.rows()
    );
    let absent = query(
        &store,
        "SELECT * WHERE { VALUES ?g { <http://example.org/nog> } GRAPH ?g { ?s ?p ?o } }",
    )
    .unwrap();
    assert_eq!(absent.rows().len(), 0, "{:?}", absent.rows());
    // The same answer through the hash join (VALUES above the row ceiling is
    // not needed: compare against the bind join's own operands directly).
    let (bind, hash) = both_ways(
        &store,
        &format!(
            "SELECT * WHERE {{ VALUES ?g {{ <{G}> <http://example.org/nog> }} GRAPH ?g {{ ?s ?p ?o }} }}"
        ),
        &TemporalContext::default(),
    );
    assert_eq!(bind, hash);
}
