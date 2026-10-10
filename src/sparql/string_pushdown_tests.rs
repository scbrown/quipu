//! String FILTER pushdown never changes an answer (aegis-tl2q4j).
//!
//! Every query runs twice: as written, where the string test narrows the scan,
//! and as `FILTER((X) || false)`, which no pushdown touches. The rows must be
//! identical. The data carries the cases a narrowing could get wrong: IRIs,
//! blank nodes, language and typed literals, numbers, booleans, and
//! characters whose case mapping changes length or leaves ASCII.

use oxrdfio::RdfFormat;
use spargebra::SparqlParser;
use spargebra::algebra::GraphPattern;

use super::pattern_util::Bindings;
use super::query;
use super::string_pushdown::collect;
use crate::rdf::ingest_rdf;
use crate::store::Store;

const PREFIXES: &str = "PREFIX ex: <http://example.org/> \
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
    PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

fn store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
        @prefix ex: <http://example.org/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:quipu-1 rdfs:label "Quipu store" ; ex:status "open" ; ex:n 42 .
        ex:Quipu-2 rdfs:label "QUIPU"@en ; ex:statusCode "closed"^^xsd:string .
        ex:other rdfs:label "Kelvin K key" ; ex:ref ex:quipu-1 ; ex:flag true .
        ex:dotted rdfs:label "İstanbul" ; ex:when "2026-01-01"^^xsd:date .
        ex:sharp rdfs:label "straße" ; ex:n 4.2e1 .
        ex:search.svc rdfs:label "search.svc" ; rdfs:comment "aegis-9ka uses Dolt" .
        _:b rdfs:label "blank quipu" ; ex:ref ex:Quipu-2 .
    "#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-10-07T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

fn rows(store: &Store, sparql: &str) -> Vec<String> {
    let result = query(store, &format!("{PREFIXES}{sparql}")).unwrap();
    let mut out: Vec<String> = result
        .rows()
        .iter()
        .map(|row: &Bindings| {
            let mut kv: Vec<String> = row.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
            kv.sort();
            kv.join(" ")
        })
        .collect();
    out.sort();
    out
}

const SUBJECTS: &[&str] = &[
    "?x",
    "STR(?x)",
    "LCASE(STR(?x))",
    "UCASE(STR(?x))",
    "LCASE(?x)",
];
const NEEDLES: &[&str] = &[
    "\"quipu\"",
    "\"QUIPU\"",
    "\"k\"",
    "\"K\"",
    "\"i\u{307}\"",
    "\"SS\"",
    "\"4\"",
    "\"true\"",
    "\"2026\"",
    "\"svc\"",
    "\"http://example.org/q\"",
    "\"\"",
    "\"quipu\"@en",
];
const TESTS: &[&str] = &["CONTAINS", "STRSTARTS", "STRENDS", "REGEX"];
const BGPS: &[&str] = &["?x ?p ?o", "?s ?x ?o", "?s ?p ?x", "?s rdfs:label ?x"];

#[test]
fn pushdown_matches_the_unpushed_filter_everywhere() {
    let store = store();
    let mut checked = 0;
    for bgp in BGPS {
        for subject in SUBJECTS {
            for test in TESTS {
                for needle in NEEDLES {
                    let call = format!("{test}({subject}, {needle})");
                    let pushed = format!("SELECT * WHERE {{ {bgp} FILTER({call}) }}");
                    let plain = format!("SELECT * WHERE {{ {bgp} FILTER(({call}) || false) }}");
                    assert_eq!(rows(&store, &pushed), rows(&store, &plain), "{pushed}");
                    checked += 1;
                }
            }
        }
    }
    let flagged = "SELECT * WHERE { ?s rdfs:label ?x FILTER(REGEX(?x, \"^q\", \"i\")) }";
    let plain = "SELECT * WHERE { ?s rdfs:label ?x FILTER(REGEX(?x, \"^q\", \"i\") || false) }";
    assert_eq!(rows(&store, flagged), rows(&store, plain));
    assert!(
        !rows(&store, flagged).is_empty(),
        "control: the flagged regex matches"
    );
    assert_eq!(
        checked,
        BGPS.len() * SUBJECTS.len() * TESTS.len() * NEEDLES.len()
    );
}

#[test]
fn a_conjunction_narrows_by_each_side_and_keeps_the_answer() {
    let store = store();
    let pushed = "SELECT * WHERE { ?s rdfs:label ?l ; ?p ?o \
        FILTER(CONTAINS(LCASE(?l), \"quipu\") && STRSTARTS(STR(?s), \"http\")) }";
    let plain = "SELECT * WHERE { ?s rdfs:label ?l ; ?p ?o \
        FILTER((CONTAINS(LCASE(?l), \"quipu\") && STRSTARTS(STR(?s), \"http\")) || false) }";
    assert_eq!(rows(&store, pushed), rows(&store, plain));
    assert!(!rows(&store, pushed).is_empty(), "control: rows exist");
}

fn filter_of(sparql: &str) -> (spargebra::algebra::Expression, GraphPattern) {
    let parsed = SparqlParser::new()
        .parse_query(&format!("{PREFIXES}{sparql}"))
        .unwrap();
    let spargebra::Query::Select { pattern, .. } = parsed else {
        panic!("select");
    };
    let mut p = pattern;
    loop {
        match p {
            GraphPattern::Project { inner, .. } => p = *inner,
            GraphPattern::Filter { expr, inner } => return (expr, *inner),
            other => panic!("no filter: {other:?}"),
        }
    }
}

#[test]
fn only_licensed_shapes_narrow() {
    let narrows = |q: &str| {
        let (expr, inner) = filter_of(q);
        collect(&expr, &inner, &Bindings::new()).len()
    };
    assert_eq!(
        narrows("SELECT * WHERE { ?s ?p ?o FILTER(CONTAINS(STR(?s), \"x\")) }"),
        1
    );
    assert_eq!(
        narrows(
            "SELECT * WHERE { ?s ?p ?o FILTER(CONTAINS(LCASE(STR(?o)), \"x\") && REGEX(?p, \"y\")) }"
        ),
        2
    );
    // Under || or !, with a non-literal needle, an invalid regex, or a
    // variable the BGP does not mention: nothing is pushed.
    for q in [
        "SELECT * WHERE { ?s ?p ?o FILTER(CONTAINS(STR(?s), \"x\") || true) }",
        "SELECT * WHERE { ?s ?p ?o FILTER(!CONTAINS(STR(?s), \"x\")) }",
        "SELECT * WHERE { ?s ?p ?o FILTER(CONTAINS(STR(?s), STR(?o))) }",
        "SELECT * WHERE { ?s ?p ?o FILTER(REGEX(?o, \"(\")) }",
        "SELECT * WHERE { ?s ?p ?o FILTER(CONTAINS(STR(?z), \"x\")) }",
        "SELECT * WHERE { ?s ?p ?o FILTER(CONTAINS(SUBSTR(?o, 2), \"x\")) }",
    ] {
        assert_eq!(narrows(q), 0, "{q}");
    }
}
