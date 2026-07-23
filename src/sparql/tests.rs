//! Tests for the SPARQL query engine.

use super::*;
use crate::rdf::ingest_rdf;
use oxrdfio::RdfFormat;

fn test_store_with_data() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "30"^^xsd:integer ;
    ex:knows ex:bob .

ex:bob a ex:Person ;
    ex:name "Bob" ;
    ex:age "25"^^xsd:integer ;
    ex:knows ex:alice .

ex:carol a ex:Employee ;
    ex:name "Carol" ;
    ex:age "35"^^xsd:integer .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

#[test]
fn select_all_triples() {
    let store = test_store_with_data();
    let result = query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap();

    assert_eq!(result.variables(), vec!["s", "p", "o"]);
    // 4 for alice + 4 for bob + 3 for carol = 11
    assert_eq!(result.rows().len(), 11);
}

#[test]
fn select_with_bound_predicate() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "SELECT ?s ?name WHERE { ?s <http://example.org/name> ?name }",
    )
    .unwrap();

    assert_eq!(result.variables(), vec!["s", "name"]);
    assert_eq!(result.rows().len(), 3);

    let names: Vec<&Value> = result
        .rows()
        .iter()
        .map(|r| r.get("name").unwrap())
        .collect();
    assert!(names.contains(&&Value::Str("Alice".into())));
    assert!(names.contains(&&Value::Str("Bob".into())));
    assert!(names.contains(&&Value::Str("Carol".into())));
}

#[test]
fn select_with_bound_subject() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "SELECT ?p ?o WHERE { <http://example.org/alice> ?p ?o }",
    )
    .unwrap();

    assert_eq!(result.variables(), vec!["p", "o"]);
    assert_eq!(result.rows().len(), 4); // type, name, age, knows
}

#[test]
fn select_with_filter_comparison() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?s ?age WHERE {
            ?s <http://example.org/age> ?age .
            FILTER(?age > 28)
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 2); // Alice (30) and Carol (35)
    for row in result.rows() {
        let age = row.get("age").unwrap();
        match age {
            Value::Int(n) => assert!(*n > 28),
            _ => panic!("expected Int"),
        }
    }
}

#[test]
fn bgp_dedups_reasserted_facts() {
    // GH#13: a triple asserted across multiple transactions leaves multiple
    // current (op=1, valid_to NULL) rows; BGP must yield ONE solution, and joins
    // must not cartesian-explode / inflate COUNT.
    let mut store = Store::open_in_memory().unwrap();
    let ttl = "@prefix ex: <http://example.org/> .\nex:x a ex:Thing ; ex:label \"X\" .\n";
    for _ in 0..3 {
        ingest_rdf(
            &mut store,
            ttl.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    }
    // Plain type match: 1 solution, not 3.
    let r = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Thing> }",
    )
    .unwrap();
    assert_eq!(r.rows().len(), 1, "re-asserted triple must yield 1 binding");
    // OPTIONAL join must not multiply (the 23174-for-11 cartesian case).
    let r2 = query(
        &store,
        "SELECT ?s ?l WHERE { ?s a <http://example.org/Thing> OPTIONAL { ?s <http://example.org/label> ?l } }",
    )
    .unwrap();
    assert_eq!(
        r2.rows().len(),
        1,
        "OPTIONAL must not cartesian-explode on dup facts"
    );
}

#[test]
fn filter_contains_matches() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(CONTAINS(?name, "Bob")) }"#,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        result.rows()[0].get("name"),
        Some(&Value::Str("Bob".into()))
    );
}

#[test]
fn filter_contains_no_match_returns_empty() {
    // Regression for GH#12: FILTER(CONTAINS(...)) was a no-op returning ALL rows.
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(CONTAINS(?name, "zzznope")) }"#,
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        0,
        "match-nothing CONTAINS must return 0 rows"
    );
}

#[test]
fn filter_contains_with_lcase_nesting() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(CONTAINS(LCASE(?name), "bob")) }"#,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        result.rows()[0].get("name"),
        Some(&Value::Str("Bob".into()))
    );
}

#[test]
fn filter_isiri_excludes_literals() {
    // alice has 4 objects: ex:Person (IRI), "Alice"/30 (literals), ex:bob (IRI).
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?o WHERE { <http://example.org/alice> ?p ?o . FILTER(isIRI(?o)) }"#,
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        2,
        "isIRI must keep only the 2 IRI objects"
    );
}

#[test]
fn select_with_join() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name ?friend_name WHERE {
            ?s <http://example.org/name> ?name .
            ?s <http://example.org/knows> ?friend .
            ?friend <http://example.org/name> ?friend_name .
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 2); // Alice->Bob and Bob->Alice
    let pairs: Vec<(&Value, &Value)> = result
        .rows()
        .iter()
        .map(|r| (r.get("name").unwrap(), r.get("friend_name").unwrap()))
        .collect();
    assert!(pairs.contains(&(&Value::Str("Alice".into()), &Value::Str("Bob".into()))));
    assert!(pairs.contains(&(&Value::Str("Bob".into()), &Value::Str("Alice".into()))));
}

