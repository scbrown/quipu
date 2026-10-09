//! Differential COUNT controls against the retained materializing evaluator.
use super::super::{Bindings, TemporalContext};
use crate::{Store, types::Value};
use oxrdfio::RdfFormat;
use spargebra::{
    Query,
    algebra::{AggregateExpression, GraphPattern},
    term::Variable,
};
const TS: &str = "2026-10-08T00:00:00Z";
const PREFIX: &str = "PREFIX ex: <http://example.org/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> ";

fn seed(store: &mut Store, graph: i64, triples: &str) {
    crate::rdf::ingest_rdf_to_graph(store, format!("@prefix ex: <http://example.org/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> . @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . {triples}").as_bytes(), RdfFormat::Turtle, None, TS, None, None, graph).unwrap();
}
fn fixture() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    seed(
        &mut store,
        0,
        r#"ex:Sub rdfs:subClassOf ex:Issue . ex:inferred a ex:Sub ; ex:id "sub" . ex:both a ex:Issue, ex:Sub ; ex:id "both" . ex:f ex:n 1, 1.0, -0.0, 0.0 . ex:equal ex:equal ex:equal ."#,
    );
    let graph = store.graph_create("http://example.org/board").unwrap();
    seed(
        &mut store,
        graph,
        r#"
        ex:a a ex:Issue ; ex:id "same" ; ex:status "open" .
        ex:alias a ex:Issue ; ex:id "same" ; ex:status "deferred" .
        ex:b a ex:Issue ; ex:id "other" .
        ex:lang a ex:Issue ; ex:id "lang" ; ex:status "closed"@en .
        ex:tomb a ex:Issue ; ex:id "tomb" ; ex:status "tombstone" .
        ex:numeric a ex:Issue ; ex:id 42 ; ex:status "open" .
        ex:foreign ex:id "foreign" ; ex:status "open" .
        ex:untyped ex:id "untyped" . _:blank a ex:Issue ; ex:id "blank" .
    "#,
    );
    let graph = store.graph_create("http://example.org/ephemeral").unwrap();
    seed(
        &mut store,
        graph,
        r#"ex:shadow a ex:Issue ; ex:id "same" ; ex:status "closed" . ex:foreign a ex:Other ; ex:id "other" ."#,
    );
    store
}
fn group(
    pattern: &GraphPattern,
) -> (
    &GraphPattern,
    &[Variable],
    &[(Variable, AggregateExpression)],
) {
    match pattern {
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => (inner, variables, aggregates),
        GraphPattern::Project { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::Filter { inner, .. } => group(inner),
        _ => panic!("expected group: {pattern:?}"),
    }
}
fn compare(store: &Store, query: &str, ctx: &TemporalContext, applicable: bool) {
    let Query::Select { pattern, .. } = super::super::sparql_parser()
        .parse_query(&format!("{PREFIX}{query}"))
        .unwrap()
    else {
        panic!()
    };
    let (inner, variables, aggregates) = group(&pattern);
    let seed = Bindings::new();
    let old = super::super::group::evaluate(
        store,
        super::super::pattern::eval_pattern_seeded(store, inner, ctx, &seed)
            .unwrap()
            .0,
        variables,
        aggregates,
    );
    let new = super::try_evaluate(store, inner, variables, aggregates, ctx, &seed).unwrap();
    if applicable {
        assert_eq!(new.unwrap(), old, "{query}");
    } else {
        assert!(new.is_none(), "{query}");
    }
}

#[test]
fn counts_preserve_terms_inference_duplicates_empty_and_groups() {
    let store = fixture();
    let ctx = TemporalContext::default();
    for q in [
        "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT (COUNT(?o) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT (COUNT(DISTINCT ?o) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s ?s ?s }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s a ex:Issue }",
        "SELECT ?o (COUNT(*) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?o",
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:missing { ?s ?p ?o } }",
        "SELECT ?o (COUNT(*) AS ?n) WHERE { GRAPH ex:missing { ?s ?p ?o } } GROUP BY ?o",
        "SELECT (COUNT(DISTINCT ?n) AS ?c) WHERE { ex:f ex:n ?n }",
        "SELECT ?status (COUNT(DISTINCT ?id) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id OPTIONAL { ?s ex:status ?status } } } GROUP BY ?status",
    ] {
        compare(&store, q, &ctx, !q.contains("OPTIONAL"));
    }
}

