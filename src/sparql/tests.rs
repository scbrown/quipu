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
fn rdfs_subclass_type_query_is_explicit_property_path() {
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
        "SELECT ?s WHERE { ?s a/<http://www.w3.org/2000/01/rdf-schema#subClassOf>* <http://example.org/Person> }",
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        3,
        "the explicit path includes employee and manager instances"
    );

    let result = query(
        &store,
        "SELECT ?s WHERE { ?s a/<http://www.w3.org/2000/01/rdf-schema#subClassOf>* <http://example.org/Employee> }",
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
fn unbound_zero_or_more_keeps_identity_for_terms_without_path_edges() {
    let mut store = test_store_with_graph();
    let turtle = r#"
@prefix ex: <http://example.org/> .
ex:isolated ex:name "isolated" ; ex:owner ex:keeper .
"#;
    ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-04-04T00:00:01Z",
        None,
        None,
    )
    .unwrap();

    let result = query(
        &store,
        "SELECT ?canon ?owner WHERE { \
         ?entity <http://example.org/name> \"isolated\" . \
         ?entity (<http://example.org/edge>|^<http://example.org/edge>)* ?canon . \
         ?canon <http://example.org/owner> ?owner }",
    )
    .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(
        value_to_iri(&store, result.rows()[0].get("canon").unwrap()),
        "http://example.org/isolated"
    );
    assert_eq!(
        value_to_iri(&store, result.rows()[0].get("owner").unwrap()),
        "http://example.org/keeper"
    );
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
        // after_millis(0) is already expired: `passed` compares with >=.
        deadline: Some(crate::time::Deadline::after_millis(0)),
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
        // after_millis(0) is already expired: `passed` compares with >=.
        deadline: Some(crate::time::Deadline::after_millis(0)),
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
        .overlay_write(
            g,
            Op::Assert,
            e_alice,
            a_name,
            Value::Str("t1-alice".into()),
            ts,
        )
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
    assert_eq!(
        result.rows().len(),
        2,
        "exactly the named graph's two triples"
    );
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
    assert_eq!(
        result.rows().len(),
        2,
        "GRAPH ?g ranges the named graphs only"
    );
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
    assert!(
        objs.contains(&"root-alice".to_string()),
        "root fact present"
    );
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
fn property_path_in_named_graph_stays_inside_that_graph() {
    let (mut store, g_iri) = test_store_named_graph();
    let g = store.lookup(&g_iri).unwrap().unwrap();
    let y = store.lookup("http://example.org/y").unwrap().unwrap();
    let p = store.lookup("http://example.org/p").unwrap().unwrap();
    let z = store.intern("http://example.org/z").unwrap();
    store
        .overlay_write(
            g,
            crate::Op::Assert,
            y,
            p,
            Value::Ref(z),
            "2026-04-04T00:00:00Z",
        )
        .unwrap();
    let q = format!(
        "SELECT ?o WHERE {{ GRAPH <{g_iri}> {{ <http://example.org/x> <http://example.org/p>+ ?o }} }}"
    );
    let result = query(&store, &q).unwrap();
    let objects: Vec<String> = result
        .rows()
        .iter()
        .map(|row| value_to_iri(&store, row.get("o").unwrap()))
        .collect();
    assert!(
        objects.contains(&"http://example.org/y".to_string())
            && objects.contains(&"http://example.org/z".to_string()),
        "the closure must traverse both named-graph edges: {objects:?}"
    );
}

#[test]
fn property_path_under_graph_var_still_fails_loud() {
    let (store, _) = test_store_named_graph();
    let err = query(
        &store,
        "SELECT ?g ?o WHERE { GRAPH ?g { <http://example.org/x> <http://example.org/p>+ ?o } }",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("named-graphs.md §6.2"), "{err}");
}

#[test]
fn property_paths_do_not_cross_graphs_but_from_traverses_the_declared_merge() {
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";
    let p = store.intern("http://example.org/p").unwrap();
    let x = store.intern("http://example.org/x").unwrap();
    let y = store.intern("http://example.org/y").unwrap();
    let z = store.intern("http://example.org/z").unwrap();
    let root_only = store.intern("http://example.org/root-only").unwrap();
    let g1_iri = "http://example.org/g/path-a";
    let g2_iri = "http://example.org/g/path-b";
    let g1 = store.overlay_create(g1_iri, 0).unwrap();
    let g2 = store.overlay_create(g2_iri, 0).unwrap();
    store
        .overlay_write(g1, crate::Op::Assert, x, p, Value::Ref(y), ts)
        .unwrap();
    store
        .overlay_write(g2, crate::Op::Assert, y, p, Value::Ref(z), ts)
        .unwrap();
    store
        .transact(
            &[crate::Datum {
                entity: y,
                attribute: p,
                value: Value::Ref(root_only),
                valid_from: ts.to_string(),
                valid_to: None,
                op: crate::Op::Assert,
            }],
            ts,
            None,
            None,
        )
        .unwrap();

    let one_graph = query(
        &store,
        &format!(
            "SELECT ?o WHERE {{ GRAPH <{g1_iri}> {{ <http://example.org/x> <http://example.org/p>+ ?o }} }}"
        ),
    )
    .unwrap();
    let one_objects: Vec<String> = one_graph
        .rows()
        .iter()
        .map(|row| value_to_iri(&store, row.get("o").unwrap()))
        .collect();
    assert_eq!(
        one_objects,
        vec!["http://example.org/y"],
        "a path in one graph must not borrow its next edge from ROOT or a sibling"
    );

    let merged = query(
        &store,
        &format!(
            "SELECT ?o FROM <{g1_iri}> FROM <{g2_iri}> WHERE {{ <http://example.org/x> <http://example.org/p>+ ?o }}"
        ),
    )
    .unwrap();
    let merged_objects: Vec<String> = merged
        .rows()
        .iter()
        .map(|row| value_to_iri(&store, row.get("o").unwrap()))
        .collect();
    assert!(
        merged_objects.contains(&"http://example.org/y".to_string())
            && merged_objects.contains(&"http://example.org/z".to_string()),
        "FROM must traverse the RDF merge: {merged_objects:?}"
    );
    assert!(
        !merged_objects.contains(&"http://example.org/root-only".to_string()),
        "ROOT is not part of the declared FROM merge: {merged_objects:?}"
    );
}

// --- FROM / FROM NAMED dataset selection + the `graph` query param (quipu #36) ---

/// Root (g=0) + two named graphs t1, t2 — each carrying one `ex:p` fact with a
/// distinct object, so dataset-selection tests can tell the graphs apart.
fn test_store_two_graphs() -> (Store, String, String) {
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";
    ingest_rdf(
        &mut store,
        "@prefix ex: <http://example.org/> .\nex:root ex:p \"R\" .\n".as_bytes(),
        RdfFormat::Turtle,
        None,
        ts,
        None,
        None,
    )
    .unwrap();
    let p = store.intern("http://example.org/p").unwrap();
    let g1_iri = "http://example.org/g/t1".to_string();
    let g1 = store.overlay_create(&g1_iri, 0).unwrap();
    let a = store.intern("http://example.org/a").unwrap();
    store
        .overlay_write(g1, Op::Assert, a, p, Value::Str("T1".into()), ts)
        .unwrap();
    let g2_iri = "http://example.org/g/t2".to_string();
    let g2 = store.overlay_create(&g2_iri, 0).unwrap();
    let b = store.intern("http://example.org/b").unwrap();
    store
        .overlay_write(g2, Op::Assert, b, p, Value::Str("T2".into()), ts)
        .unwrap();
    (store, g1_iri, g2_iri)
}

fn objs_of(store: &Store, r: &QueryResult) -> Vec<String> {
    let mut v: Vec<String> = r
        .rows()
        .iter()
        .map(|row| value_to_iri(store, row.get("o").unwrap()))
        .collect();
    v.sort();
    v
}

#[test]
fn from_makes_named_graph_the_default() {
    let (store, g1, _g2) = test_store_two_graphs();
    let q = format!("SELECT ?o FROM <{g1}> WHERE {{ ?s <http://example.org/p> ?o }}");
    assert_eq!(
        objs_of(&store, &query(&store, &q).unwrap()),
        vec!["T1".to_string()],
        "FROM <g1> makes g1 the default graph; ROOT is excluded"
    );
}

#[test]
fn from_union_merges_graphs() {
    let (store, g1, g2) = test_store_two_graphs();
    let q = format!("SELECT ?o FROM <{g1}> FROM <{g2}> WHERE {{ ?s <http://example.org/p> ?o }}");
    assert_eq!(
        objs_of(&store, &query(&store, &q).unwrap()),
        vec!["T1".to_string(), "T2".to_string()],
        "FROM union is the merge of g1 and g2; ROOT still excluded"
    );
}

#[test]
fn from_unknown_graph_is_empty_default() {
    let (store, _g1, _g2) = test_store_two_graphs();
    let q = "SELECT ?o FROM <http://example.org/g/nope> WHERE { ?s <http://example.org/p> ?o }";
    assert!(
        query(&store, q).unwrap().rows().is_empty(),
        "an all-unknown FROM set is an empty default graph, not a ROOT fall-through"
    );
}

#[test]
fn from_named_restricts_graph_var() {
    let (store, g1, _g2) = test_store_two_graphs();
    let q = format!(
        "SELECT ?g ?o FROM NAMED <{g1}> WHERE {{ GRAPH ?g {{ ?s <http://example.org/p> ?o }} }}"
    );
    let r = query(&store, &q).unwrap();
    assert_eq!(
        objs_of(&store, &r),
        vec!["T1".to_string()],
        "FROM NAMED restricts GRAPH ?g to g1 (t2 excluded)"
    );
    for row in r.rows() {
        assert_eq!(value_to_iri(&store, row.get("g").unwrap()), g1);
    }
}

#[test]
fn from_without_from_named_deactivates_named_graphs() {
    let (store, g1, _g2) = test_store_two_graphs();
    let q =
        format!("SELECT ?g ?o FROM <{g1}> WHERE {{ GRAPH ?g {{ ?s <http://example.org/p> ?o }} }}");
    assert!(
        query(&store, &q).unwrap().rows().is_empty(),
        "a dataset with FROM but no FROM NAMED activates no named graphs; GRAPH matches nothing"
    );
}

#[test]
fn graph_query_param_scopes_default_graph() {
    // End-to-end through the REST/MCP entry point: {query, graph:<iri>}.
    let (store, g1, _g2) = test_store_two_graphs();
    let input = serde_json::json!({
        "query": "SELECT ?o WHERE { ?s <http://example.org/p> ?o }",
        "graph": g1,
    });
    let (result, _truncated) = crate::mcp::query_result(&store, &input).unwrap();
    assert_eq!(
        objs_of(&store, &result),
        vec!["T1".to_string()],
        "the `graph` param scopes the default graph to g1 without a FROM/GRAPH clause"
    );
}

#[test]
fn graph_query_param_unknown_iri_is_empty() {
    let (store, _g1, _g2) = test_store_two_graphs();
    let input = serde_json::json!({
        "query": "SELECT ?o WHERE { ?s <http://example.org/p> ?o }",
        "graph": "http://example.org/g/nope",
    });
    let (result, _truncated) = crate::mcp::query_result(&store, &input).unwrap();
    assert!(
        result.rows().is_empty(),
        "an unknown graph param yields an empty default graph, never a ROOT fall-through"
    );
}

// ---------------------------------------------------------------------------
// VALUES — inline relations (quipu #51)
// ---------------------------------------------------------------------------

#[test]
fn values_single_variable_constrains() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             VALUES ?n { "Alice" "Carol" }
             ?s ex:name ?n
           }"#,
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice", "Carol"],
        "VALUES restricts to the listed candidates"
    );
}

