//! `FILTER(?v = <iri>)` is pushed into the BGP it wraps, and ONLY where that
//! cannot change the answer (aegis-o3l46b).

use oxrdfio::RdfFormat;
use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};

use super::filter_pushdown::seed_iri_equalities;
use super::pattern_util::Bindings;
use super::query;
use crate::rdf::ingest_rdf;
use crate::store::Store;
use crate::types::Value;

const EX: &str = "http://example.org/";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let mut turtle = String::from(
        "@prefix ex: <http://example.org/> .
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
         ex:Sub rdfs:subClassOf ex:T .
         ex:a a ex:T ; ex:by ex:r .
         ex:b a ex:Sub ; ex:by ex:r .
         ex:c a ex:U ; ex:by ex:r .
         ex:n1 ex:val 1 .
         ex:n2 ex:val 1.0 .\n",
    );
    // Typed nodes that are NOT joined: what the unpushed `?f a ?t` scans.
    for i in 0..200 {
        turtle.push_str(&format!("ex:o{i} a ex:Other .\n"));
    }
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-02T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

fn subjects(store: &Store, sparql: &str) -> Vec<String> {
    let result = query(store, &format!("PREFIX ex: <{EX}> {sparql}")).unwrap();
    let mut out: Vec<String> = result
        .rows()
        .iter()
        .map(|r| match r.get("f") {
            Some(Value::Ref(id)) => store.resolve(*id).unwrap(),
            other => panic!("unexpected binding {other:?}"),
        })
        .collect();
    out.sort();
    out
}

fn var(n: &str) -> Box<Expression> {
    Box::new(Expression::Variable(Variable::new_unchecked(n)))
}
fn iri(n: &str) -> Box<Expression> {
    Box::new(Expression::NamedNode(NamedNode::new_unchecked(format!(
        "{EX}{n}"
    ))))
}
fn type_bgp() -> GraphPattern {
    GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::Variable(Variable::new_unchecked("f")),
            predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(RDF_TYPE)),
            object: TermPattern::Variable(Variable::new_unchecked("t")),
        }],
    }
}

#[test]
fn the_filtered_variable_is_seeded_with_the_iri() {
    // The mechanism itself: without this seed `?f a ?t` scans every type fact.
    let store = store();
    let t_id = store.lookup(&format!("{EX}T")).unwrap().unwrap();
    let eq = Expression::Equal(var("t"), iri("T"));
    let seed = seed_iri_equalities(&store, &eq, &type_bgp(), &Bindings::new()).unwrap();
    assert_eq!(
        seed.and_then(|s| s.get("t").cloned()),
        Some(Value::Ref(t_id))
    );
    // Mirrored operands and sameTerm push the same way.
    let same = Expression::SameTerm(iri("T"), var("t"));
    assert!(
        seed_iri_equalities(&store, &same, &type_bgp(), &Bindings::new())
            .unwrap()
            .is_some()
    );
}

#[test]
fn nothing_is_seeded_where_it_could_change_the_answer() {
    let store = store();
    let none = |e: &Expression, p: &GraphPattern, seed: &Bindings| {
        seed_iri_equalities(&store, e, p, seed).unwrap().is_none()
    };
    let empty = Bindings::new();
    // Under OR: either side may hold.
    let or = Expression::Or(
        Box::new(Expression::Equal(var("t"), iri("T"))),
        Box::new(Expression::Equal(var("t"), iri("U"))),
    );
    assert!(none(&or, &type_bgp(), &empty), "OR");
    // Negated.
    let not = Expression::Not(Box::new(Expression::Equal(var("t"), iri("T"))));
    assert!(none(&not, &type_bgp(), &empty), "NOT");
    // A variable the BGP never binds: the filter is false; a seed would make it true.
    assert!(
        none(&Expression::Equal(var("zz"), iri("T")), &type_bgp(), &empty),
        "absent var"
    );
    // Already bound by the caller.
    let mut bound = Bindings::new();
    bound.insert("t".into(), Value::Ref(-1));
    assert!(
        none(&Expression::Equal(var("t"), iri("T")), &type_bgp(), &bound),
        "bound"
    );
    // Not a plain BGP.
    let opt = GraphPattern::LeftJoin {
        left: Box::new(GraphPattern::Bgp { patterns: vec![] }),
        right: Box::new(type_bgp()),
        expression: None,
    };
    assert!(
        none(&Expression::Equal(var("t"), iri("T")), &opt, &empty),
        "OPTIONAL"
    );
}