#[test]
fn union_shadow_status_defaults_and_plain_identifiers_match_exactly() {
    let store = fixture();
    let inner = r#"{ { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id ; ex:status ?status . FILTER(isLiteral(?status) && sameTerm(?status, STR(?status))) FILTER(?status != "tombstone") FILTER(isLiteral(?id) && sameTerm(?id, STR(?id))) } FILTER NOT EXISTS { GRAPH ex:ephemeral { ?shadow a ex:Issue ; ex:id ?id } } } UNION { GRAPH ex:ephemeral { ?s a ex:Issue ; ex:id ?id ; ex:status ?status . FILTER(isLiteral(?status) && sameTerm(?status, STR(?status))) FILTER(?status != "tombstone") FILTER(isLiteral(?id) && sameTerm(?id, STR(?id))) } } } UNION { { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id . FILTER NOT EXISTS { ?s ex:status ?raw FILTER(isLiteral(?raw) && sameTerm(?raw, STR(?raw))) } BIND("open" AS ?status) FILTER(isLiteral(?id) && sameTerm(?id, STR(?id))) } FILTER NOT EXISTS { GRAPH ex:ephemeral { ?shadow a ex:Issue ; ex:id ?id } } } UNION { GRAPH ex:ephemeral { ?s a ex:Issue ; ex:id ?id . FILTER NOT EXISTS { ?s ex:status ?raw FILTER(isLiteral(?raw) && sameTerm(?raw, STR(?raw))) } BIND("open" AS ?status) FILTER(isLiteral(?id) && sameTerm(?id, STR(?id))) } } }"#;
    for q in [
        format!("SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {inner} }}"),
        format!("SELECT ?status (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {inner} }} GROUP BY ?status"),
    ] {
        compare(&store, &q, &TemporalContext::default(), true);
    }
    let q = format!("{PREFIX}SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {inner} }}");
    assert_eq!(
        super::super::query(&store, &q).unwrap().rows()[0]["n"],
        Value::Int(4)
    );
}

#[test]
fn datasets_seeds_and_unsupported_forms_do_not_widen() {
    let store = fixture();
    let ctx = TemporalContext {
        named_dataset: Some(vec![]),
        ..TemporalContext::default()
    };
    compare(
        &store,
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s ?p ?o } }",
        &ctx,
        true,
    );
    let ctx = TemporalContext::default();
    for q in [
        "SELECT (SUM(?o) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT (COUNT(DISTINCT *) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT (COUNT(?o + 1) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } }",
        "SELECT (COUNT(*) AS ?n) WHERE { { SELECT ?s WHERE { ?s ?p ?o } LIMIT 2 } }",
    ] {
        compare(&store, q, &ctx, false);
    }
    let mut seed = Bindings::new();
    seed.insert(
        "s".into(),
        Value::Ref(store.lookup("http://example.org/foreign").unwrap().unwrap()),
    );
    let Query::Select { pattern, .. } = super::super::sparql_parser()
        .parse_query(&format!(
            "{PREFIX}SELECT (COUNT(*) AS ?n) WHERE {{ GRAPH ex:board {{ ?s ?p ?o }} }}"
        ))
        .unwrap()
    else {
        panic!()
    };
    let (inner, variables, aggregates) = group(&pattern);
    let old = super::super::group::evaluate(
        &store,
        super::super::pattern::eval_pattern_seeded(&store, inner, &ctx, &seed)
            .unwrap()
            .0,
        variables,
        aggregates,
    );
    assert_eq!(
        super::try_evaluate(&store, inner, variables, aggregates, &ctx, &seed)
            .unwrap()
            .unwrap(),
        old
    );
}

#[test]
fn counter_preserves_float_zero_and_nan_partial_equality() {
    let mut counter = super::Counter::default();
    counter.add(Value::Float(-0.0), true);
    counter.add(Value::Float(0.0), true);
    counter.add(Value::Float(f64::NAN), true);
    counter.add(Value::Float(f64::NAN), true);
    assert_eq!(counter.count, 3);
}

