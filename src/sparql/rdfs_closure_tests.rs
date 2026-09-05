//! Tests for the materialised RDFS closure.
//!
//! Each rule is pinned by the W3C case that needs it, so a reader can trace a
//! test back to the thing it makes pass rather than to a rule number.

use super::rdfs_closure::materialise;
use crate::namespace::{RDF_TYPE, RDFS_DOMAIN, RDFS_RANGE, RDFS_SUBCLASS_OF, RDFS_SUBPROPERTY_OF};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const TS: &str = "2026-09-05T00:00:00Z";
const G: &str = "http://example.org/graph/a";

fn triple(store: &mut Store, graph: &str, s: &str, p: &str, o: &str) {
    let (e, a) = (store.intern(s).unwrap(), store.intern(p).unwrap());
    let v = store.intern(o).unwrap();
    let g = store.intern(graph).unwrap();
    store
        .transact_to_graph(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Ref(v),
                valid_from: TS.into(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            None,
            None,
            g,
        )
        .unwrap();
}

/// Assert `s p "lit"` — a LITERAL-valued triple, which `closure_of` cannot see.
fn literal_triple(store: &mut Store, graph: &str, s: &str, p: &str, lit: &str) {
    let (e, a) = (store.intern(s).unwrap(), store.intern(p).unwrap());
    let g = store.intern(graph).unwrap();
    store
        .transact_to_graph(
            &[Datum {
                entity: e,
                attribute: a,
                value: Value::Str(lit.into()),
                valid_from: TS.into(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            None,
            None,
            g,
        )
        .unwrap();
}

/// Every IRI-object triple visible in the graph and its companion.
fn closure_of(store: &mut Store, graph: &str) -> Vec<(String, String, String)> {
    let g = store.lookup(graph).unwrap().unwrap();
    let companion = store.ensure_companion_inferred_graph(g, TS).unwrap();
    let mut stmt = store
        .prepare("SELECT e, a, v FROM facts WHERE g IN (?1, ?2) AND op = 1 AND valid_to IS NULL")
        .unwrap();
    let rows: Vec<(i64, i64, Vec<u8>)> = stmt
        .query_map([g, companion], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, Vec<u8>>(2)?))
        })
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    drop(stmt);
    let mut out = Vec::new();
    for (e, a, raw) in rows {
        if let Ok(Value::Ref(o)) = Value::from_bytes(&raw) {
            out.push((
                store.resolve(e).unwrap(),
                store.resolve(a).unwrap(),
                store.resolve(o).unwrap(),
            ));
        }
    }
    out
}

fn has(c: &[(String, String, String)], s: &str, p: &str, o: &str) -> bool {
    c.iter().any(|(a, b, d)| a == s && b == p && d == o)
}

#[test]
fn rdfs7_subproperty_is_the_case_neither_other_mechanism_can_express() {
    // The W3C `rdfs01` shape exactly: ex:a ex:b1 ex:c + b1 subPropertyOf b2,
    // asked as `SELECT ?x WHERE { ex:a ?x ex:c }`, expecting {b1, b2}.
    // `?x` is a VARIABLE PREDICATE, so no pattern rewrite can produce b2 --
    // the triple must EXIST. That is what this materialises.
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", "http://ex/b1", "http://ex/c");
    triple(
        &mut store,
        G,
        "http://ex/b1",
        RDFS_SUBPROPERTY_OF,
        "http://ex/b2",
    );

    let g = store.lookup(G).unwrap().unwrap();
    let report = materialise(&mut store, g, TS).unwrap();
    assert!(report.derived_anything());

    let c = closure_of(&mut store, G);
    assert!(
        has(&c, "http://ex/a", "http://ex/b1", "http://ex/c"),
        "premise kept"
    );
    assert!(
        has(&c, "http://ex/a", "http://ex/b2", "http://ex/c"),
        "rdfs7: the super-property triple must EXIST, not be rewritten into"
    );
}

#[test]
fn rdfs2_domain_types_the_subject() {
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", "http://ex/p", "http://ex/o");
    triple(&mut store, G, "http://ex/p", RDFS_DOMAIN, "http://ex/C");
    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);
    assert!(has(&c, "http://ex/a", RDF_TYPE, "http://ex/C"));
    assert!(
        !has(&c, "http://ex/o", RDF_TYPE, "http://ex/C"),
        "domain types the SUBJECT only"
    );
}