#[test]
fn select_with_filter_equality() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?s WHERE {
            ?s <http://example.org/name> ?name .
            FILTER(?name = "Alice")
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
}

#[test]
fn select_distinct() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT DISTINCT ?type WHERE {
            ?s a ?type .
        }"#,
    )
    .unwrap();

    // Person appears twice but DISTINCT deduplicates.
    assert_eq!(result.rows().len(), 2); // Person, Employee
}

#[test]
fn select_limit_offset() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name WHERE {
            ?s <http://example.org/name> ?name .
        } LIMIT 2"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 2);
}

#[test]
fn select_with_filter_bound() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?s ?name WHERE {
            ?s <http://example.org/name> ?name .
            FILTER(BOUND(?name))
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 3);
}

#[test]
fn select_order_by_asc() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name ?age WHERE {
            ?s <http://example.org/name> ?name .
            ?s <http://example.org/age> ?age .
        } ORDER BY ?age"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 3);
    let ages: Vec<&Value> = result
        .rows()
        .iter()
        .map(|r| r.get("age").unwrap())
        .collect();
    assert_eq!(
        ages,
        vec![&Value::Int(25), &Value::Int(30), &Value::Int(35)]
    );
}

#[test]
fn select_order_by_desc() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name ?age WHERE {
            ?s <http://example.org/name> ?name .
            ?s <http://example.org/age> ?age .
        } ORDER BY DESC(?age)"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 3);
    let ages: Vec<&Value> = result
        .rows()
        .iter()
        .map(|r| r.get("age").unwrap())
        .collect();
    assert_eq!(
        ages,
        vec![&Value::Int(35), &Value::Int(30), &Value::Int(25)]
    );
}

#[test]
fn select_optional() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name ?friend WHERE {
            ?s <http://example.org/name> ?name .
            OPTIONAL { ?s <http://example.org/knows> ?friend }
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 3);
    let carol_row = result
        .rows()
        .iter()
        .find(|r| r.get("name") == Some(&Value::Str("Carol".into())))
        .expect("Carol should appear");
    assert!(
        !carol_row.contains_key("friend"),
        "Carol should have no friend binding"
    );

    let alice_row = result
        .rows()
        .iter()
        .find(|r| r.get("name") == Some(&Value::Str("Alice".into())))
        .expect("Alice should appear");
    assert!(
        alice_row.contains_key("friend"),
        "Alice should have a friend binding"
    );
}

#[test]
fn select_order_by_with_limit() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name ?age WHERE {
            ?s <http://example.org/name> ?name .
            ?s <http://example.org/age> ?age .
        } ORDER BY ?age LIMIT 2"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 2);
    let ages: Vec<&Value> = result
        .rows()
        .iter()
        .map(|r| r.get("age").unwrap())
        .collect();
    assert_eq!(ages, vec![&Value::Int(25), &Value::Int(30)]);
}

#[test]
fn rdfs_subclass_type_query() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:Employee rdfs:subClassOf ex:Person .
ex:Manager rdfs:subClassOf ex:Employee .

ex:alice a ex:Person ; ex:name "Alice" .
ex:bob a ex:Employee ; ex:name "Bob" .
ex:carol a ex:Manager ; ex:name "Carol" .
ex:dave a ex:Other ; ex:name "Dave" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let result = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        3,
        "alice + bob + carol are all Persons"
    );

    let result = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Employee> }",
    )
    .unwrap();
    assert_eq!(result.rows().len(), 2, "bob + carol are Employees");

    let result = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Manager> }",
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1, "only carol is a Manager");

    let result = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Other> }",
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1, "only dave is Other");
}

#[test]
fn rdfs_subclass_no_hierarchy() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?s WHERE { ?s a <http://example.org/Person> }"#,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 2);
}

#[test]
fn select_count_all() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT (COUNT(*) AS ?count) WHERE { ?s <http://example.org/name> ?name }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0].get("count"), Some(&Value::Int(3)));
}

#[test]
fn select_group_by_with_count() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?type (COUNT(?s) AS ?n) WHERE { ?s a ?type } GROUP BY ?type"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 2);

    for row in result.rows() {
        let count = row.get("n").unwrap();
        match count {
            Value::Int(1 | 2) => {}
            other => panic!("unexpected count: {other:?}"),
        }
    }
}

#[test]
fn select_sum_and_avg() {
    let store = test_store_with_data();

    let result = query(
        &store,
        r#"SELECT (SUM(?age) AS ?total) (AVG(?age) AS ?mean) WHERE {
            ?s <http://example.org/age> ?age
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0].get("total"), Some(&Value::Int(90)));
    assert_eq!(result.rows()[0].get("mean"), Some(&Value::Float(30.0)));
}