#[test]
fn values_candidate_matching_nothing_yields_no_rows() {
    // The no-op check (cf. #12): a VALUES that names only absent values must
    // return ZERO rows, never fall through to every row.
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             VALUES ?n { "Nobody" }
             ?s ex:name ?n
           }"#,
    )
    .unwrap();
    assert!(result.rows().is_empty(), "no name matches \"Nobody\"");
}

#[test]
fn values_joins_with_bgp_in_either_order() {
    let store = test_store_with_data();
    let before = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE { VALUES ?n { "Bob" } ?s ex:name ?n }"#,
    )
    .unwrap();
    let after = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE { ?s ex:name ?n VALUES ?n { "Bob" } }"#,
    )
    .unwrap();
    assert_eq!(names(&before), vec!["Bob"]);
    assert_eq!(
        names(&before),
        names(&after),
        "VALUES before and after the BGP are the same join"
    );
}

#[test]
fn values_binds_iris() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE {
           VALUES ?s { ex:alice ex:carol }
           ?s ex:name ?n
         }",
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice", "Carol"],
        "an IRI in VALUES resolves to the interned term and joins on the subject"
    );
}

#[test]
fn values_unknown_iri_yields_no_rows() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE {
           VALUES ?s { ex:nobody }
           ?s ex:name ?n
         }",
    )
    .unwrap();
    assert!(
        result.rows().is_empty(),
        "an IRI absent from the dictionary matches nothing"
    );
}