#[test]
fn the_asserted_only_type_join_returns_only_direct_members() {
    // ex:b is a T only through ex:Sub. The variable-object form is the
    // asserted-only reader path, so pushing the filter must NOT start inferring.
    let store = store();
    let pushed = subjects(
        &store,
        "SELECT ?f WHERE { ?f a ?t . FILTER(?t = ex:T) . ?f ex:by ex:r }",
    );
    assert_eq!(pushed, vec![format!("{EX}a")]);
    // Same answer from a filter the pushdown cannot use (control: it is the
    // seed that changed, not the result).
    let unpushed = subjects(
        &store,
        "SELECT ?f WHERE { ?f a ?t . FILTER(STR(?t) = \"http://example.org/T\") . ?f ex:by ex:r }",
    );
    assert_eq!(pushed, unpushed);
}

#[test]
fn unpushed_shapes_keep_their_answers() {
    let store = store();
    let either = subjects(
        &store,
        "SELECT ?f WHERE { ?f a ?t . FILTER(?t = ex:T || ?t = ex:U) . ?f ex:by ex:r }",
    );
    assert_eq!(either, vec![format!("{EX}a"), format!("{EX}c")]);
    let absent = subjects(
        &store,
        "SELECT ?f WHERE { ?f ex:by ex:r . FILTER(?zz = ex:T) }",
    );
    assert!(absent.is_empty(), "{absent:?}");
    // Literals compare by VALUE: 1 = 1.0 holds, so both rows survive.
    let lit = query(
        &store,
        &format!("PREFIX ex: <{EX}> SELECT ?f WHERE {{ ?f ex:val ?v . FILTER(?v = 1) }}"),
    )
    .unwrap();
    assert_eq!(lit.rows().len(), 2);
}

/// Timing evidence, not a gate: `cargo test --release -- --ignored --nocapture
/// filter_pushdown_bench`. The `STR()` form is the same question the pushdown
/// cannot use, so it measures the old cost on the same data and build.
#[test]
#[ignore = "timing evidence; run by hand"]
fn filter_pushdown_bench() {
    let mut store = Store::open_in_memory().unwrap();
    let mut turtle = String::from("@prefix ex: <http://example.org/> .\n");
    for i in 0..200_000 {
        turtle.push_str(&format!("ex:o{i} a ex:Other{} .\n", i % 50));
    }
    for i in 0..27 {
        turtle.push_str(&format!(
            "ex:f{i} a ex:Firing ; ex:by ex:r ; ex:at \"{i}\" .\n"
        ));
    }
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-02T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let time = |q: &str| {
        let t = std::time::Instant::now();
        let n = query(&store, &format!("PREFIX ex: <{EX}> {q}"))
            .unwrap()
            .rows()
            .len();
        (t.elapsed(), n)
    };
    for _ in 0..3 {
        let pushed =
            time("SELECT ?f WHERE { ?f a ?t . FILTER(?t = ex:Firing) . ?f ex:by ex:r ; ex:at ?a }");
        let unpushed = time(
            "SELECT ?f WHERE { ?f a ?t . FILTER(STR(?t) = \"http://example.org/Firing\") . ?f ex:by ex:r ; ex:at ?a }",
        );
        assert_eq!(pushed.1, 27);
        assert_eq!(unpushed.1, 27);
        println!("pushed {:?}  unpushed {:?}", pushed.0, unpushed.0);
    }
}