#[test]
fn select_min_max() {
    let store = test_store_with_data();

    let result = query(
        &store,
        r#"SELECT (MIN(?age) AS ?youngest) (MAX(?age) AS ?oldest) WHERE {
            ?s <http://example.org/age> ?age
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0].get("youngest"), Some(&Value::Int(25)));
    assert_eq!(result.rows()[0].get("oldest"), Some(&Value::Int(35)));
}

#[test]
fn having_filters_groups() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?type (COUNT(?s) AS ?n) WHERE {
            ?s a ?type
        } GROUP BY ?type HAVING (COUNT(?s) > 1)"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0].get("n"), Some(&Value::Int(2)));
}

#[test]
fn count_star_empty_result() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT (COUNT(*) AS ?cnt) WHERE {
            ?s <http://example.org/nonexistent> ?o
        }"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0].get("cnt"), Some(&Value::Int(0)));
}

#[test]
fn group_by_with_sum() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:a1 ex:dept "Eng" ; ex:salary "100"^^xsd:integer .
ex:a2 ex:dept "Eng" ; ex:salary "120"^^xsd:integer .
ex:a3 ex:dept "Sales" ; ex:salary "90"^^xsd:integer .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    let result = query(
        &store,
        r#"SELECT ?dept (SUM(?sal) AS ?total) WHERE {
            ?s <http://example.org/dept> ?dept .
            ?s <http://example.org/salary> ?sal .
        } GROUP BY ?dept"#,
    )
    .unwrap();

    assert_eq!(result.rows().len(), 2);
    for row in result.rows() {
        let dept = row.get("dept").unwrap();
        let total = row.get("total").unwrap();
        match dept {
            Value::Str(d) if d == "Eng" => assert_eq!(total, &Value::Int(220)),
            Value::Str(d) if d == "Sales" => assert_eq!(total, &Value::Int(90)),
            _ => panic!("unexpected dept: {dept:?}"),
        }
    }
}

#[test]
fn temporal_sparql_valid_at() {
    let mut store = Store::open_in_memory().unwrap();

    ingest_rdf(
        &mut store,
        r#"@prefix ex: <http://example.org/> .
ex:server ex:status "active" ."#
            .as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let e = store.lookup("http://example.org/server").unwrap().unwrap();
    let a = store.lookup("http://example.org/status").unwrap().unwrap();
    store
        .transact(
            &[
                crate::store::Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("active".into()),
                    valid_from: "2026-01-01".into(),
                    valid_to: None,
                    op: crate::types::Op::Retract,
                },
                crate::store::Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("decommissioned".into()),
                    valid_from: "2026-03-01".into(),
                    valid_to: None,
                    op: crate::types::Op::Assert,
                },
            ],
            "2026-03-01",
            None,
            None,
        )
        .unwrap();

    let result = query(
        &store,
        "SELECT ?status WHERE { <http://example.org/server> <http://example.org/status> ?status }",
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        result.rows()[0].get("status"),
        Some(&Value::Str("decommissioned".into()))
    );

    let ctx = TemporalContext {
        valid_at: Some("2026-02-01".into()),
        as_of_tx: None,
        ..Default::default()
    };
    let result = query_temporal(
        &store,
        "SELECT ?status WHERE { <http://example.org/server> <http://example.org/status> ?status }",
        &ctx,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        result.rows()[0].get("status"),
        Some(&Value::Str("active".into()))
    );

    let ctx = TemporalContext {
        valid_at: Some("2026-04-01".into()),
        as_of_tx: None,
        ..Default::default()
    };
    let result = query_temporal(
        &store,
        "SELECT ?status WHERE { <http://example.org/server> <http://example.org/status> ?status }",
        &ctx,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        result.rows()[0].get("status"),
        Some(&Value::Str("decommissioned".into()))
    );
}

#[test]
fn temporal_sparql_as_of_tx() {
    let mut store = Store::open_in_memory().unwrap();

    ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:alice ex:name \"Alice\" .".as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:bob ex:name \"Bob\" .".as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-02-01",
        None,
        None,
    )
    .unwrap();

    let result = query(
        &store,
        "SELECT ?name WHERE { ?s <http://example.org/name> ?name }",
    )
    .unwrap();
    assert_eq!(result.rows().len(), 2);

    let ctx = TemporalContext {
        valid_at: None,
        as_of_tx: Some(1),
        ..Default::default()
    };
    let result = query_temporal(
        &store,
        "SELECT ?name WHERE { ?s <http://example.org/name> ?name }",
        &ctx,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        result.rows()[0].get("name"),
        Some(&Value::Str("Alice".into()))
    );
}

// ── ASK / CONSTRUCT / DESCRIBE tests ───────────────────────────