#[test]
fn values_multi_variable_binds_both() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             VALUES (?s ?n) { (ex:alice "Alice") (ex:bob "Carol") }
             ?s ex:name ?n
           }"#,
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice"],
        "both columns must agree: (ex:bob, \"Carol\") is not a fact"
    );
}

#[test]
fn values_undef_leaves_variable_unbound() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             VALUES (?s ?n) { (ex:alice UNDEF) }
             ?s ex:name ?n
           }"#,
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice"],
        "UNDEF leaves ?n free, so the BGP binds it rather than the row erroring"
    );
}

#[test]
fn values_undef_is_not_bound() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?x WHERE {
           VALUES ?x { UNDEF }
           FILTER(!BOUND(?x))
         }",
    )
    .unwrap();
    assert_eq!(
        result.rows().len(),
        1,
        "UNDEF is an unbound variable, not a sentinel value"
    );
}

#[test]
fn values_empty_table_yields_no_rows() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             VALUES ?n { }
             ?s ex:name ?n
           }"#,
    )
    .unwrap();
    assert!(
        result.rows().is_empty(),
        "an empty VALUES is the empty relation — zero rows, not every row"
    );
}

// ---------------------------------------------------------------------------
// FILTER IN / NOT IN (quipu #52)
// ---------------------------------------------------------------------------

#[test]
fn filter_in_constrains() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             ?s ex:name ?n
             FILTER(?n IN ("Alice", "Bob"))
           }"#,
    )
    .unwrap();
    assert_eq!(names(&result), vec!["Alice", "Bob"]);
}

#[test]
fn filter_in_matching_nothing_yields_no_rows() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             ?s ex:name ?n
             FILTER(?n IN ("Nobody", "Nothing"))
           }"#,
    )
    .unwrap();
    assert!(
        result.rows().is_empty(),
        "IN must constrain, never pass everything through"
    );
}

#[test]
fn filter_not_in_excludes() {
    let store = test_store_with_data();
    let result = query(
        &store,
        r#"PREFIX ex: <http://example.org/>
           SELECT ?n WHERE {
             ?s ex:name ?n
             FILTER(?n NOT IN ("Alice", "Bob"))
           }"#,
    )
    .unwrap();
    assert_eq!(names(&result), vec!["Carol"]);
}

#[test]
fn filter_in_empty_list_is_false() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE { ?s ex:name ?n FILTER(?n IN ()) }",
    )
    .unwrap();
    assert!(result.rows().is_empty(), "IN () is false for every row");
}

#[test]
fn filter_not_in_empty_list_is_true() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE { ?s ex:name ?n FILTER(?n NOT IN ()) }",
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice", "Bob", "Carol"],
        "NOT IN () is true for every row"
    );
}

#[test]
fn filter_in_over_iris() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE {
           ?s ex:name ?n
           FILTER(?s IN (ex:alice, ex:carol))
         }",
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Alice", "Carol"],
        "IN works over IRIs, not only strings"
    );
}

#[test]
fn filter_in_over_numeric_literals() {
    let store = test_store_with_data();
    let result = query(
        &store,
        "PREFIX ex: <http://example.org/>
         SELECT ?n WHERE {
           ?s ex:name ?n .
           ?s ex:age ?age
           FILTER(?age IN (25, 35))
         }",
    )
    .unwrap();
    assert_eq!(
        names(&result),
        vec!["Bob", "Carol"],
        "IN works over numeric literals"
    );
}

// --- the type-inference asymmetry, announced --------------------------------

/// The fixture the whole defect lives on: a type WITH subclasses and a leaf type
/// without, so the marker can be shown to fire on one and stay silent on the other.
fn inference_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

ex:WebApplication rdfs:subClassOf ex:Service .
ex:SearchService rdfs:subClassOf ex:Service .

ex:svc1 a ex:Service .
ex:svc2 a ex:WebApplication .
ex:svc3 a ex:SearchService .
ex:t1 a ex:Tool .
ex:t2 a ex:Tool .
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
fn constant_type_is_inferred_while_variable_filter_is_asserted_only() {
    let store = inference_store();
    let inferred = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ex:Service }"}),
    )
    .unwrap();
    let asserted = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ?t . FILTER(?t = ex:Service) }"}),
    )
    .unwrap();
    assert_eq!(
        inferred["rows"][0]["n"], 3,
        "constant form includes subclasses"
    );
    assert_eq!(
        asserted["rows"][0]["n"], 1,
        "variable form is asserted-only"
    );
    assert_ne!(inferred["rows"][0]["n"], asserted["rows"][0]["n"]);
}

#[test]
fn applied_inference_is_announced_when_subclasses_expand_the_answer() {
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(out["rows"][0]["n"], 3);
    assert_eq!(out["inference"]["applied"], true);
    let expanded = &out["inference"]["expandedTypes"][0];
    assert_eq!(expanded["type"], "http://example.org/Service");
    let subs: Vec<String> = expanded["subclasses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(subs.contains(&"http://example.org/WebApplication".to_string()));
    assert!(subs.contains(&"http://example.org/SearchService".to_string()));
}

#[test]
fn joined_type_query_keeps_subclass_entailment() {
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { \
               ?s a ex:Service . ?s a ?assertedType \
             }"}),
    )
    .unwrap();
    assert_eq!(out["rows"][0]["n"], 3);
    assert_eq!(out["inference"]["applied"], true);
}

#[test]
fn the_marker_is_absent_when_the_flip_changed_nothing() {
    // Presence remains the signal: variable, leaf, and unrelated queries have
    // the same answer before and after the migration.
    let store = inference_store();
    for q in [
        "PREFIX ex: <http://example.org/> \
         SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ?t . FILTER(?t = ex:Service) }",
        "PREFIX ex: <http://example.org/> \
         SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ex:Tool }",
        "SELECT ?s WHERE { ?s ?p ?o } LIMIT 1",
    ] {
        let out = crate::mcp::tool_query(&store, &serde_json::json!({ "query": q })).unwrap();
        assert!(
            out.get("inference").is_none(),
            "marker must be ABSENT for a non-inferred answer, got {out} for {q}"
        );
    }
}