#[test]
fn rdfs3_range_types_the_object() {
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", "http://ex/p", "http://ex/o");
    triple(&mut store, G, "http://ex/p", RDFS_RANGE, "http://ex/C");
    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);
    assert!(has(&c, "http://ex/o", RDF_TYPE, "http://ex/C"));
    assert!(
        !has(&c, "http://ex/a", RDF_TYPE, "http://ex/C"),
        "range types the OBJECT only"
    );
}

#[test]
fn rdfs9_subclass_lifts_the_type() {
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", RDF_TYPE, "http://ex/C1");
    triple(
        &mut store,
        G,
        "http://ex/C1",
        RDFS_SUBCLASS_OF,
        "http://ex/C2",
    );
    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();
    assert!(has(
        &closure_of(&mut store, G),
        "http://ex/a",
        RDF_TYPE,
        "http://ex/C2"
    ));
}

#[test]
fn the_hierarchies_are_transitively_closed() {
    let mut store = Store::open_in_memory().unwrap();
    triple(
        &mut store,
        G,
        "http://ex/C1",
        RDFS_SUBCLASS_OF,
        "http://ex/C2",
    );
    triple(
        &mut store,
        G,
        "http://ex/C2",
        RDFS_SUBCLASS_OF,
        "http://ex/C3",
    );
    triple(
        &mut store,
        G,
        "http://ex/p1",
        RDFS_SUBPROPERTY_OF,
        "http://ex/p2",
    );
    triple(
        &mut store,
        G,
        "http://ex/p2",
        RDFS_SUBPROPERTY_OF,
        "http://ex/p3",
    );
    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);
    assert!(
        has(&c, "http://ex/C1", RDFS_SUBCLASS_OF, "http://ex/C3"),
        "rdfs11"
    );
    assert!(
        has(&c, "http://ex/p1", RDFS_SUBPROPERTY_OF, "http://ex/p3"),
        "rdfs5"
    );
}

#[test]
fn rules_compose_across_rounds_not_just_within_one() {
    // rdfs7 produces a triple whose predicate has a domain, which rdfs2 then
    // types, and rdfs9 then lifts. A single pass produces none of the last two.
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", "http://ex/b1", "http://ex/c");
    triple(
        &mut store,
        G,
        "http://ex/b1",
        RDFS_SUBPROPERTY_OF,
        "http://ex/b2",
    );
    triple(&mut store, G, "http://ex/b2", RDFS_DOMAIN, "http://ex/C1");
    triple(
        &mut store,
        G,
        "http://ex/C1",
        RDFS_SUBCLASS_OF,
        "http://ex/C2",
    );

    let g = store.lookup(G).unwrap().unwrap();
    let report = materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);
    assert!(
        has(&c, "http://ex/a", "http://ex/b2", "http://ex/c"),
        "rdfs7"
    );
    assert!(
        has(&c, "http://ex/a", RDF_TYPE, "http://ex/C1"),
        "rdfs2 over the DERIVED triple"
    );
    assert!(
        has(&c, "http://ex/a", RDF_TYPE, "http://ex/C2"),
        "rdfs9 over the DERIVED type"
    );
    assert!(
        report.rounds > 2,
        "a fixed point needs more than one pass, got {}",
        report.rounds
    );
}

#[test]
fn derivations_land_in_the_companion_and_never_beside_their_premises() {
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", RDF_TYPE, "http://ex/C1");
    triple(
        &mut store,
        G,
        "http://ex/C1",
        RDFS_SUBCLASS_OF,
        "http://ex/C2",
    );
    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();

    // The premise graph must be untouched: an overlay's closure is its own, so
    // it cannot leak into the graph it reasons over.
    let n: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE g = ?1 AND op = 1 AND valid_to IS NULL",
            [g],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        n, 2,
        "premise graph must still hold exactly its own 2 triples"
    );
}

#[test]
fn a_graph_with_no_schema_closes_to_itself_and_asserts_nothing() {
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", "http://ex/p", "http://ex/o");
    let g = store.lookup(G).unwrap().unwrap();
    let report = materialise(&mut store, g, TS).unwrap();
    // Not a failure: nothing was entailed because nothing entails anything.
    assert!(!report.derived_anything());
    assert_eq!(report.asserted, 0);
}

