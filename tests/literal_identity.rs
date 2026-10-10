//! RDF spelling survives ingestion, query constants, joins, and value operations.
use oxrdfio::RdfFormat;
use quipu::{Store, Value};

fn load(store: &mut Store, text: &str) {
    quipu::ingest_rdf(
        store,
        text.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
}

#[test]
fn lexical_variants_and_ill_typed_literals_round_trip() {
    let mut store = Store::open_in_memory().unwrap();
    let cases = [
        ("01", "integer"),
        ("+1", "integer"),
        ("1", "integer"),
        ("-0", "integer"),
        ("00", "integer"),
        ("9223372036854775808", "integer"),
        ("z", "integer"),
        ("z", "boolean"),
        ("1", "boolean"),
        ("true", "boolean"),
        ("1e0", "double"),
        ("1E0", "double"),
        ("-0e0", "double"),
        ("NaN", "double"),
        ("INF", "double"),
        ("1.00000000000000000001", "decimal"),
        ("bad", "decimal"),
    ];
    for (n, (lexical, datatype)) in cases.iter().enumerate() {
        load(
            &mut store,
            &format!(
                "<http://example.org/s{n}> <http://example.org/p> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>; <http://example.org/marker> true ."
            ),
        );
    }
    let exported =
        String::from_utf8(quipu::export_rdf(&store, RdfFormat::NTriples).unwrap()).unwrap();
    for (lexical, datatype) in cases {
        assert!(
            exported.contains(&format!(
                "\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>"
            )),
            "missing {lexical}:{datatype}"
        );
        for enabled in [false, true] {
            store.set_read_model_enabled(enabled);
            let query = format!(
                "SELECT ?s WHERE {{ ?s <http://example.org/p> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>; <http://example.org/marker> true }}"
            );
            assert_eq!(
                quipu::sparql::query(&store, &query).unwrap().rows().len(),
                1,
                "{lexical}:{datatype} model={enabled}"
            );
        }
    }
}

#[test]
fn min_and_max_return_original_numeric_terms() {
    let mut store = Store::open_in_memory().unwrap();
    load(
        &mut store,
        "<http://example.org/s> <http://example.org/p> 2E-1, 2.2, 3E1 .",
    );
    // SPARQL 18.5.1.5/6 select an input term, rather than constructing a
    // canonical spelling. The pinned agg-min-02 expected file spells its
    // input 2E-1 as 2.0E-1; keep the runtime contract explicit here.
    for enabled in [false, true] {
        store.set_read_model_enabled(enabled);
        let result = quipu::sparql::query(
            &store,
            "SELECT (MIN(?v) AS ?min) (MAX(?v) AS ?max) WHERE { ?s <http://example.org/p> ?v }",
        )
        .unwrap();
        for (name, lexical) in [("min", "2E-1"), ("max", "3E1")] {
            assert_eq!(
                result.rows()[0].get(name),
                Some(&Value::Typed {
                    lexical: lexical.into(),
                    datatype: "http://www.w3.org/2001/XMLSchema#double".into(),
                }),
                "{name} model={enabled}",
            );
        }
    }
}

#[test]
fn value_equality_is_separate_from_same_term_and_str() {
    let store = Store::open_in_memory().unwrap();
    let query = r#"PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
      SELECT (sameTerm("01"^^xsd:integer,1) AS ?term)
       ("01"^^xsd:integer = 1 AS ?equal) (STR("01"^^xsd:integer) AS ?str)
       ("9007199254740993"^^xsd:integer = 9007199254740992 AS ?rounded)
       ("184467440737095516160"^^xsd:integer = "184467440737095516160.0"^^xsd:decimal AS ?large)
       ("1.00000000000000000001"^^xsd:decimal < "1.00000000000000000002"^^xsd:decimal AS ?precise)
       ("1"^^xsd:boolean = true AS ?bool)
       (sameTerm("z"^^xsd:boolean,"z"^^xsd:boolean) AS ?invalid)
       ("+9007199254740993"^^xsd:integer + 1 AS ?sum)
       WHERE {}"#;
    let result = quipu::sparql::query(&store, query).unwrap();
    let row = &result.rows()[0];
    for name in ["equal", "large", "precise", "bool", "invalid"] {
        assert_eq!(row.get(name), Some(&Value::Bool(true)), "{name}");
    }
    for name in ["term", "rounded"] {
        assert_eq!(row.get(name), Some(&Value::Bool(false)), "{name}");
    }
    assert_eq!(row.get("str"), Some(&Value::Str("01".into())));
    assert_eq!(row.get("sum"), Some(&Value::Int(9007199254740994)));
}

#[test]
fn derived_numeric_operations_do_not_depend_on_storage_variant() {
    let store = Store::open_in_memory().unwrap();
    let result = quipu::sparql::query(
        &store,
        r#"PREFIX xsd:<http://www.w3.org/2001/XMLSchema#>
      SELECT (SUM(?v) AS ?sum) (AVG(?v) AS ?avg) (MIN(?v) AS ?min)
      WHERE { VALUES ?v { "+9007199254740993"^^xsd:integer "9007199254740995"^^xsd:integer } }"#,
    )
    .unwrap();
    let row = &result.rows()[0];
    assert_eq!(row.get("sum"), Some(&Value::Int(18014398509481988)));
    assert_eq!(
        row.get("avg"),
        Some(&Value::Typed {
            lexical: "9007199254740994.0".into(),
            datatype: quipu::namespace::XSD_DECIMAL.into()
        })
    );
    assert_eq!(
        row.get("min"),
        Some(&Value::Typed {
            lexical: "+9007199254740993".into(),
            datatype: quipu::namespace::XSD_INTEGER.into()
        })
    );
    let result = quipu::sparql::query(
        &store,
        r#"PREFIX xsd:<http://www.w3.org/2001/XMLSchema#>
      SELECT ("+9007199254740993"^^xsd:integer / 1 AS ?quotient)
      (ABS("+9007199254740993"^^xsd:integer) AS ?abs)
      (FLOOR("9007199254740993.1"^^xsd:decimal) AS ?floor)
      (ROUND("-1.5"^^xsd:decimal) AS ?round)
      (-"+9007199254740993"^^xsd:integer AS ?negative) WHERE {}"#,
    )
    .unwrap();
    let row = &result.rows()[0];
    assert_eq!(
        row.get("quotient"),
        Some(&Value::Typed {
            lexical: "9007199254740993.0".into(),
            datatype: quipu::namespace::XSD_DECIMAL.into()
        })
    );
    assert_eq!(row.get("negative"), Some(&Value::Int(-9007199254740993)));
    assert_eq!(row.get("abs"), Some(&Value::Int(9007199254740993)));
    for (name, lexical) in [("floor", "9007199254740993"), ("round", "-1")] {
        assert_eq!(
            row.get(name),
            Some(&Value::Typed {
                lexical: lexical.into(),
                datatype: quipu::namespace::XSD_DECIMAL.into(),
            })
        );
    }
}

#[test]
fn physical_import_and_fork_promotion_share_literal_identity() {
    use quipu::{Datum, Op};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let destination = dir.path().join("destination.db");
    let mut old = Store::open(source.to_str().unwrap()).unwrap();
    let e = old.intern("http://example.org/s").unwrap();
    let a = old.intern("http://example.org/p").unwrap();
    let make = |value, op| Datum {
        entity: e,
        attribute: a,
        value,
        op,
        valid_from: "2026-01-01".into(),
        valid_to: None,
    };
    old.transact(
        &[make(Value::Float(1.0), Op::Assert)],
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    drop(old);
    quipu::store::import::import_graph(&destination, &source, quipu::schema::ROOT_GRAPH_IRI)
        .unwrap();
    let mut store = Store::open(destination.to_str().unwrap()).unwrap();
    let e = store.lookup("http://example.org/s").unwrap().unwrap();
    let a = store.lookup("http://example.org/p").unwrap().unwrap();
    let value = Value::Typed {
        lexical: "1".into(),
        datatype: quipu::namespace::XSD_DOUBLE.into(),
    };
    let d = Datum {
        entity: e,
        attribute: a,
        value,
        op: Op::Assert,
        valid_from: "2026-01-02".into(),
        valid_to: None,
    };
    store
        .transact(std::slice::from_ref(&d), "2026-01-02", None, None)
        .unwrap();
    assert_eq!(store.current_facts().unwrap().len(), 1);
    assert_eq!(
        store.current_facts().unwrap()[0].value.to_bytes(),
        Value::Float(1.0).to_bytes()
    );
    let fork = store
        .fork_create(
            "literal-test",
            store.latest_tx_id().unwrap(),
            "2026-01-03",
            None,
        )
        .unwrap();
    store
        .transact_to_graph(std::slice::from_ref(&d), "2026-01-03", None, None, fork.g)
        .unwrap();
    assert!(
        store
            .fork_diff("main", "literal-test")
            .unwrap()
            .added
            .is_empty()
    );
    let mut retract = d;
    retract.op = Op::Retract;
    store
        .transact_to_graph(&[retract], "2026-01-04", None, None, fork.g)
        .unwrap();
    assert_eq!(
        store
            .fork_diff("main", "literal-test")
            .unwrap()
            .removed
            .len(),
        1
    );
    store
        .fork_promote("literal-test", "2026-01-05", None)
        .unwrap();
    assert!(
        store
            .current_facts()
            .unwrap()
            .iter()
            .all(|f| f.entity != e || f.attribute != a)
    );
}