#[test]
fn a_leaf_type_is_the_control_that_proves_the_marker_is_specific() {
    let store = inference_store();
    let inferred = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ex:Tool }"}),
    )
    .unwrap();
    let asserted = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ?t . FILTER(?t = ex:Tool) }"}),
    )
    .unwrap();
    assert_eq!(inferred["rows"][0]["n"], asserted["rows"][0]["n"]);
    assert!(inferred.get("inference").is_none());
}

#[test]
fn ask_carries_the_applied_marker_too() {
    // ASK is a boolean, so the count-changing semantic flip is especially easy
    // to miss without the marker. svc2 is only WebApplication, not Service.
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> ASK { ex:svc2 a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(out["result"], true);
    assert_eq!(out["inference"]["applied"], true);
    assert_eq!(
        out["inference"]["expandedTypes"][0]["type"],
        "http://example.org/Service"
    );
}

#[test]
fn an_asserted_ask_and_a_leaf_ask_mark_only_the_changed_form() {
    let store = inference_store();
    for q in [
        "PREFIX ex: <http://example.org/> ASK { ex:t1 a ex:Tool }",
        "PREFIX ex: <http://example.org/> ASK { ex:svc2 a ?t . FILTER(?t = ex:WebApplication) }",
    ] {
        let out = crate::mcp::tool_query(&store, &serde_json::json!({ "query": q })).unwrap();
        assert_eq!(out["result"], true, "control must still be true for {q}");
        assert!(
            out.get("inference").is_none(),
            "marker must be ABSENT on an asserted-only ASK, got {out} for {q}"
        );
    }
}

#[test]
fn the_marker_reports_applied_expansion_even_when_the_boolean_stays_true() {
    // svc1 is directly asserted Service, so the value does not change, but the
    // query form did change and must still announce that fact.
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> ASK { ex:svc1 a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(out["result"], true);
    assert_eq!(out["inference"]["applied"], true);
}

#[test]
fn a_false_ask_still_says_inference_was_applied() {
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> ASK { ex:t1 a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(out["result"], false);
    assert_eq!(out["inference"]["applied"], true);
}

#[test]
fn construct_carries_the_applied_marker_too() {
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             CONSTRUCT { ?s a ex:Service } WHERE { ?s a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(out["count"], 3);
    assert_eq!(out["inference"]["applied"], true);
}

#[test]
fn the_note_explains_the_explicit_inferred_form() {
    let store = inference_store();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> ASK { ex:svc2 a ex:Service }"}),
    )
    .unwrap();
    let note = out["inference"]["note"].as_str().unwrap();
    assert!(note.contains("asserted-only"), "got: {note}");
    assert!(note.contains("subclass expansion"), "got: {note}");
}

#[test]
fn the_w3c_shapes_get_the_applied_marker_as_a_header_value() {
    // `Accept: application/sparql-results+json` reopens the whole defect: the
    // spec body has nowhere to put the marker, so on that path it must ride a
    // header or be silently dropped. Presence is still the signal, and the
    // widened parents are still NAMED.
    let store = inference_store();
    let inferred = crate::mcp::query_inference(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> ASK { ex:svc2 a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(
        crate::mcp::inference_header(&inferred).as_deref(),
        Some("applied: http://example.org/Service")
    );

    // Control: the header is absent exactly when the JSON field would be.
    let leaf = crate::mcp::query_inference(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> ASK { ex:t1 a ex:Tool }"}),
    )
    .unwrap();
    assert!(crate::mcp::inference_header(&leaf).is_none());
}

#[test]
fn the_explicit_path_form_preserves_the_inferred_question() {
    let store = inference_store();
    let path = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a/rdfs:subClassOf* ex:Service }"}),
    )
    .unwrap();
    assert_eq!(path["rows"][0]["n"], 3);
    let constant = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query":
            "PREFIX ex: <http://example.org/> \
             SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ex:Service }"}),
    )
    .unwrap();
    assert_eq!(constant["rows"][0]["n"], 3);
}

/// Canonical rendering of a `QueryResult` for equality assertions.
///
/// `Bindings` is a `HashMap`, so `{:?}` on a row varies by key iteration order
/// and is NOT a stable serialization — comparing debug strings reports a
/// difference where there is none. (Caught exactly that way while writing the
/// byte-identical test below: same rows, same order, different key order.)
/// Sort each row's keys; keep ROW order, which is semantically meaningful.
fn canonical(result: &QueryResult) -> String {
    match result {
        QueryResult::Select { variables, rows } => {
            let body: Vec<String> = rows
                .iter()
                .map(|r| {
                    let mut kv: Vec<String> = r.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
                    kv.sort();
                    format!("{{{}}}", kv.join(","))
                })
                .collect();
            format!("Select vars={variables:?} rows=[{}]", body.join(","))
        }
        other => format!("{other:?}"),
    }
}

// --- quipu #67: dataset labels on the query path ---

/// Two named graphs, one fresh and one stale, plus a ROOT triple.
fn labeled_store() -> (Store, String, String) {
    use crate::lattice::Freshness;
    use crate::store::labels::GraphLabel;
    use crate::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-08-06T00:00:00Z";
    let (a, b) = ("urn:g:fresh".to_string(), "urn:g:stale".to_string());
    let e = store.intern("http://example.org/s").unwrap();
    let p = store.intern("http://example.org/p").unwrap();

    for (iri, f) in [(&a, Freshness::Fresh), (&b, Freshness::Stale)] {
        let g = store.overlay_create(iri, 0).unwrap();
        store
            .overlay_write(g, Op::Assert, e, p, Value::Str(iri.clone()), ts)
            .unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    freshness: Some(f),
                    ..Default::default()
                },
                ts,
                None,
            )
            .unwrap();
    }
    (store, a, b)
}

