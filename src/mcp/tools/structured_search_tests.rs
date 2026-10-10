use super::*;
use crate::vector::{KnowledgeVectorStore, VectorMatch};
use crate::{Datum, Op, Value};
use std::collections::HashSet;
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const AT: &str = "2026-01-01T00:00:00Z";
fn put(store: &mut Store, entity: &str, predicate: &str, value: Value) {
    let datum = Datum {
        entity: store.intern(entity).unwrap(),
        attribute: store.intern(predicate).unwrap(),
        value,
        valid_from: AT.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[datum], AT, None, None).unwrap();
}
fn fixture() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.search_config_mut().structured = true;
    store.search_config_mut().keyword = true;
    store.initialize_lexical_index().unwrap();
    store
        .load_shapes("prefix", "@prefix ex: <https://example.org/> .", AT)
        .unwrap();
    for (name, label) in [
        ("Service", "Service"),
        ("status", "status"),
        ("created", "created"),
        ("owner", "owner"),
        ("Alice", "Alice"),
    ] {
        put(
            &mut store,
            &format!("https://example.org/{name}"),
            LABEL,
            Value::Str(label.into()),
        );
    }
    for (name, status, date, vector) in [
        ("a", "open", "2026-01-03", [1.0, 0.0]),
        ("b", "closed", "2025-12-30", [0.0, 1.0]),
        ("c", "open", "2026-01-04", [0.0, 1.0]),
    ] {
        let entity = format!("https://example.org/{name}");
        let type_id = store.intern("https://example.org/Service").unwrap();
        put(
            &mut store,
            &entity,
            crate::namespace::RDF_TYPE,
            Value::Ref(type_id),
        );
        put(
            &mut store,
            &entity,
            "https://example.org/status",
            Value::Str(status.into()),
        );
        put(
            &mut store,
            &entity,
            "https://example.org/created",
            Value::Str(date.into()),
        );
        put(
            &mut store,
            &entity,
            LABEL,
            Value::Str(format!("quipu {name}")),
        );
        store
            .embed_entity(store.lookup(&entity).unwrap().unwrap(), name, &vector, AT)
            .unwrap();
    }
    let alice = store.lookup("https://example.org/Alice").unwrap().unwrap();
    put(
        &mut store,
        "https://example.org/c",
        "https://example.org/owner",
        Value::Ref(alice),
    );
    while !store.backfill_lexical_batch(64).unwrap().complete {}
    store
}
fn request(query: &str) -> Json {
    json!({"structured_query":query,"embedding":[1.0,0.0],"verbose":true})
}
fn entities(response: &Json) -> HashSet<String> {
    response["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["entity"].as_str().unwrap().into())
        .collect()
}
#[test]
fn default_off_explicitly_refuses() {
    let mut store = fixture();
    store.search_config_mut().structured = false;
    assert!(
        search(&store, &request("type:Service"))
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
}
#[test]
fn naive_boolean_range_and_one_hop_controls() {
    let store = fixture();
    for (expr, expected) in [
        ("type:Service status:open", vec!["a", "c"]),
        ("type:Service NOT status:open", vec!["b"]),
        ("status:closed OR owner:Alice", vec!["b", "c"]),
        ("type:Service created>=2026-01-04", vec!["c"]),
        (
            "type:Service (status:closed OR owner:Alice)",
            vec!["b", "c"],
        ),
        ("ex:status:open", vec!["a", "c"]),
        ("<https://example.org/status>:closed", vec!["b"]),
        ("quip* type:Service", vec!["a", "b", "c"]),
    ] {
        let want: HashSet<_> = expected
            .iter()
            .map(|s| format!("https://example.org/{s}"))
            .collect();
        assert_eq!(
            entities(&search(&store, &request(expr)).unwrap()),
            want,
            "{expr}"
        );
    }
}
#[test]
fn json_dsl_equivalence_and_strict_refusals() {
    let store = fixture();
    let mut input = request("type:Service NOT status:closed");
    input.as_object_mut().unwrap().remove("structured_query");
    input["filters"] = json!({"and":[{"term":"type:Service"},{"not":{"term":"status:closed"}}]});
    assert_eq!(
        entities(&search(&store, &input).unwrap()),
        entities(&search(&store, &request("type:Service NOT status:closed")).unwrap())
    );
    for expr in [
        json!({"unknown":"type:Service"}),
        json!({"term":"a OR b"}),
        json!({"and":[]}),
        json!({"term":"a","extra":true}),
    ] {
        input["filters"] = expr;
        assert!(search(&store, &input).is_err());
    }
    for query in [
        "NOT type:Service",
        "type:Service OR NOT status:open",
        "status:",
        "created>NaN",
        "unknown:open",
        "status:<https://example.org/x>}",
    ] {
        assert!(search(&store, &request(query)).is_err(), "{query}");
    }
}
#[test]
fn candidate_before_top_k_finds_low_score() {
    let store = fixture();
    for n in 0..30 {
        let id = store
            .intern(&format!("https://example.org/distractor{n}"))
            .unwrap();
        store
            .embed_entity(id, "distractor", &[1.0, 0.0], AT)
            .unwrap();
    }
    let mut input = request("owner:Alice");
    input["limit"] = json!(1);
    assert_eq!(
        entities(&search(&store, &input).unwrap()),
        HashSet::from(["https://example.org/c".into()])
    );
}
#[test]
fn scope_and_non_sqlite_refuse_without_fallback() {
    let mut store = fixture();
    for key in [
        "graph",
        "graphs",
        "anchor",
        "group_ids",
        "entity_type",
        "alpha",
        "fusion",
        "rrf_k",
    ] {
        let mut input = request("type:Service");
        input[key] = json!("unknown");
        assert!(search(&store, &input).is_err(), "{key}");
    }
    struct Backend;
    impl crate::vector_delegate::VectorSearchDelegate for Backend {
        fn vector_search(&self, _: &[f32], _: usize, _: Option<&str>) -> Result<Vec<VectorMatch>> {
            panic!("must not fall back")
        }
        fn vector_count(&self) -> Result<usize> {
            Ok(0)
        }
    }
    store.set_vector_search_delegate(std::sync::Arc::new(Backend));
    assert!(
        search(&store, &request("type:Service"))
            .unwrap_err()
            .to_string()
            .contains("SQLite")
    );
}
#[test]
fn temporal_candidates_and_vectors_match_same_time() {
    let store = fixture();
    let c = store.lookup("https://example.org/c").unwrap().unwrap();
    store.close_embedding(c, "2026-02-01T00:00:00Z").unwrap();
    let mut input = request("owner:Alice");
    assert!(entities(&search(&store, &input).unwrap()).is_empty());
    input["valid_at"] = json!("2026-01-15T00:00:00Z");
    assert_eq!(
        entities(&search(&store, &input).unwrap()),
        HashSet::from(["https://example.org/c".into()])
    );
}

#[test]
fn independent_naive_model_boolean_differential() {
    fn matches(expr: &Expr, name: &str) -> bool {
        match expr {
            Expr::Atom(atom) => match atom.as_str() {
                "type:Service" => true,
                "status:open" => name != "b",
                "owner:Alice" => name == "c",
                "created>=2026-01-04" => name == "c",
                "\"quipu a\"" => name == "a",
                _ => panic!("unmodeled atom {atom}"),
            },
            Expr::And(a, b) => matches(a, name) && matches(b, name),
            Expr::Or(a, b) => matches(a, name) || matches(b, name),
            Expr::Not(a) => !matches(a, name),
        }
    }
    let store = fixture();
    let atoms = [
        "status:open",
        "owner:Alice",
        "created>=2026-01-04",
        "\"quipu a\"",
    ];
    for a in atoms {
        for b in atoms {
            for op in ["AND", "OR"] {
                for not in ["", "NOT "] {
                    let text = format!("type:Service ({a} {op} {not}{b})");
                    let ast = structured_syntax::parse(&text).unwrap();
                    let expected: HashSet<_> = ["a", "b", "c"]
                        .into_iter()
                        .filter(|s| matches(&ast, s))
                        .map(|s| format!("https://example.org/{s}"))
                        .collect();
                    assert_eq!(
                        entities(&search(&store, &request(&text)).unwrap()),
                        expected,
                        "{text}"
                    );
                }
            }
        }
    }
}
#[test]
fn bad_dates_and_ambiguous_labels_refuse_without_panic() {
    let mut store = fixture();
    for value in ["2026-02-29", "2024-13-01", "0000-01-01", "💩abcdef"] {
        assert!(search(&store, &request(&format!("created>={value}"))).is_err());
    }
    put(
        &mut store,
        "https://example.org/OtherAlice",
        LABEL,
        Value::Str("Alice".into()),
    );
    assert!(
        search(&store, &request("owner:Alice"))
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    assert_eq!(
        entities(&search(&store, &request("owner:ex:Alice")).unwrap()),
        HashSet::from(["https://example.org/c".into()])
    );
}
#[test]
fn numeric_comparisons_and_keyword_scoping() {
    let mut store = fixture();
    put(
        &mut store,
        "https://example.org/memory",
        LABEL,
        Value::Str("memory".into()),
    );
    put(
        &mut store,
        "https://example.org/a",
        "https://example.org/memory",
        Value::Int(4),
    );
    put(
        &mut store,
        "https://example.org/b",
        "https://example.org/memory",
        Value::Int(16),
    );
    assert_eq!(
        entities(&search(&store, &request("memory>8")).unwrap()),
        HashSet::from(["https://example.org/b".into()])
    );
    let mut input = request("owner:Alice");
    input.as_object_mut().unwrap().remove("embedding");
    input["query"] = json!("quipu");
    input["mode"] = json!("keyword");
    assert_eq!(
        entities(&search(&store, &input).unwrap()),
        HashSet::from(["https://example.org/c".into()])
    );
    input["structured_query"] = json!("status:missing");
    assert!(entities(&search(&store, &input).unwrap()).is_empty());
}
#[test]
fn candidate_cap_positive_and_overflow_controls() {
    let mut store = fixture();
    let predicate = store.intern("https://example.org/population").unwrap();
    let mut datums = Vec::new();
    for n in 0..4097 {
        datums.push(Datum {
            entity: store
                .intern(&format!("https://example.org/pop{n}"))
                .unwrap(),
            attribute: predicate,
            value: Value::Str(if n == 4096 { "overflow" } else { "base" }.into()),
            valid_from: AT.into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    store.transact(&datums, AT, None, None).unwrap();
    let ctx = Context {
        store: &store,
        at: None,
    };
    let parse = |s| structured_syntax::parse(s).unwrap();
    assert_eq!(
        ctx.eval(&parse("ex:population:base"), None).unwrap().len(),
        4096
    );
    assert!(
        ctx.eval(&parse("ex:population:base OR ex:population:overflow"), None)
            .is_err()
    );
    assert!(ctx.eval(&parse("ex:population:b*"), None).is_ok());
    assert!(ctx.eval(&parse("ex:population:*"), None).is_err());
}
#[test]
fn malformed_ranking_inputs_refuse() {
    let store = fixture();
    for embedding in [json!([]), json!([1e100, 0]), json!(["bad", 0]), json!(null)] {
        let mut input = request("type:Service");
        input["embedding"] = embedding;
        assert!(search(&store, &input).is_err());
    }
    let mut input = request("type:Service");
    input["mode"] = json!(13);
    assert!(search(&store, &input).is_err());
}
#[test]
fn unchanged_legacy_search_and_native_schema() {
    let store = fixture();
    let input = json!({"embedding":[1.0,0.0],"verbose":true});
    let before = super::super::search::tool_search(&store, &input).unwrap();
    assert_eq!(before["count"], 3);
    let _ = search(&store, &request("owner:Alice")).unwrap();
    assert_eq!(
        super::super::search::tool_search(&store, &input).unwrap(),
        before
    );
    let definition = crate::tool_definitions()
        .into_iter()
        .find(|d| d["name"] == "quipu_search")
        .unwrap();
    assert_eq!(
        definition["inputSchema"]["properties"]["structured_query"]["type"],
        "string"
    );
    assert_eq!(
        definition["inputSchema"]["properties"]["filters"]["type"],
        "object"
    );
}

#[test]
fn temporal_fact_and_root_dataset_controls() {
    let mut store = fixture();
    let entity = store.intern("https://example.org/future").unwrap();
    let status = store.intern("https://example.org/status").unwrap();
    store
        .transact(
            &[Datum {
                entity,
                attribute: status,
                value: Value::Str("future".into()),
                valid_from: "2026-02-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-02-01T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    store
        .embed_entity(entity, "future", &[1.0, 0.0], AT)
        .unwrap();
    let mut input = request("ex:status:future");
    assert_eq!(search(&store, &input).unwrap()["count"], 1);
    input["valid_at"] = json!("2026-01-15T00:00:00Z");
    assert_eq!(search(&store, &input).unwrap()["count"], 0);
    input["structured_query"] = json!("ex:status:open");
    assert_eq!(search(&store, &input).unwrap()["count"], 2);
    let graph = store.overlay_create("urn:test:named", 0).unwrap();
    crate::rdf::ingest_rdf_to_graph(
        &mut store,
        b"<https://example.org/named> <https://example.org/status> \"open\" .".as_slice(),
        oxrdfio::RdfFormat::Turtle,
        None,
        AT,
        None,
        None,
        graph,
    )
    .unwrap();
    let named = store.lookup("https://example.org/named").unwrap().unwrap();
    store.embed_entity(named, "named", &[1.0, 0.0], AT).unwrap();
    assert_eq!(
        search(&store, &request("ex:status:open")).unwrap()["count"],
        2
    );
}
#[test]
fn expired_ambient_deadline_refuses_then_restores_control() {
    let store = fixture();
    let input = request("type:Service");
    {
        let _guard =
            crate::time::set_request_deadline(Some(crate::time::Deadline::after_millis(0)));
        assert!(search(&store, &input).is_err());
    }
    assert_eq!(search(&store, &input).unwrap()["count"], 3);
}