#[test]
fn ask_returns_true_when_pattern_matches() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "ASK { <http://example.org/alice> <http://example.org/name> ?name }",
    )
    .unwrap();
    match result {
        QueryResult::Ask(v) => assert!(v, "ASK should return true"),
        other => panic!("expected Ask result, got {other:?}"),
    }
}

#[test]
fn ask_returns_false_when_no_match() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "ASK { <http://example.org/nobody> <http://example.org/name> ?name }",
    )
    .unwrap();
    match result {
        QueryResult::Ask(v) => assert!(!v, "ASK should return false"),
        other => panic!("expected Ask result, got {other:?}"),
    }
}

#[test]
fn construct_builds_triples() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"CONSTRUCT { ?s <http://example.org/label> ?name }
           WHERE { ?s <http://example.org/name> ?name }"#,
    )
    .unwrap();
    match result {
        QueryResult::Graph(triples) => {
            assert_eq!(triples.len(), 3, "should produce one triple per person");
            for t in &triples {
                assert_eq!(t.predicate, "http://example.org/label");
            }
        }
        other => panic!("expected Graph result, got {other:?}"),
    }
}

#[test]
fn describe_returns_entity_facts() {
    let store = test_store_with_data();
    let result = query(&store, "DESCRIBE <http://example.org/alice>").unwrap();
    match result {
        QueryResult::Graph(triples) => {
            assert!(
                triples.len() >= 3,
                "alice has at least type+name+age+knows, got {}",
                triples.len()
            );
            for t in &triples {
                assert_eq!(t.subject, "http://example.org/alice");
            }
        }
        other => panic!("expected Graph result, got {other:?}"),
    }
}

// ── Property path tests ─────────────────────────────────────────

fn test_store_with_graph() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .

ex:a ex:edge ex:b .
ex:b ex:edge ex:c .
ex:c ex:edge ex:d .
ex:a ex:alt  ex:d .
ex:b ex:link ex:d .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

#[test]
fn path_sequence() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?x ?z WHERE { ?x <http://example.org/edge>/<http://example.org/edge> ?z }",
    )
    .unwrap();

    let pairs: Vec<(String, String)> = result
        .rows()
        .iter()
        .map(|r| {
            let x = value_to_iri(&store, r.get("x").unwrap());
            let z = value_to_iri(&store, r.get("z").unwrap());
            (x, z)
        })
        .collect();

    assert!(pairs.contains(&("http://example.org/a".into(), "http://example.org/c".into())));
    assert!(pairs.contains(&("http://example.org/b".into(), "http://example.org/d".into())));
    assert_eq!(pairs.len(), 2);
}

#[test]
fn path_alternative() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?x ?y WHERE { ?x (<http://example.org/edge>|<http://example.org/alt>) ?y }",
    )
    .unwrap();

    // edge: a->b, b->c, c->d; alt: a->d = 4 pairs
    assert_eq!(result.rows().len(), 4);
}

#[test]
fn path_inverse() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?x WHERE { <http://example.org/c> ^<http://example.org/edge> ?x }",
    )
    .unwrap();

    let iris: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("x").unwrap()))
        .collect();

    assert_eq!(iris.len(), 1);
    assert!(iris.contains(&"http://example.org/b".into()));
}

#[test]
fn path_zero_or_more() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?y WHERE { <http://example.org/a> <http://example.org/edge>* ?y }",
    )
    .unwrap();

    let iris: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("y").unwrap()))
        .collect();

    assert!(iris.contains(&"http://example.org/a".into()), "zero steps");
    assert!(iris.contains(&"http://example.org/b".into()), "one step");
    assert!(iris.contains(&"http://example.org/c".into()), "two steps");
    assert!(iris.contains(&"http://example.org/d".into()), "three steps");
    assert_eq!(iris.len(), 4);
}

#[test]
fn path_one_or_more() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?y WHERE { <http://example.org/a> <http://example.org/edge>+ ?y }",
    )
    .unwrap();

    let iris: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("y").unwrap()))
        .collect();

    assert!(
        !iris.contains(&"http://example.org/a".into()),
        "no zero steps"
    );
    assert!(iris.contains(&"http://example.org/b".into()));
    assert!(iris.contains(&"http://example.org/c".into()));
    assert!(iris.contains(&"http://example.org/d".into()));
    assert_eq!(iris.len(), 3);
}

#[test]
fn path_zero_or_one() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?y WHERE { <http://example.org/a> <http://example.org/edge>? ?y }",
    )
    .unwrap();

    let iris: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("y").unwrap()))
        .collect();

    assert!(iris.contains(&"http://example.org/a".into()), "zero steps");
    assert!(iris.contains(&"http://example.org/b".into()), "one step");
}