#[test]
fn from_two_graphs_reports_the_weaker_freshness() {
    // #67 acceptance: FROM a b with fresh+stale -> labels report stale.
    let (store, a, b) = labeled_store();
    let q = format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}");
    let out = query_labeled(&store, &q, &TemporalContext::default()).unwrap();

    let labels = out.labels.expect("both members declared freshness");
    assert_eq!(
        labels.freshness.value,
        Some(crate::lattice::Freshness::Stale)
    );
    assert_eq!(labels.freshness.coverage, crate::lattice::Coverage::Full);
    assert_eq!(out.result.rows().len(), 2, "and the rows are unaffected");
}

#[test]
fn an_unlabelled_dataset_reports_no_labels_and_an_identical_payload() {
    // #67 acceptance, the important half: the QueryResult payload must be
    // byte-identical to what the un-labelled entry point returns.
    let store = test_store_with_data();
    let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";

    let plain = query(&store, q).unwrap();
    let labeled = query_labeled(&store, q, &TemporalContext::default()).unwrap();

    assert!(
        labeled.labels.is_none(),
        "nothing declared -> null, never a fabricated label"
    );
    // Compare the rendered payload, which is what a client actually receives.
    assert_eq!(
        canonical(&labeled.result),
        canonical(&plain),
        "the result payload must not change when labels are requested"
    );
}

#[test]
fn labels_do_not_change_the_rows_a_query_returns() {
    // A conservative dataset label must not be implemented by filtering.
    let (store, a, b) = labeled_store();
    let q = format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}");
    let plain = query(&store, &q).unwrap();
    let labeled = query_labeled(&store, &q, &TemporalContext::default()).unwrap();
    assert_eq!(canonical(&labeled.result), canonical(&plain));
}

#[test]
fn the_root_only_path_is_unlabelled_and_untouched() {
    let store = test_store_with_data();
    let out = query_labeled(
        &store,
        "SELECT ?s WHERE { ?s ?p ?o }",
        &TemporalContext::default(),
    )
    .unwrap();
    assert!(
        out.labels.is_none(),
        "ROOT alone declares nothing by default"
    );
}

#[test]
fn the_query_json_carries_a_labels_key() {
    // #67: /query and quipu_query gain a top-level "labels" key beside
    // `truncated`. Always present; null when undeclared.
    let (store, a, b) = labeled_store();
    let input = serde_json::json!({
        "query": format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}")
    });
    let out = crate::mcp::tool_query(&store, &input).unwrap();
    let labels = out.get("labels").expect("the key is always present");
    assert_eq!(
        labels["freshness"]["value"], "stale",
        "the weaker member wins"
    );
    assert_eq!(labels["freshness"]["coverage"], "full");
}

#[test]
fn an_undeclared_dataset_serializes_labels_as_null() {
    let store = test_store_with_data();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query": "SELECT ?s WHERE { ?s ?p ?o }"}),
    )
    .unwrap();
    assert!(
        out.get("labels").is_some_and(serde_json::Value::is_null),
        "present and null — a reader must tell 'undeclared' from 'this server has no labels'"
    );
    // And the rest of the payload is untouched.
    assert!(out.get("rows").is_some() && out.get("truncated").is_some());
}

#[test]
fn a_cross_chain_dataset_reports_a_label_error_without_failing_the_query() {
    // The regression this design avoids: `labels` is attached to EVERY
    // response, so a refusal must not take the query down with it. Callers who
    // never asked about labels must keep getting their rows.
    use crate::lattice::Trust;
    use crate::store::labels::GraphLabel;
    use crate::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-08-06T00:00:00Z";
    let e = store.intern("http://example.org/s").unwrap();
    let p = store.intern("http://example.org/p").unwrap();
    for (iri, chain) in [("urn:g:x1", "urn:chain:a"), ("urn:g:x2", "urn:chain:b")] {
        let g = store.overlay_create(iri, 0).unwrap();
        store
            .overlay_write(g, Op::Assert, e, p, Value::Str(iri.into()), ts)
            .unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    trust: Some(Trust::new(format!("{iri}#t"), chain, 10)),
                    ..Default::default()
                },
                ts,
                None,
            )
            .unwrap();
    }

    let input = serde_json::json!({
        "query": "SELECT ?o FROM <urn:g:x1> FROM <urn:g:x2> WHERE { ?s ?p ?o }"
    });
    let out = crate::mcp::tool_query(&store, &input).expect("the QUERY must still succeed");
    assert_eq!(out["count"], 2, "rows are returned as normal");
    let err = out["labels"]["error"]
        .as_str()
        .expect("the refusal is reported IN the label, not raised");
    assert!(
        err.contains("urn:chain:a") && err.contains("urn:chain:b"),
        "{err}"
    );
}

#[test]
fn a_configured_floor_refuses_at_the_query_surface() {
    // #68 end to end: the refusal reaches the caller of /query & quipu_query.
    let (mut store, a, b) = labeled_store();
    store.labels_config_mut().min_freshness = Some("fresh".into());

    let input = serde_json::json!({
        "query": format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}")
    });
    let err = crate::mcp::tool_query(&store, &input).expect_err("urn:g:stale is below the floor");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:g:stale"),
        "names the offending graph: {msg}"
    );
    assert!(msg.contains("refused"), "{msg}");
}

#[test]
fn without_a_floor_the_same_query_succeeds() {
    // The control that makes the test above mean something: the query itself is
    // fine, and it is the FLOOR that refuses it.
    let (store, a, b) = labeled_store();
    let input = serde_json::json!({
        "query": format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}")
    });
    let out = crate::mcp::tool_query(&store, &input).expect("no floor configured");
    assert_eq!(out["count"], 2);
}

#[test]
fn a_floor_does_not_gate_the_raw_evaluator() {
    // Deliberate boundary (graph-labels.md §11): floors are a consumer-facing
    // quality gate at the service surface, NOT access control. The reasoner,
    // SHACL validation and the episode write path use `query`/`query_temporal`
    // and must keep working — refusing an internal maintenance query because a
    // graph is stale would break the machinery that makes it fresh again.
    let (mut store, a, b) = labeled_store();
    store.labels_config_mut().min_freshness = Some("fresh".into());
    let q = format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}");
    assert_eq!(
        query(&store, &q)
            .expect("the raw evaluator is not gated")
            .rows()
            .len(),
        2
    );
}

// --- quipu #69: named datasets ---

