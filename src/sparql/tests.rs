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