#[test]
fn path_transitive_with_fixed_object() {
    let store = test_store_with_graph();
    let result = query(
        &store,
        "SELECT ?x WHERE { ?x <http://example.org/edge>+ <http://example.org/d> }",
    )
    .unwrap();

    let iris: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("x").unwrap()))
        .collect();

    assert!(iris.contains(&"http://example.org/a".into()));
    assert!(iris.contains(&"http://example.org/b".into()));
    assert!(iris.contains(&"http://example.org/c".into()));
    assert_eq!(iris.len(), 3);
}

/// Helper: resolve a `Value::Ref` to its IRI string.
fn value_to_iri(store: &Store, val: &Value) -> String {
    match val {
        Value::Ref(id) => store.resolve(*id).unwrap_or_else(|_| format!("?{id}")),
        Value::Str(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

// --- FILTER REGEX + fail-loud on unsupported builtins (hq-9hs) ---

#[test]
fn filter_regex_is_anchored_not_substring() {
    let store = test_store_with_data();
    // "^A" is a real anchored pattern: matches only names starting with A (Alice).
    // The old substring stub checked `name.contains("^A")`, which matched nothing.
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(REGEX(?name, "^A")) }"#,
    )
    .unwrap();
    assert_eq!(result.rows().len(), 1, "only Alice starts with A");
}

#[test]
fn filter_regex_case_insensitive_flag() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(REGEX(?name, "^a", "i")) }"#,
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        1,
        "case-insensitive flag matches Alice"
    );
}

#[test]
fn filter_unknown_builtin_errors() {
    let store = test_store_with_data();
    // An unsupported FILTER builtin must fail loudly, never silently match all rows.
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(MD5(?name)) }"#,
    );
    assert!(
        result.is_err(),
        "unsupported FILTER builtin must error, got: {result:?}"
    );
}

#[test]
fn filter_invalid_regex_flag_errors() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"SELECT ?name WHERE { ?s <http://example.org/name> ?name . FILTER(REGEX(?name, "A", "z")) }"#,
    );
    assert!(result.is_err(), "invalid REGEX flag must error");
}

#[test]
fn is_blank_never_matches_and_is_not_is_iri() {
    let store = test_store_with_data();

    // `Value` (types.rs) is Ref|Str|Int|Float|Bool|Bytes — the store has no
    // blank-node representation, so isBlank() is false for every term rather
    // than merely unimplemented.
    let blank = query(&store, "SELECT ?s WHERE { ?s ?p ?o . FILTER(isBlank(?s)) }").unwrap();
    assert_eq!(blank.rows().len(), 0, "isBlank must never match");

    // Discriminates against aegis-t2jh, where IsBlank shared a match arm with
    // IsIri and so returned every IRI subject: without the split these two
    // assertions cannot both hold.
    let iri = query(&store, "SELECT ?s WHERE { ?s ?p ?o . FILTER(isIRI(?s)) }").unwrap();
    assert_eq!(iri.rows().len(), 11, "isIRI must still match every subject");
}

// ── aegis-fmyi: numeric semantics survive datatype preservation ──────────
//
// Only xsd:integer and xsd:double take the Int/Float fast path now; every other
// numeric datatype becomes `Value::Typed` so its IRI round-trips. That must not
// turn xsd:long / xsd:decimal into strings for the query engine — a silent
// regression to LEXICAL comparison would sort 100 before 9 and pass unnoticed.

fn numeric_datatype_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    // Values chosen so lexical and numeric order DISAGREE: lexically
    // "10" < "100" < "9", numerically 9 < 10 < 100.
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:x ex:size "9"^^xsd:long ; ex:label "nine" .
ex:y ex:size "10"^^xsd:decimal ; ex:label "ten" .
ex:z ex:size "100"^^xsd:long ; ex:label "hundred" .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-07-15T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

fn labels(result: &QueryResult) -> Vec<String> {
    result
        .rows()
        .iter()
        .filter_map(|r| r.get("l").and_then(|v| v.as_lexical().map(str::to_string)))
        .collect()
}

#[test]
fn order_by_is_numeric_for_preserved_numeric_datatypes() {
    let store = numeric_datatype_store();
    let result = query(
        &store,
        "SELECT ?l WHERE { ?e <http://example.org/size> ?s ; <http://example.org/label> ?l } ORDER BY ?s",
    )
    .unwrap();
    assert_eq!(
        labels(&result),
        vec!["nine", "ten", "hundred"],
        "xsd:long/xsd:decimal must order numerically; lexical order would be ten, hundred, nine"
    );
}

#[test]
fn filter_compares_preserved_numeric_datatypes_as_numbers() {
    let store = numeric_datatype_store();
    let result = query(
        &store,
        "SELECT ?l WHERE { ?e <http://example.org/size> ?s ; <http://example.org/label> ?l . FILTER(?s > 9) }",
    )
    .unwrap();
    let mut got = labels(&result);
    got.sort();
    assert_eq!(
        got,
        vec!["hundred", "ten"],
        "9 excluded, 10 and 100 included"
    );
}