#[test]
fn from_a_dataset_iri_equals_from_over_its_members() {
    // #69 acceptance 1, asserted as an EQUIVALENCE rather than a row count, so
    // it cannot pass by coincidentally returning the right number of rows.
    use crate::store::datasets::DatasetMember;
    let (mut store, a, b) = labeled_store();
    store
        .dataset_create(
            "urn:ds:both",
            &[DatasetMember::new(&a), DatasetMember::new(&b)],
            "2026-08-06T00:00:00Z",
            None,
        )
        .unwrap();

    let via_members = query(
        &store,
        &format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}"),
    )
    .unwrap();
    let via_dataset = query(&store, "SELECT ?o FROM <urn:ds:both> WHERE { ?s ?p ?o }").unwrap();

    assert_eq!(canonical(&via_dataset), canonical(&via_members));
    assert_eq!(via_dataset.rows().len(), 2, "and it is not vacuously empty");
}

#[test]
fn a_dataset_is_never_implicitly_active() {
    // Silence must not widen the dataset. Registering one must not change what
    // a query with no FROM clause reads.
    use crate::store::datasets::DatasetMember;
    let (mut store, a, b) = labeled_store();
    let before = query(&store, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    store
        .dataset_create(
            "urn:ds:lurking",
            &[DatasetMember::new(&a), DatasetMember::new(&b)],
            "2026-08-06T00:00:00Z",
            None,
        )
        .unwrap();
    let after = query(&store, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(
        canonical(&after),
        canonical(&before),
        "the ROOT-alone default survives"
    );
}

#[test]
fn no_dataset_means_apply_dataset_behaves_exactly_as_today() {
    // #69 acceptance 4. With the datasets table empty, a FROM over ordinary
    // graphs is unchanged.
    let (store, a, b) = labeled_store();
    assert!(store.dataset_list().unwrap().is_empty());
    let out = query(
        &store,
        &format!("SELECT ?o FROM <{a}> FROM <{b}> WHERE {{ ?s ?p ?o }}"),
    )
    .unwrap();
    assert_eq!(out.rows().len(), 2);
}

#[test]
fn a_dataset_label_is_the_fold_over_its_members() {
    // The reason datasets and labels meet: #66's homomorphism is what makes a
    // named set's label well defined.
    use crate::store::datasets::DatasetMember;
    let (mut store, a, b) = labeled_store();
    store
        .dataset_create(
            "urn:ds:mixed",
            &[DatasetMember::new(&a), DatasetMember::new(&b)],
            "2026-08-06T00:00:00Z",
            None,
        )
        .unwrap();

    let out = query_labeled(
        &store,
        "SELECT ?o FROM <urn:ds:mixed> WHERE { ?s ?p ?o }",
        &TemporalContext::default(),
    )
    .unwrap();
    let labels = out.labels.expect("members declare freshness");
    assert_eq!(
        labels.freshness.value,
        Some(crate::lattice::Freshness::Stale),
        "fresh meet stale = stale, through the dataset name"
    );
}

#[test]
fn a_floor_sees_through_a_dataset_name() {
    // #68 + #69: the floor must not be bypassable by naming a dataset instead
    // of its members. Both go through the same resolve closure, which is why.
    use crate::store::datasets::DatasetMember;
    let (mut store, a, b) = labeled_store();
    store
        .dataset_create(
            "urn:ds:gated",
            &[DatasetMember::new(&a), DatasetMember::new(&b)],
            "2026-08-06T00:00:00Z",
            None,
        )
        .unwrap();
    store.labels_config_mut().min_freshness = Some("fresh".into());

    let err = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query": "SELECT ?o FROM <urn:ds:gated> WHERE { ?s ?p ?o }"}),
    )
    .expect_err("the stale member is still a member");
    assert!(err.to_string().contains("urn:g:stale"), "{err}");
}

#[test]
fn the_graph_param_resolves_a_dataset_name_too() {
    use crate::store::datasets::DatasetMember;
    let (mut store, a, b) = labeled_store();
    store
        .dataset_create(
            "urn:ds:param",
            &[DatasetMember::new(&a), DatasetMember::new(&b)],
            "2026-08-06T00:00:00Z",
            None,
        )
        .unwrap();
    let out = crate::mcp::tool_query(
        &store,
        &serde_json::json!({"query": "SELECT ?o WHERE { ?s ?p ?o }", "graph": "urn:ds:param"}),
    )
    .unwrap();
    assert_eq!(out["count"], 2, "`graph` and `FROM` must agree");
}

// --- quipu #70: per-row labels under GRAPH ?g + precedence as ORDER BY ---

/// Two named graphs at different trust ranks, plus a meta-graph ranking them.
/// This is graph-labels.md §6's worked example, as a fixture.
fn precedence_store() -> Store {
    use crate::lattice::{Freshness, Trust};
    use crate::store::labels::GraphLabel;
    use crate::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-08-06T00:00:00Z";
    let e_s = store.intern("http://example.org/s").unwrap();
    let a_p = store.intern("http://example.org/p").unwrap();

    for (iri, rank, fresh, val) in [
        ("urn:g:canonical", 40, Freshness::Fresh, "from-canonical"),
        ("urn:g:learned", 10, Freshness::Stale, "from-learned"),
    ] {
        let g = store.overlay_create(iri, 0).unwrap();
        store
            .overlay_write(g, Op::Assert, e_s, a_p, Value::Str(val.into()), ts)
            .unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    freshness: Some(fresh),
                    trust: Some(Trust::new(format!("{iri}#t"), "urn:chain:demo", rank)),
                    ..Default::default()
                },
                ts,
                None,
            )
            .unwrap();
    }
    store
}

#[test]
fn per_row_labels_appear_under_graph_var_when_requested() {
    // #70 acceptance 1, the "when requested" half.
    let store = precedence_store();
    let q = "SELECT ?g ?o WHERE { GRAPH ?g { ?s ?p ?o } }";
    let out = query_row_labeled(&store, q, &TemporalContext::default()).unwrap();

    assert!(out.variables().contains(&"_freshness".to_string()));
    assert_eq!(out.rows().len(), 2);
    for row in out.rows() {
        let g = value_to_iri(&store, row.get("g").unwrap());
        let f = row.get("_freshness").expect("annotated");
        let expected = if g == "urn:g:canonical" {
            "fresh"
        } else {
            "stale"
        };
        assert_eq!(
            f,
            &Value::Str(expected.into()),
            "row label follows its own graph"
        );
    }
}

