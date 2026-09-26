//! CLI enumeration uses the same stored query as HTTP /ask.
use quipu::{
    Store,
    store::Datum,
    types::{Op, Value},
};
use serde_json::Value as Json;

const TS: &str = "2026-09-26T00:00:00Z";

fn list(dir: &std::path::Path, db: &std::path::Path) -> Json {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_quipu"))
        .current_dir(dir)
        .args(["demotions", "--db"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
fn fresh_store_has_no_unsupported_demotions() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(list(dir.path(), &dir.path().join("empty.db"))["count"], 0);
}

#[test]
fn cli_and_http_tool_find_the_retained_fact() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("demoted.db");
    let mut store = Store::open(db.to_str().unwrap()).unwrap();
    let rules = quipu::reasoner::parse_rules(
        r#"
        @prefix rule: <http://quipu.local/rule#> .
        <urn:test:lift> a rule:Rule ; rule:id "LIFT" ;
        rule:head "<urn:test:h>(?x, ?y)" ; rule:body "<urn:test:p>(?x, ?y)" .
    "#,
        None,
    )
    .unwrap();
    let s = store.intern("urn:test:s").unwrap();
    let p = store.intern("urn:test:p").unwrap();
    let h = store.intern("urn:test:h").unwrap();
    let o = store.intern("urn:test:o").unwrap();
    let d = Datum {
        entity: s,
        attribute: p,
        value: Value::Ref(o),
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(std::slice::from_ref(&d), TS, None, Some("base"))
        .unwrap();
    quipu::reasoner::evaluate(&mut store, &rules, TS).unwrap();
    let c = store.ensure_companion_inferred_graph(0, TS).unwrap();
    store
        .transact_to_graph(
            &[Datum {
                attribute: h,
                op: Op::Retract,
                ..d.clone()
            }],
            TS,
            None,
            Some("reasoner:LIFT"),
            c,
        )
        .unwrap();
    store
        .transact(
            &[Datum {
                attribute: h,
                ..d.clone()
            }],
            TS,
            Some("promoter"),
            Some("reasoner:LIFT"),
        )
        .unwrap();
    store
        .transact(
            &[Datum {
                op: Op::Retract,
                ..d
            }],
            TS,
            None,
            Some("base"),
        )
        .unwrap();
    quipu::reasoner::evaluate(&mut store, &rules, TS).unwrap();
    let expected =
        quipu::tool_ask(&store, &serde_json::json!({"name":"unsupported_demotions"})).unwrap();
    assert_eq!(expected["count"], 1);
    #[cfg(feature = "shacl")]
    {
        let (turtle, _) = quipu::rdf::export_rdf_subset(
            &store,
            oxrdfio::RdfFormat::Turtle,
            Some(quipu::store::inferred::ROOT_INFERRED_GRAPH_IRI),
        )
        .unwrap();
        let report = quipu::validate_shapes(
            include_str!("../shapes/demoted-derivation.ttl"),
            std::str::from_utf8(&turtle).unwrap(),
        )
        .unwrap();
        assert!(
            report.conforms,
            "runtime output must obey the shipped schema: {report:?}"
        );
    }
    drop(store);
    assert_eq!(list(dir.path(), &db)["rows"], expected["rows"]);
}