#[test]
fn temporal_default_union_and_scope_restoration_controls() {
    let store = fixture();
    let graph = store.lookup("http://example.org/board").unwrap().unwrap();
    let other = store
        .lookup("http://example.org/ephemeral")
        .unwrap()
        .unwrap();
    for ctx in [
        TemporalContext {
            valid_at: Some("2026-10-07T00:00:00Z".into()),
            ..TemporalContext::default()
        },
        TemporalContext {
            as_of_tx: Some(1),
            ..TemporalContext::default()
        },
        TemporalContext {
            graph: super::super::GraphScope::Default(vec![graph, other]),
            ..TemporalContext::default()
        },
    ] {
        compare(
            &store,
            "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }",
            &ctx,
            true,
        );
        compare(
            &store,
            "SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id } }",
            &ctx,
            true,
        );
    }
    assert!(!super::direct_refs());
    let Query::Select { pattern, .. } = super::super::sparql_parser()
        .parse_query(&format!(
            "{PREFIX}SELECT (COUNT(DISTINCT ?o) AS ?n) WHERE {{ ?s ?p ?o }}"
        ))
        .unwrap()
    else {
        panic!()
    };
    let (inner, variables, aggregates) = group(&pattern);
    let ctx = TemporalContext {
        row_cap: Some(1),
        ..TemporalContext::default()
    };
    assert!(
        super::try_evaluate(&store, inner, variables, aggregates, &ctx, &Bindings::new()).is_err()
    );
    assert!(
        !super::direct_refs(),
        "error must not leak the scoped reference fast path"
    );
}

#[test]
fn attached_graph_aliases_use_canonical_binding_and_dedup() {
    use crate::store::attach::Attachment;
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.db");
    let raw = dir.path().join("raw.db");
    let layer = dir.path().join("layer.db");
    for path in [&main, &raw] {
        let mut store = Store::open(path.to_str().unwrap()).unwrap();
        let graph = store.graph_create("http://example.org/board").unwrap();
        seed(&mut store, graph, r#"ex:a a ex:Issue ; ex:id "same" ."#);
    }
    crate::store::respace::respace_file(&raw, &layer, 4).unwrap();
    let store = Store::open_with_attachments(
        main.to_str().unwrap(),
        &[Attachment::read_only("layer", layer.to_str().unwrap())],
    )
    .unwrap();
    assert!(store.has_attachments());
    for q in [
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s ?p ?o } }",
        "SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id } }",
        "SELECT ?id (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id } } GROUP BY ?id",
    ] {
        compare(&store, q, &TemporalContext::default(), true);
    }
    assert!(!super::direct_refs());
}

#[test]
fn paired_mean_count_preserves_types_invalid_and_empty_inputs() {
    let mut store = fixture();
    seed(
        &mut store,
        0,
        r#"ex:date1 ex:created "2020-01-01T00:00:00Z" . ex:date2 ex:created "2024-03-01T00:00:00Z" . ex:datebad ex:created "not-a-date" ."#,
    );
    for q in [
        "SELECT (AVG(?n) AS ?mean) (COUNT(?n) AS ?n) WHERE { ex:f ex:n ?n }",
        "SELECT (AVG(DISTINCT ?n) AS ?mean) (COUNT(?n) AS ?n) WHERE { ex:f ex:n ?n }",
        "SELECT (AVG(?n) AS ?mean) (COUNT(?n) AS ?n) WHERE { ex:missing ex:n ?n }",
        "SELECT (AVG(YEAR(STRDT(STR(?created), xsd:dateTime))) AS ?mean) (COUNT(YEAR(STRDT(STR(?created), xsd:dateTime))) AS ?n) WHERE { ?s ex:created ?created }",
    ] {
        compare(&store, q, &TemporalContext::default(), true);
    }
    let positive=super::super::query(&store,&format!("{PREFIX}SELECT (AVG(YEAR(STRDT(STR(?created), xsd:dateTime))) AS ?mean) (COUNT(YEAR(STRDT(STR(?created), xsd:dateTime))) AS ?n) WHERE {{ ?s ex:created ?created }}")).unwrap();
    assert_eq!(positive.rows()[0]["n"], Value::Int(2));
    assert!(positive.rows()[0].contains_key("mean"));
    seed(&mut store, 0, r#"ex:f ex:n "not numeric" ."#);
    compare(
        &store,
        "SELECT (AVG(?n) AS ?mean) (COUNT(?n) AS ?n) WHERE { ex:f ex:n ?n }",
        &TemporalContext::default(),
        true,
    );
    for q in [
        "SELECT (AVG(?n) AS ?mean) WHERE { ex:f ex:n ?n }",
        "SELECT (AVG(RAND()) AS ?mean) (COUNT(?n) AS ?n) WHERE { ex:f ex:n ?n }",
    ] {
        compare(&store, q, &TemporalContext::default(), false);
    }
}

#[test]
fn covering_scalar_count_keeps_same_fact_currentness_and_missing_index_fallback() {
    let store = fixture();
    let graph = store.lookup("http://example.org/board").unwrap().unwrap();
    // One triple has two current physical assertions. Other rows of the same
    // subjects are retracted/closed: intersecting subjects instead of rowids
    // would wrongly make those noncurrent facts visible again.
    store
        .conn
        .execute(
            "INSERT INTO facts(e,a,v,g,tx,valid_from,valid_to,op)
         SELECT e,a,v,g,(SELECT MAX(id) FROM transactions),valid_from,valid_to,op
         FROM facts WHERE g=?1 LIMIT 1",
            [graph],
        )
        .unwrap();
    let status = store.lookup("http://example.org/status").unwrap().unwrap();
    let alias = store.lookup("http://example.org/alias").unwrap().unwrap();
    let tomb = store.lookup("http://example.org/tomb").unwrap().unwrap();
    store
        .conn
        .execute(
            "UPDATE facts SET valid_to='2026-10-07T00:00:00Z' WHERE e=?1 AND a=?2 AND g=?3",
            [alias, status, graph],
        )
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE facts SET op=0 WHERE e=?1 AND a=?2 AND g=?3",
            [tomb, status, graph],
        )
        .unwrap();
    for q in [
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s ?p ?o } }",
        "SELECT (COUNT(?id) AS ?n) WHERE { GRAPH ex:board { ?s ex:id ?id } }",
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:missing { ?s ?p ?o } }",
    ] {
        compare(&store, q, &TemporalContext::default(), true);
    }
    store.conn.execute("DROP INDEX idx_current_g", []).unwrap();
    compare(
        &store,
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s ?p ?o } }",
        &TemporalContext::default(),
        true,
    );
}