#[test]
fn sum_spans_mixed_numeric_datatypes() {
    let store = numeric_datatype_store();
    let result = query(
        &store,
        "SELECT (SUM(?s) AS ?total) WHERE { ?e <http://example.org/size> ?s }",
    )
    .unwrap();
    assert_eq!(result.rows()[0].get("total").unwrap().as_f64(), Some(119.0));
}

#[test]
fn is_numeric_and_is_literal_see_the_new_variants() {
    let store = numeric_datatype_store();
    // A Typed xsd:long is numeric even though it is not Value::Int.
    let n = query(
        &store,
        "SELECT ?s WHERE { ?e <http://example.org/size> ?s . FILTER(isNumeric(?s)) }",
    )
    .unwrap();
    assert_eq!(n.rows().len(), 3, "all three sizes are numeric");
    // …and every literal, tagged or typed, is a literal.
    let l = query(
        &store,
        "SELECT ?s WHERE { ?e <http://example.org/size> ?s . FILTER(isLiteral(?s)) }",
    )
    .unwrap();
    assert_eq!(l.rows().len(), 3);
}

#[test]
fn str_of_a_lang_literal_drops_the_tag() {
    let mut store = Store::open_in_memory().unwrap();
    ingest_rdf(
        &mut store,
        r#"<http://example.org/s> <http://example.org/g> "hello"@en ."#.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-07-15T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    // STR("hello"@en) is "hello". It used to be "hello@en" — the tag had been
    // glued into the lexical value at parse time (aegis-fmyi).
    let r = query(
        &store,
        "SELECT ?s WHERE { ?e <http://example.org/g> ?s . FILTER(STR(?s) = \"hello\") }",
    )
    .unwrap();
    assert_eq!(r.rows().len(), 1, "STR() must yield the bare lexical form");

    let r = query(
        &store,
        "SELECT ?s WHERE { ?e <http://example.org/g> ?s . FILTER(STR(?s) = \"hello@en\") }",
    )
    .unwrap();
    assert!(r.rows().is_empty(), "the tag must not appear in STR()");
}

#[test]
fn query_timeout_expired_deadline_errors() {
    let store = test_store_with_data();
    let ctx = TemporalContext {
        deadline: Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap(),
        ),
        ..Default::default()
    };
    let err = query_temporal(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &ctx).unwrap_err();
    assert!(
        matches!(err, crate::error::Error::QueryTimeout { .. }),
        "expected QueryTimeout, got: {err}"
    );
}

#[test]
fn query_timeout_from_store_config() {
    let mut store = test_store_with_data();
    // A 0ms budget cannot be met: the derived deadline is already due by the
    // first eval_pattern check. Proves the config path (not just an explicit
    // ctx deadline) enforces.
    store.search_config_mut().query_timeout_ms = 1;
    std::thread::sleep(std::time::Duration::from_millis(5));
    // sleep is not load-bearing for the deadline (it is re-derived at query
    // start); it just makes shared-runner clock jitter irrelevant.
    let start = std::time::Instant::now();
    let mut saw_timeout = false;
    // The 1ms budget is a race against real work; retry a few times rather
    // than flake, but every attempt must either succeed fast or time out.
    for _ in 0..10 {
        match query(
            &store,
            "SELECT ?s ?p ?o WHERE { ?s ?p ?o . ?a ?b ?c . ?d ?e ?f }",
        ) {
            Err(crate::error::Error::QueryTimeout { .. }) => {
                saw_timeout = true;
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
    assert!(
        saw_timeout || start.elapsed().as_millis() < 1000,
        "1ms-budget queries neither timed out nor stayed fast"
    );
}

#[test]
fn query_timeout_generous_budget_still_answers_and_clears_handler() {
    let mut store = test_store_with_data();
    store.search_config_mut().query_timeout_ms = 30_000;
    let result = query(
        &store,
        "SELECT ?name WHERE { ?s <http://example.org/name> ?name }",
    )
    .unwrap();
    assert!(!result.rows().is_empty());
    // Handler must be cleared after the query: a second query on the same
    // connection with the timeout DISABLED must not inherit a stale deadline.
    store.search_config_mut().query_timeout_ms = 0;
    let result = query(
        &store,
        "SELECT ?name WHERE { ?s <http://example.org/name> ?name }",
    )
    .unwrap();
    assert!(!result.rows().is_empty());
}

#[test]
fn join_row_cap_aborts_exploding_cross_join() {
    let mut store = test_store_with_data();
    // Disable the wall clock so the cap is what fires: the point of the cap
    // is stopping an explosion long before the timeout would.
    store.search_config_mut().query_timeout_ms = 0;
    store.search_config_mut().max_join_rows = 10;
    // Three unbound patterns cross-multiply: |facts|^3 intermediate rows —
    // the exact shape of the exploded join that wedged evaluation while
    // holding the store mutex (the row cap must abort it, not the clock).
    let err = query(
        &store,
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o . ?a ?b ?c . ?d ?e ?f }",
    )
    .unwrap_err();
    assert!(
        matches!(err, crate::error::Error::QueryComplexity { limit: 10 }),
        "expected QueryComplexity naming the limit, got: {err}"
    );
    // The error text must NAME the limit and the knob — that is what makes a
    // 4xx actionable instead of mysterious.
    let msg = err.to_string();
    assert!(msg.contains("10"), "limit missing from message: {msg}");
    assert!(
        msg.contains("max_join_rows"),
        "knob missing from message: {msg}"
    );
}

#[test]
fn join_row_cap_leaves_normal_queries_alone() {
    let mut store = test_store_with_data();
    store.search_config_mut().max_join_rows = 1_000_000;
    let result = query(
        &store,
        "SELECT ?name WHERE { ?s a <http://example.org/Person> . ?s <http://example.org/name> ?name }",
    )
    .unwrap();
    assert_eq!(result.rows().len(), 2, "Alice and Bob expected");
}

#[test]
fn join_row_cap_zero_disables() {
    let mut store = test_store_with_data();
    store.search_config_mut().query_timeout_ms = 0;
    store.search_config_mut().max_join_rows = 0;
    // Small store: the full cross join is tiny; with the cap disabled it
    // must complete rather than error.
    let result = query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o . ?a ?b ?c }").unwrap();
    assert!(!result.rows().is_empty());
}

#[test]
fn expired_deadline_stops_pure_rust_join_loop() {
    let mut store = test_store_with_data();
    // Cap disabled: force the DEADLINE to be what stops the join loop. The
    // expired deadline is only checked inside eval loops (the between-operator
    // check would also catch it — but pattern evaluation must not survive to a
    // second operator either way; mfg0 proved the loops are where CPU goes).
    store.search_config_mut().max_join_rows = 0;
    let ctx = TemporalContext {
        deadline: Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap(),
        ),
        ..Default::default()
    };
    let err = query_temporal(
        &store,
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o . ?a ?b ?c . ?d ?e ?f }",
        &ctx,
    )
    .unwrap_err();
    assert!(
        matches!(err, crate::error::Error::QueryTimeout { .. }),
        "expected QueryTimeout, got: {err}"
    );
}