#[test]
fn per_row_labels_are_absent_without_a_graph_variable() {
    // #70 acceptance 1, the "only under GRAPH ?g" half. A FROM-union query has
    // no per-row graph — the dataset label (#67) is the right granularity, and
    // projecting `g` to fake one would change the results.
    let store = precedence_store();
    let q = "SELECT ?o FROM <urn:g:canonical> FROM <urn:g:learned> WHERE { ?s ?p ?o }";
    let out = query_row_labeled(&store, q, &TemporalContext::default()).unwrap();
    assert!(!out.variables().contains(&"_freshness".to_string()));
}

#[test]
fn per_row_labels_are_off_unless_asked_for() {
    // Opt-in per request: the default response shape is unchanged.
    let store = precedence_store();
    let q = "SELECT ?g ?o WHERE { GRAPH ?g { ?s ?p ?o } }";

    let plain = crate::mcp::tool_query(&store, &serde_json::json!({"query": q})).unwrap();
    assert!(
        plain["rows"][0].get("_freshness").is_none(),
        "off by default"
    );

    let asked =
        crate::mcp::tool_query(&store, &serde_json::json!({"query": q, "row_labels": true}))
            .unwrap();
    assert!(
        asked["rows"][0].get("_freshness").is_some(),
        "on when asked"
    );
}

#[test]
fn the_precedence_query_returns_plane_ordered_results() {
    // #70 acceptance 2 — graph-labels.md §6's worked example, pinned. This is
    // the query the whole design promises needs NO engine change, and it joins
    // two GRAPH patterns on ?g (verified during sequencing; the sabotage that
    // proves the join is load-bearing is `the_g_join_is_not_a_cartesian` below).
    let store = precedence_store();
    // NOTE the extra hop through `quipu:trust`. graph-labels.md §6 writes this
    // example as `GRAPH <meta> { ?g quipu:trustRank ?rank }`, i.e. as if the
    // RANK were a property of the graph — but §2 is explicit that the rank
    // belongs to the TRUST VALUE (`smac:canonical quipu:trustRank 40`), which is
    // what makes an ordering shareable data rather than a per-graph number. The
    // two sections disagree, and §6 taken literally returns ZERO rows.
    let q = format!(
        "SELECT ?o ?g ?rank WHERE {{ \
           GRAPH ?g {{ ?s <http://example.org/p> ?o }} \
           GRAPH <{meta}> {{ ?g <{trust_p}> ?t . ?t <{rank_p}> ?rank }} \
         }} ORDER BY DESC(?rank)",
        meta = crate::namespace::META_GRAPH_IRI,
        trust_p = crate::namespace::QUIPU_TRUST,
        rank_p = crate::namespace::QUIPU_TRUST_RANK,
    );
    let out = query(&store, &q).unwrap();
    let rows = out.rows();
    assert_eq!(rows.len(), 2, "one row per data graph");
    assert_eq!(
        rows[0].get("rank"),
        Some(&Value::Int(40)),
        "canonical (40) outranks learned (10)"
    );
    assert_eq!(rows[1].get("rank"), Some(&Value::Int(10)));
}

#[test]
fn the_g_join_is_not_a_cartesian() {
    // The sabotage, kept as a test: unsharing ?g must change the answer. If it
    // did not, the precedence test above would pass without the join doing any
    // work — 2 rows by luck rather than by pairing.
    let store = precedence_store();
    let q = format!(
        "SELECT ?o ?g ?rank WHERE {{ \
           GRAPH ?g {{ ?s <http://example.org/p> ?o }} \
           GRAPH <{meta}> {{ ?other <{trust_p}> ?t . ?t <{rank_p}> ?rank }} \
         }}",
        meta = crate::namespace::META_GRAPH_IRI,
        trust_p = crate::namespace::QUIPU_TRUST,
        rank_p = crate::namespace::QUIPU_TRUST_RANK,
    );
    assert_eq!(
        query(&store, &q).unwrap().rows().len(),
        4,
        "unshared ?g is a 2x2 cartesian — so the shared form's 2 rows are a real join"
    );
}

#[test]
fn ties_are_returned_as_ties_never_silently_broken() {
    // #70 acceptance 2, second half. Two graphs at the SAME rank must both come
    // back. A silent tiebreak is how "learned tactic beats canonical" ships.
    use crate::lattice::Trust;
    use crate::store::labels::GraphLabel;
    use crate::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-08-06T00:00:00Z";
    let e_s = store.intern("http://example.org/s").unwrap();
    let a_p = store.intern("http://example.org/p").unwrap();
    for iri in ["urn:g:tieA", "urn:g:tieB"] {
        let g = store.overlay_create(iri, 0).unwrap();
        store
            .overlay_write(g, Op::Assert, e_s, a_p, Value::Str(iri.into()), ts)
            .unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    trust: Some(Trust::new(format!("{iri}#t"), "urn:chain:demo", 25)),
                    ..Default::default()
                },
                ts,
                None,
            )
            .unwrap();
    }

    let q = format!(
        "SELECT ?g ?rank WHERE {{ \
           GRAPH ?g {{ ?s <http://example.org/p> ?o }} \
           GRAPH <{meta}> {{ ?g <{trust_p}> ?t . ?t <{rank_p}> ?rank }} \
         }} ORDER BY DESC(?rank)",
        meta = crate::namespace::META_GRAPH_IRI,
        trust_p = crate::namespace::QUIPU_TRUST,
        rank_p = crate::namespace::QUIPU_TRUST_RANK,
    );
    let out = query(&store, &q).unwrap();
    assert_eq!(
        out.rows().len(),
        2,
        "both tied graphs are returned, not one"
    );
    for r in out.rows() {
        assert_eq!(r.get("rank"), Some(&Value::Int(25)));
    }
}

#[test]
fn cross_chain_row_labels_error_rather_than_being_compared() {
    // #70 acceptance 3. Nothing here composes ranks — but the point of these
    // columns is ORDER BY, and ordering ranks from two chains is exactly the
    // silent cross-chain comparison the trust axis exists to refuse.
    use crate::lattice::Trust;
    use crate::store::labels::GraphLabel;
    use crate::types::{Op, Value};

    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-08-06T00:00:00Z";
    let e_s = store.intern("http://example.org/s").unwrap();
    let a_p = store.intern("http://example.org/p").unwrap();
    for (iri, chain) in [("urn:g:c1", "urn:chain:one"), ("urn:g:c2", "urn:chain:two")] {
        let g = store.overlay_create(iri, 0).unwrap();
        store
            .overlay_write(g, Op::Assert, e_s, a_p, Value::Str(iri.into()), ts)
            .unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    trust: Some(Trust::new(format!("{iri}#t"), chain, 10)),
                    ..Default::default()
                },
                ts,
                None,
            )
            .unwrap();
    }

    let err = query_row_labeled(
        &store,
        "SELECT ?g ?o WHERE { GRAPH ?g { ?s ?p ?o } }",
        &TemporalContext::default(),
    )
    .expect_err("two chains in one result set");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:chain:one") && msg.contains("urn:chain:two"),
        "{msg}"
    );
}