#[test]
fn fresh_property_optional_preserves_numeric_default_bags_and_group_order() {
    let mut store = fixture();
    let graph = store.lookup("http://example.org/board").unwrap().unwrap();
    seed(
        &mut store,
        graph,
        r#"ex:a ex:priority 1, 2 . ex:alias ex:priority 3 . ex:lang ex:priority -1 . ex:tomb ex:priority "2" . ex:b ex:priority 0.5 . ex:numeric ex:priority 256 . _:extra a ex:Issue ; ex:id "extra" ; ex:priority 0 ."#,
    );
    let body = r#"GRAPH ex:board { ?s a ex:Issue ; ex:id ?id OPTIONAL { ?s ex:priority ?raw FILTER(sameTerm(?raw - ?raw,0) && sameTerm(?raw,?raw+0) && ?raw>=0 && ?raw<=255) } BIND(COALESCE(?raw,2) AS ?priority) FILTER(isLiteral(?id) && sameTerm(?id,STR(?id))) }"#;
    for q in [
        format!("SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {body} }}"),
        format!(
            "SELECT ?priority (COUNT(*) AS ?n) (COUNT(DISTINCT ?id) AS ?distinct) WHERE {{ {body} }} GROUP BY ?priority"
        ),
        format!("SELECT (COUNT(?raw) AS ?n) WHERE {{ {body} }}"),
    ] {
        compare(&store, &q, &TemporalContext::default(), true);
    }
    let q = format!("{PREFIX}SELECT (COUNT(?raw) AS ?n) WHERE {{ {body} }}");
    let positive = super::super::query(&store, &q).unwrap();
    assert_eq!(
        positive.rows()[0]["n"],
        Value::Int(4),
        "two valid priorities plus alias and blank subject; invalids leave OPTIONAL unbound"
    );
    // OPTIONAL succeeds several times or not at all; COALESCE defaults only
    // unmatched parents, and a shared external right object forbids seeding.
    for q in [
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue ; ex:id ?id OPTIONAL { ?s ex:priority ?raw FILTER(?id=\"same\") } } }",
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue ; ex:id ?raw OPTIONAL { ?s ex:priority ?raw } } }",
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?s a ex:Issue OPTIONAL { ?s ex:priority ?raw . ?s ex:id ?id } } }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s a ex:Issue OPTIONAL { ?s ex:priority ?raw } }",
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ex:board { ?x ex:id ?s OPTIONAL { ?s ex:priority ?raw FILTER(isNumeric(?raw)) } } }",
    ] {
        compare(&store, q, &TemporalContext::default(), false);
    }
    for ctx in [
        TemporalContext {
            valid_at: Some(TS.into()),
            ..TemporalContext::default()
        },
        TemporalContext {
            as_of_tx: Some(1),
            ..TemporalContext::default()
        },
    ] {
        compare(
            &store,
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ {body} }}"),
            &ctx,
            false,
        );
    }
}