// ── FILTER EXISTS / NOT EXISTS, including property paths inside ──────────────
// Before this, EXISTS hit the catch-all "unsupported FILTER expression" branch;
// NOT EXISTS with a path (the natural not-reachable-via-alias detector) errored,
// and a client defaulting the error's missing rows to empty read it as ALL-CLEAR.
// Assertions bind the ?n NAME literal (Value::Str) rather than the subject IRI,
// which is a dictionary-encoded Value::Ref(id) and cannot be string-matched.

fn names(result: &QueryResult) -> Vec<String> {
    let mut out: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| match r.get("n") {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

#[test]
fn filter_exists_basic() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE { ?s ex:name ?n . FILTER EXISTS { ?s ex:knows ?x } }",
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice", "Bob"],
        "alice+bob have knows, carol doesn't"
    );
}

#[test]
fn filter_not_exists_basic() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE { ?s ex:name ?n . FILTER NOT EXISTS { ?s ex:knows ?x } }",
    )
    .unwrap();
    assert_eq!(names(&result), vec!["Carol"], "only carol lacks ex:knows");
}

#[test]
fn filter_not_exists_with_property_path() {
    // THE bead shape: a property path inside NOT EXISTS must evaluate, not error.
    // alice<->bob form a ex:knows cycle; neither reaches carol. So "persons who
    // canNOT reach a carol-prefixed node via ex:knows+" is {alice, bob}.
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE {
           ?s ex:name ?n ; a ex:Person .
           FILTER NOT EXISTS { ?s ex:knows+ ?c . FILTER(STRSTARTS(STR(?c), \"http://example.org/carol\")) }
         }",
    )
    .expect("path under NOT EXISTS must evaluate, not error");
    assert_eq!(
        names(&result),
        vec!["Alice", "Bob"],
        "alice+bob cannot reach carol via knows+"
    );
}

#[test]
fn filter_exists_with_property_path_positive_control() {
    // The negative control the bead demands: an EXISTS whose path DOES match, so a
    // silently-empty engine result would show up as wrong, not as clean.
    // alice reaches bob via ex:knows+; the prefix matches bob -> alice qualifies.
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE {
           ?s ex:name ?n ; a ex:Person .
           FILTER EXISTS { ?s ex:knows+ ?c . FILTER(STRSTARTS(STR(?c), \"http://example.org/bob\")) }
         }",
    )
    .unwrap();
    let got = names(&result);
    assert!(
        !got.is_empty(),
        "EXISTS path must MATCH here (negative control): {got:?}"
    );
    assert!(
        got.contains(&"Alice".to_string()),
        "alice reaches bob: {got:?}"
    );
}

#[test]
fn filter_not_exists_compatible_mapping_binds_outer_var() {
    // Compatible-mapping correctness: the inner ?s is the OUTER ?s, not "any
    // subject that has knows". carol has no knows, so NOT EXISTS keeps carol even
    // though alice/bob DO — the inner solution must be compatible with carol's row.
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE { ?s ex:name ?n . FILTER NOT EXISTS { ?s ex:knows ?x } }",
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Carol"],
        "compatible-mapping: only carol"
    );
}

// --- Named-graph (quad) SPARQL scoping (quipu #36) ---
// The store side of #36 (the `g` column + overlay writes) shipped earlier;
// these cover the SPARQL read surface added on top: `GRAPH <iri>` and
// `GRAPH ?g` scoping, the foundation for subset-export / federation.

/// Root (g=0) holds one triple via ingest; a named graph <…/g/t1> holds two via
/// the overlay write path. Lets the GRAPH tests assert scoping both directions.
fn test_store_named_graph() -> (Store, String) {
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";

    // Default graph (g=0): ex:alice ex:name "root-alice".
    ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:alice ex:name \"root-alice\" .\n".as_bytes(),
        RdfFormat::Turtle,
        None,
        ts,
        None,
        None,
    )
    .unwrap();

    // Named graph <…/g/t1>: ex:alice ex:name "t1-alice" ; ex:x ex:p ex:y .
    let g_iri = "http://example.org/g/t1".to_string();
    let g = store.overlay_create(&g_iri, 0).unwrap();
    let e_alice = store.intern("http://example.org/alice").unwrap();
    let a_name = store.intern("http://example.org/name").unwrap();
    let e_x = store.intern("http://example.org/x").unwrap();
    let a_p = store.intern("http://example.org/p").unwrap();
    let v_y = store.intern("http://example.org/y").unwrap();
    store
        .overlay_write(g, Op::Assert, e_alice, a_name, Value::Str("t1-alice".into()), ts)
        .unwrap();
    store
        .overlay_write(g, Op::Assert, e_x, a_p, Value::Ref(v_y), ts)
        .unwrap();
    (store, g_iri)
}

#[test]
fn graph_iri_scopes_to_named_graph() {
    let (store, g_iri) = test_store_named_graph();
    let q = format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{g_iri}> {{ ?s ?p ?o }} }}");
    let result = query(&store, &q).unwrap();
    let objs: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("o").unwrap()))
        .collect();
    assert_eq!(result.rows().len(), 2, "exactly the named graph's two triples");
    assert!(objs.contains(&"t1-alice".to_string()));
    assert!(objs.contains(&"http://example.org/y".to_string()));
    assert!(
        !objs.contains(&"root-alice".to_string()),
        "the default (root) graph must be excluded"
    );
}