#[test]
fn re_running_the_closure_asserts_nothing_the_second_time() {
    let mut store = Store::open_in_memory().unwrap();
    triple(&mut store, G, "http://ex/a", "http://ex/b1", "http://ex/c");
    triple(
        &mut store,
        G,
        "http://ex/b1",
        RDFS_SUBPROPERTY_OF,
        "http://ex/b2",
    );
    let g = store.lookup(G).unwrap().unwrap();
    let first = materialise(&mut store, g, TS).unwrap();
    let second = materialise(&mut store, g, TS).unwrap();
    assert!(first.derived_anything());
    assert_eq!(
        second.asserted, 0,
        "premises include the companion, so a re-run is a no-op"
    );
}

/// rdfs2 must fire when the premise's object is a LITERAL (aegis-x9bmhf).
///
/// `rdfs:domain` on a datatype property is one of the commonest inferences in
/// real RDF, and it derived nothing: `load()` kept only `Value::Ref` objects, so
/// a literal-valued triple never entered the working set at all. The conclusion
/// here is all-IRI (`ex:a rdf:type ex:Person`), so only the PREMISE side needed
/// widening.
///
/// The IRI-object control in the same test is what makes a failure readable: if
/// both arms go to zero the closure is broken generally, and only the literal
/// arm failing localises it to this defect.
#[test]
fn rdfs2_domain_types_the_subject_of_a_literal_valued_premise() {
    let mut store = Store::open_in_memory().unwrap();
    literal_triple(&mut store, G, "http://example.org/a", "http://example.org/name", "n");
    triple(&mut store, G, "http://example.org/name", RDFS_DOMAIN, "http://example.org/Person");
    // CONTROL: an IRI-object premise through a DIFFERENT property, so the two
    // arms cannot mask each other.
    triple(&mut store, G, "http://example.org/c", "http://example.org/knows", "http://example.org/d");
    triple(&mut store, G, "http://example.org/knows", RDFS_DOMAIN, "http://example.org/Agent");

    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);

    assert!(
        has(&c, "http://example.org/c", RDF_TYPE, "http://example.org/Agent"),
        "CONTROL FAILED: rdfs2 does not fire even over an IRI object — the defect is \
         not literal-specific and this test cannot localise it"
    );
    assert!(
        has(&c, "http://example.org/a", RDF_TYPE, "http://example.org/Person"),
        "rdfs2 did not fire over a literal-valued premise"
    );
}

/// The derived type must then close under rdfs9, so a literal-valued premise is
/// a first-class citizen of the fixed point rather than a special case bolted on
/// after it.
#[test]
fn a_type_derived_from_a_literal_premise_closes_under_subclass() {
    let mut store = Store::open_in_memory().unwrap();
    literal_triple(&mut store, G, "http://example.org/a", "http://example.org/name", "n");
    triple(&mut store, G, "http://example.org/name", RDFS_DOMAIN, "http://example.org/Person");
    triple(&mut store, G, "http://example.org/Person", RDFS_SUBCLASS_OF, "http://example.org/Agent");

    let g = store.lookup(G).unwrap().unwrap();
    materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);
    assert!(has(&c, "http://example.org/a", RDF_TYPE, "http://example.org/Person"));
    assert!(
        has(&c, "http://example.org/a", RDF_TYPE, "http://example.org/Agent"),
        "rdfs9 must close a type that rdfs2 derived from a literal premise"
    );
}

/// rdfs3 must NOT fire over a literal object: its conclusion types the OBJECT,
/// and a literal cannot be the subject of an `rdf:type`.
///
/// This is the arm that keeps the fix honest. Widening the premise set for rdfs2
/// makes those triples visible to every rule in the same loop, and the obvious
/// wrong fix — feeding them through the existing body — would silently start
/// typing literals.
#[test]
fn rdfs3_still_does_not_type_a_literal_object() {
    let mut store = Store::open_in_memory().unwrap();
    literal_triple(&mut store, G, "http://example.org/a", "http://example.org/name", "n");
    triple(&mut store, G, "http://example.org/name", RDFS_RANGE, "http://example.org/Name");
    let g = store.lookup(G).unwrap().unwrap();
    let report = materialise(&mut store, g, TS).unwrap();
    let c = closure_of(&mut store, G);
    assert!(
        !c.iter().any(|(_, p, o)| p == RDF_TYPE && o == "http://example.org/Name"),
        "a literal object must never be typed by rdfs3"
    );
    assert_eq!(
        report.asserted, 0,
        "rdfs3 over a literal object must derive NOTHING, not an unreadable triple"
    );
}