#[test]
fn fresh_optional_local_string_filters_preserve_principals_and_default() {
    let mut store = fixture();
    let graph = store.lookup("http://example.org/board").unwrap().unwrap();
    seed(
        &mut store,
        graph,
        r#"ex:a ex:owner <http://example.org/principal/alice> . ex:alias ex:owner "alice" . ex:b ex:owner <http://example.org/foreign> . ex:lang ex:owner "alice"@en . ex:tomb ex:owner "" . ex:numeric ex:owner 7 ."#,
    );
    let body = r#"GRAPH ex:board { ?s a ex:Issue ; ex:id ?id OPTIONAL { ?s ex:owner ?raw FILTER((isIRI(?raw) && STRSTARTS(STR(?raw),"http://example.org/principal/")) || (isLiteral(?raw) && sameTerm(?raw,STR(?raw)))) } BIND(COALESCE(STR(?raw),"") AS ?owner) }"#;
    for q in [
        format!("SELECT ?owner (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {body} }} GROUP BY ?owner"),
        format!("SELECT (COUNT(?raw) AS ?n) WHERE {{ {body} }}"),
    ] {
        compare(&store, &q, &TemporalContext::default(), true);
    }
    let q = format!("{PREFIX}SELECT (COUNT(?raw) AS ?n) WHERE {{ {body} }}");
    assert_eq!(
        super::super::query(&store, &q).unwrap().rows()[0]["n"],
        Value::Int(3),
        "principalIRI/plain/empty pass; foreignIRI/lang/numeric fail the local guard"
    );
    for guard in [
        "RAND()>0.5",
        "NOW()=NOW()",
        "STRUUID()=STRUUID()",
        "BOUND(?id)",
        "EXISTS { ?s ex:id ?id }",
    ] {
        compare(
            &store,
            &format!(
                "SELECT (COUNT(*) AS ?n) WHERE {{ GRAPH ex:board {{ ?s a ex:Issue OPTIONAL {{ ?s ex:owner ?raw FILTER({guard}) }} }} }}"
            ),
            &TemporalContext::default(),
            false,
        );
    }
}

#[test]
fn optional_unknown_custom_filter_preserves_original_error_path() {
    let store = fixture();
    let text = format!(
        "{PREFIX}SELECT (COUNT(*) AS ?n) WHERE {{ GRAPH ex:board {{ ?s a ex:Issue OPTIONAL {{ ?s ex:id ?raw FILTER(<http://example.org/custom>(?raw)) }} }} }}"
    );
    let Query::Select { pattern, .. } = super::super::sparql_parser().parse_query(&text).unwrap()
    else {
        panic!()
    };
    let (inner, variables, aggregates) = group(&pattern);
    assert!(
        super::try_evaluate(
            &store,
            inner,
            variables,
            aggregates,
            &TemporalContext::default(),
            &Bindings::new()
        )
        .unwrap()
        .is_none()
    );
    assert!(
        super::super::query(&store, &text)
            .unwrap_err()
            .to_string()
            .contains("unsupported FILTER function")
    );
}

#[test]
fn two_fresh_optional_plain_fields_preserve_preference_empty_and_multiplicity() {
    let mut store = fixture();
    let graph = store.lookup("http://example.org/board").unwrap().unwrap();
    seed(
        &mut store,
        graph,
        r#"ex:a ex:name "preferred" ; rdfs:label "fallback" . ex:alias rdfs:label "fallback" . ex:b ex:name "ignored"@en ; rdfs:label "fallback" . ex:lang ex:name 1 ; rdfs:label "fallthrough" . ex:tomb ex:name "" ; rdfs:label "fallback" . ex:numeric ex:name "one", "two" ; rdfs:label "y", "z" ."#,
    );
    let body = r#"GRAPH ex:board { ?s a ex:Issue ; ex:id ?id OPTIONAL { ?s ex:name ?name FILTER(isLiteral(?name) && sameTerm(?name,STR(?name))) } OPTIONAL { ?s rdfs:label ?label FILTER(isLiteral(?label) && sameTerm(?label,STR(?label))) } BIND(COALESCE(?name,?label,"") AS ?title) }"#;
    for q in [
        format!(
            "SELECT ?title (COUNT(*) AS ?n) (COUNT(DISTINCT ?id) AS ?ids) WHERE {{ {body} }} GROUP BY ?title"
        ),
        format!(
            "SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {body} FILTER(CONTAINS(?title,\"fallback\")) }}"
        ),
    ] {
        compare(&store, &q, &TemporalContext::default(), true);
    }
    let q = format!(
        "{PREFIX}SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {body} FILTER(CONTAINS(?title,\"fallback\")) }}"
    );
    assert_eq!(
        super::super::query(&store, &q).unwrap().rows()[0]["n"],
        Value::Int(2),
        "rdfs-only and ignored-language name use fallback; preferred/empty names do not"
    );
}