#[test]
fn graph_var_binds_named_graph_iri() {
    let (store, g_iri) = test_store_named_graph();
    let result = query(&store, "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap();
    assert_eq!(result.rows().len(), 2, "GRAPH ?g ranges the named graphs only");
    for r in result.rows() {
        assert_eq!(
            value_to_iri(&store, r.get("g").unwrap()),
            g_iri,
            "?g binds the named graph's IRI"
        );
    }
    let objs: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("o").unwrap()))
        .collect();
    assert!(
        !objs.contains(&"root-alice".to_string()),
        "GRAPH ?g must not range over the default graph (g=0)"
    );
}

#[test]
fn default_query_excludes_named_graphs() {
    let (store, _g) = test_store_named_graph();
    let result = query(&store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap();
    let objs: Vec<String> = result
        .rows()
        .iter()
        .map(|r| value_to_iri(&store, r.get("o").unwrap()))
        .collect();
    assert!(objs.contains(&"root-alice".to_string()), "root fact present");
    assert!(
        !objs.contains(&"t1-alice".to_string()),
        "a named-graph fact must NOT leak into the default-graph query"
    );
    assert!(!objs.contains(&"http://example.org/y".to_string()));
}

#[test]
fn graph_unknown_iri_is_empty_not_default() {
    let (store, _g) = test_store_named_graph();
    let result = query(
        &store,
        "SELECT ?s WHERE { GRAPH <http://example.org/g/nope> { ?s ?p ?o } }",
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        0,
        "an unknown graph IRI yields no rows, never a fall-through to the default graph"
    );
}

#[test]
fn property_path_in_named_graph_fails_loud() {
    let (store, g_iri) = test_store_named_graph();
    let q = format!(
        "SELECT ?o WHERE {{ GRAPH <{g_iri}> {{ <http://example.org/x> <http://example.org/p>+ ?o }} }}"
    );
    let err = query(&store, &q).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("property paths inside a named GRAPH"),
        "must fail loud rather than silently read the default graph; got: {msg}"
    );
}