#[test]
fn graph_var_excludes_the_meta_graph_but_it_stays_reachable_by_name() {
    // A DECISION, pinned (quipu #70). `GRAPH ?g { ?s ?p ?o }` is the natural
    // "every named graph's triples" query. Letting ?g range over the reserved
    // label meta-graph would make it return freshness/trust facts as if they
    // were data — and a consumer's result set would silently change the first
    // time anyone labelled anything.
    //
    // Naming the meta-graph is deliberate; ranging over it is not.
    let store = precedence_store();

    let ranged = query(&store, "SELECT ?g ?o WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap();
    for row in ranged.rows() {
        assert_ne!(
            value_to_iri(&store, row.get("g").unwrap()),
            crate::namespace::META_GRAPH_IRI,
            "?g must not range over the meta-graph"
        );
    }
    assert_eq!(ranged.rows().len(), 2, "just the two data graphs");

    // But an explicit name still reads it — §6's precedence query depends on it.
    let named = query(
        &store,
        &format!(
            "SELECT ?s ?o WHERE {{ GRAPH <{}> {{ ?s <{}> ?o }} }}",
            crate::namespace::META_GRAPH_IRI,
            crate::namespace::QUIPU_TRUST_RANK,
        ),
    )
    .unwrap();
    assert_eq!(
        named.rows().len(),
        2,
        "explicitly named, it is fully readable"
    );
}

// -- quipu-0lr: LIMIT pushdown and selectivity-ordered joins ----------------

/// A bounded single-pattern scan returns exactly LIMIT rows, each a real
/// solution of the unbounded query (the pushdown may reorder nothing).
#[test]
fn limit_pushdown_returns_a_correct_prefix() {
    let store = test_store_with_data();
    let full = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Person> }",
    )
    .unwrap();
    assert_eq!(full.rows().len(), 2);
    let limited = query(
        &store,
        "SELECT ?s WHERE { ?s a <http://example.org/Person> } LIMIT 1",
    )
    .unwrap();
    assert_eq!(limited.rows().len(), 1);
    assert!(
        full.rows().contains(&limited.rows()[0]),
        "the limited row must be one of the full query's solutions"
    );
}

/// OFFSET participates in the pushed cap (start + length), so a paged read
/// still sees the row the offset points at.
#[test]
fn limit_pushdown_respects_offset() {
    let store = test_store_with_data();
    let full = query(&store, "SELECT ?s WHERE { ?s ?p ?o }").unwrap();
    let paged = query(&store, "SELECT ?s WHERE { ?s ?p ?o } OFFSET 2 LIMIT 3").unwrap();
    assert_eq!(paged.rows().len(), 3);
    assert_eq!(
        paged.rows(),
        &full.rows()[2..5],
        "same rows as slicing the full set"
    );
}

/// A FILTER between LIMIT and the BGP blocks the pushdown — the filtered
/// query must still find every matching row, not a capped prefix of inputs.
#[test]
fn limit_does_not_push_through_filter() {
    let store = test_store_with_data();
    let rows = query(
        &store,
        "SELECT ?s WHERE { ?s <http://example.org/age> ?age . FILTER(?age > 26) } LIMIT 5",
    )
    .unwrap();
    // alice (30) and carol (35): a naive cap of 5 SQL rows could have stopped
    // on bob (25) rows and missed one.
    assert_eq!(rows.rows().len(), 2);
}

/// ACCEPTANCE (quipu-0lr): the same join written pathologically (broad scan
/// first) and well (selective pattern first) returns identical solutions, and
/// the planner provably folds them in the same order.
#[test]
fn join_order_is_planned_not_inherited() {
    let store = test_store_with_data();
    // Broad-first: ?s ?p ?o style scan, then the selective name lookup.
    let bad = query(
        &store,
        r#"SELECT ?s ?o WHERE { ?s <http://example.org/knows> ?o . ?s <http://example.org/name> "Alice" }"#,
    )
    .unwrap();
    let good = query(
        &store,
        r#"SELECT ?s ?o WHERE { ?s <http://example.org/name> "Alice" . ?s <http://example.org/knows> ?o }"#,
    )
    .unwrap();
    let norm = |r: &QueryResult| {
        let mut v = r.rows().to_vec();
        v.sort_by_key(|row| format!("{row:?}"));
        v
    };
    assert_eq!(norm(&bad), norm(&good));
    assert_eq!(bad.rows().len(), 1, "alice knows bob");
}

/// The pure planner: a pathological source ordering yields the same plan as a
/// good one — smallest first, then connected-smallest, cartesian only when
/// the query genuinely contains one.
#[test]
fn join_plan_ignores_source_order() {
    use std::collections::HashSet;
    let vars = |names: &[&str]| -> Vec<String> { names.iter().map(ToString::to_string).collect() };
    let none = HashSet::new();

    // good order: tiny (10), linked (100), huge (10_000)
    let good = super::join::join_plan(
        &[vars(&["x"]), vars(&["x", "y"]), vars(&["y", "z"])],
        &[10, 100, 10_000],
        &none,
    );
    assert_eq!(good, vec![0, 1, 2]);

    // pathological order: same patterns listed huge-first
    let bad = super::join::join_plan(
        &[vars(&["y", "z"]), vars(&["x", "y"]), vars(&["x"])],
        &[10_000, 100, 10],
        &none,
    );
    assert_eq!(bad, vec![2, 1, 0], "the fold sequence is identical");

    // A disconnected small pattern must not bait the planner into a cartesian
    // while a connected alternative exists.
    let plan = super::join::join_plan(
        &[vars(&["a"]), vars(&["a", "b"]), vars(&["q"])],
        &[50, 500, 2],
        &none,
    );
    assert_eq!(
        plan,
        vec![2, 0, 1],
        "smallest starts; then the connected chain beats staying small"
    );
}
