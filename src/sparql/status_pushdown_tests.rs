//! Differential controls for the narrow plain-property absence scan.

use oxrdfio::RdfFormat;
use spargebra::algebra::{Expression, Function, GraphPattern};
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};

use super::filter::eval_filter;
use super::pattern::eval_pattern_seeded;
use super::pattern_util::Bindings;
use super::status_pushdown::{candidates, constant_candidates};
use super::{GraphScope, TemporalContext, query};
use crate::rdf::ingest_rdf_to_graph;
use crate::store::Store;
use crate::types::Value;

const EX: &str = "http://example.org/";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn node(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn var(s: &str) -> Variable {
    Variable::new(s).unwrap()
}
fn expression_var(s: &str) -> Expression {
    Expression::Variable(var(s))
}

fn shape() -> (Expression, GraphPattern) {
    let typed = GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::Variable(var("s")),
            predicate: NamedNodePattern::NamedNode(node(RDF_TYPE)),
            object: TermPattern::NamedNode(node(&format!("{EX}T"))),
        }],
    };
    let property = GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::Variable(var("s")),
            predicate: NamedNodePattern::NamedNode(node(&format!("{EX}status"))),
            object: TermPattern::Variable(var("v")),
        }],
    };
    let guard = Expression::And(
        Box::new(Expression::FunctionCall(
            Function::IsLiteral,
            vec![expression_var("v")],
        )),
        Box::new(Expression::SameTerm(
            Box::new(expression_var("v")),
            Box::new(Expression::FunctionCall(
                Function::Str,
                vec![expression_var("v")],
            )),
        )),
    );
    (
        Expression::Not(Box::new(Expression::Exists(Box::new(
            GraphPattern::Filter {
                expr: guard,
                inner: Box::new(property),
            },
        )))),
        typed,
    )
}

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    let mut data = String::new();
    for name in [
        "absent", "closed", "open", "lang", "number", "ref", "mixed", "dup1", "dup2",
    ] {
        data.push_str(&format!("<{EX}{name}> <{RDF_TYPE}> <{EX}T> <{EX}g> .\n"));
    }
    data.push_str(&format!(
        "<{EX}closed> <{EX}status> \"closed\" <{EX}g> .\n\
         <{EX}open> <{EX}status> \"open\" <{EX}g> .\n\
         <{EX}lang> <{EX}status> \"closed\"@en <{EX}g> .\n\
         <{EX}number> <{EX}status> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> <{EX}g> .\n\
         <{EX}ref> <{EX}status> <{EX}closed> <{EX}g> .\n\
         <{EX}mixed> <{EX}status> \"closed\" <{EX}g> .\n\
         <{EX}mixed> <{EX}status> \"closed\"@fr <{EX}g> .\n\
         <{EX}dup1> <{EX}id> \"duplicate-id\" <{EX}g> .\n\
         <{EX}dup2> <{EX}id> \"duplicate-id\" <{EX}g> .\n\
         <{EX}foreign> <{RDF_TYPE}> <{EX}Other> <{EX}g> .\n\
         <{EX}untyped> <{EX}id> \"untyped-id\" <{EX}g> .\n\
         <{EX}crossgraph> <{RDF_TYPE}> <{EX}T> <{EX}other> .\n\
         <{EX}crossgraph> <{EX}lookalike> <{EX}T> <{EX}g> .\n\
         <{EX}absent> <{EX}status> \"closed\" <{EX}other> .\n\
         <{EX}Sub> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <{EX}T> .\n\
         <{EX}sub> <{RDF_TYPE}> <{EX}Sub> .\n\
         <{EX}root_status> <{EX}status> \"closed\" .\n"
    ));
    for graph in ["g", "other", "root"] {
        let suffix = format!(" <{EX}{graph}> .");
        let mut triples = String::new();
        for line in data.lines() {
            if let Some(triple) = line.strip_suffix(&suffix) {
                triples.push_str(triple);
                triples.push_str(" .\n");
            } else if graph == "root"
                && !line.ends_with(&format!(" <{EX}g> ."))
                && !line.ends_with(&format!(" <{EX}other> ."))
            {
                triples.push_str(line);
                triples.push('\n');
            }
        }
        let gid = if graph == "root" {
            0
        } else {
            s.intern(&format!("{EX}{graph}")).unwrap()
        };
        ingest_rdf_to_graph(
            &mut s,
            triples.as_bytes(),
            RdfFormat::NTriples,
            None,
            "2026-01-01T00:00:00Z",
            None,
            None,
            gid,
        )
        .unwrap();
    }
    s
}

fn accepted(
    s: &Store,
    rows: Vec<Bindings>,
    expr: &Expression,
    ctx: &TemporalContext,
) -> Vec<String> {
    let mut ids = rows
        .into_iter()
        .filter(|r| eval_filter(s, expr, r, ctx).unwrap())
        .map(|r| match r.get("s").unwrap() {
            Value::Ref(id) => s.resolve(*id).unwrap(),
            other => panic!("unexpected subject {other:?}"),
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[test]
fn status_pushdown_preserves_nonempty_rdf_identity_and_graph_isolation() {
    let s = store();
    let ctx = TemporalContext {
        graph: GraphScope::Named(vec![s.lookup(&format!("{EX}g")).unwrap().unwrap()]),
        ..TemporalContext::default()
    };
    let (expr, inner) = shape();
    let (optimized, vars) = candidates(&s, &expr, &inner, &ctx, &Bindings::new())
        .unwrap()
        .unwrap();
    assert_eq!(vars, vec!["s"]);
    let (original, _) = eval_pattern_seeded(&s, &inner, &ctx, &Bindings::new()).unwrap();
    assert!(
        optimized.len() < original.len(),
        "the optimization never narrowed"
    );
    let actual = accepted(&s, optimized, &expr, &ctx);
    assert_eq!(actual, accepted(&s, original, &expr, &ctx));
    assert_eq!(
        actual,
        ["absent", "dup1", "dup2", "lang", "number", "ref"].map(|name| format!("{EX}{name}"))
    );
    let result = query(&s, &format!("SELECT ?s WHERE {{ GRAPH <{EX}g> {{ ?s a <{EX}T> FILTER NOT EXISTS {{ ?s <{EX}status> ?v FILTER(isLiteral(?v) && sameTerm(?v, STR(?v))) }} }} }} ORDER BY STR(?s)" )).unwrap();
    assert_eq!(result.rows().len(), actual.len());
}

#[test]
fn status_pushdown_keeps_root_inference_and_historical_seeded_or_other_shapes_on_fallback() {
    let s = store();
    let (expr, inner) = shape();
    let seed = Bindings::new();
    assert!(
        candidates(&s, &expr, &inner, &TemporalContext::default(), &seed)
            .unwrap()
            .is_none()
    );
    let inferred = query(&s, &format!("SELECT ?s WHERE {{ ?s a <{EX}T> }}")).unwrap();
    assert!(
        !inferred.rows().is_empty(),
        "root subclass positive control failed"
    );
    let named = TemporalContext {
        graph: GraphScope::Named(vec![s.lookup(&format!("{EX}g")).unwrap().unwrap()]),
        ..TemporalContext::default()
    };
    for ctx in [
        TemporalContext {
            valid_at: Some("2026-01-02T00:00:00Z".into()),
            ..named.clone()
        },
        TemporalContext {
            as_of_tx: Some(1),
            ..named.clone()
        },
        TemporalContext {
            row_limit: Some(1),
            ..named.clone()
        },
    ] {
        assert!(
            candidates(&s, &expr, &inner, &ctx, &seed)
                .unwrap()
                .is_none()
        );
    }
    let mut bound = Bindings::new();
    bound.insert(
        "s".into(),
        Value::Ref(s.lookup(&format!("{EX}absent")).unwrap().unwrap()),
    );
    assert!(
        candidates(&s, &expr, &inner, &named, &bound)
            .unwrap()
            .is_none()
    );
    assert!(
        candidates(
            &s,
            &Expression::Not(Box::new(expr.clone())),
            &inner,
            &named,
            &seed
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn status_pushdown_composed_aliases_use_exact_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main.db");
    let layer = temp.path().join("layer.db");
    for (path, is_main) in [(&main, true), (&layer, false)] {
        let mut s = Store::open(&path.to_string_lossy()).unwrap();
        s.intern(if is_main {
            "urn:main:padding"
        } else {
            "urn:layer:padding"
        })
        .unwrap();
        let graph = s.intern(&format!("{EX}g")).unwrap();
        let data = if is_main {
            format!("<{EX}shared> <{RDF_TYPE}> <{EX}T> .\n<{EX}shared> <{EX}status> \"closed\" .")
        } else {
            format!(
                "<{EX}shared> <{RDF_TYPE}> <{EX}T> .\n<{EX}layer_absent> <{RDF_TYPE}> <{EX}T> ."
            )
        };
        ingest_rdf_to_graph(
            &mut s,
            data.as_bytes(),
            RdfFormat::NTriples,
            None,
            "2026-01-01T00:00:00Z",
            None,
            None,
            graph,
        )
        .unwrap();
    }
    let remapped = temp.path().join("layer-space31.db");
    crate::store::respace::respace_file(&layer, &remapped, 31).unwrap();
    let s = Store::open_with_attachments(
        &main.to_string_lossy(),
        &[crate::store::attach::Attachment::read_only(
            "layer",
            &remapped.to_string_lossy(),
        )],
    )
    .unwrap();
    assert_eq!(
        s.lookup_all(&format!("{EX}shared")).unwrap().len(),
        2,
        "alias control did not cover both term spaces"
    );
    let ctx = TemporalContext {
        graph: GraphScope::Named(s.lookup_all(&format!("{EX}g")).unwrap()),
        ..TemporalContext::default()
    };
    let constant = [TriplePattern {
        subject: TermPattern::Variable(var("s")),
        predicate: NamedNodePattern::NamedNode(node(RDF_TYPE)),
        object: TermPattern::NamedNode(node(&format!("{EX}T"))),
    }];
    assert!(
        constant_candidates(&s, &constant, &ctx, &Bindings::new())
            .unwrap()
            .is_none()
    );
    let (expr, inner) = shape();
    assert!(
        candidates(&s, &expr, &inner, &ctx, &Bindings::new())
            .unwrap()
            .is_none()
    );
    let (original, _) = eval_pattern_seeded(&s, &inner, &ctx, &Bindings::new()).unwrap();
    assert_eq!(
        accepted(&s, original, &expr, &ctx),
        vec![format!("{EX}layer_absent")]
    );
}

#[test]
fn status_pushdown_dataset_restrictions_empty_graph_and_missing_index_keep_exactness() {
    let s = store();
    let body = format!(
        "GRAPH <{EX}g> {{ ?s a <{EX}T> FILTER NOT EXISTS {{ ?s <{EX}status> ?v FILTER(isLiteral(?v) && sameTerm(?v, STR(?v))) }} }}"
    );
    let admitted = query(
        &s,
        &format!("SELECT ?s FROM NAMED <{EX}g> WHERE {{ {body} }}"),
    )
    .unwrap();
    assert_eq!(
        admitted.rows().len(),
        6,
        "nonempty admitted dataset control"
    );
    let excluded = query(
        &s,
        &format!("SELECT ?s FROM NAMED <{EX}other> WHERE {{ {body} }}"),
    )
    .unwrap();
    assert!(excluded.rows().is_empty());
    let empty = query(&s, &format!("SELECT ?s WHERE {{ GRAPH <{EX}not-interned> {{ ?s a <{EX}T> FILTER NOT EXISTS {{ ?s <{EX}status> ?v FILTER(isLiteral(?v) && sameTerm(?v, STR(?v))) }} }} }}")).unwrap();
    assert!(empty.rows().is_empty());
    for index in ["idx_active_vge", "idx_current_aev", "idx_current_g"] {
        let s = store();
        s.conn.execute(&format!("DROP INDEX {index}"), []).unwrap();
        let ctx = TemporalContext {
            graph: GraphScope::Named(vec![s.lookup(&format!("{EX}g")).unwrap().unwrap()]),
            ..TemporalContext::default()
        };
        let (expr, inner) = shape();
        assert!(
            candidates(&s, &expr, &inner, &ctx, &Bindings::new())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            query(&s, &format!("SELECT ?s WHERE {{ {body} }}"))
                .unwrap()
                .rows()
                .len(),
            6
        );
    }
}

fn subject_names(store: &Store, rows: Vec<Bindings>) -> Vec<String> {
    let mut names = rows
        .into_iter()
        .map(|row| match row.get("s").unwrap() {
            Value::Ref(id) => store.resolve(*id).unwrap(),
            value => panic!("unexpected subject {value:?}"),
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn constant_status_projection_preserves_rdf_terms_and_same_fact_identity() {
    let s = store();
    let ctx = TemporalContext {
        graph: GraphScope::Named(vec![s.lookup(&format!("{EX}g")).unwrap().unwrap()]),
        ..TemporalContext::default()
    };
    let cases = [
        (
            format!("{EX}status"),
            TermPattern::Literal(spargebra::term::Literal::new_simple_literal("closed")),
            2,
        ),
        (
            format!("{EX}status"),
            TermPattern::Literal(
                spargebra::term::Literal::new_language_tagged_literal("closed", "en").unwrap(),
            ),
            1,
        ),
        (
            format!("{EX}status"),
            TermPattern::Literal(spargebra::term::Literal::new_typed_literal(
                "7",
                node("http://www.w3.org/2001/XMLSchema#integer"),
            )),
            1,
        ),
        (
            format!("{EX}status"),
            TermPattern::Literal(spargebra::term::Literal::new_simple_literal("7")),
            0,
        ),
        (
            format!("{EX}status"),
            TermPattern::NamedNode(node(&format!("{EX}closed"))),
            1,
        ),
        (
            RDF_TYPE.into(),
            TermPattern::NamedNode(node(&format!("{EX}T"))),
            9,
        ),
    ];
    for (predicate, object, expected) in cases {
        let patterns = [TriplePattern {
            subject: TermPattern::Variable(var("s")),
            predicate: NamedNodePattern::NamedNode(node(&predicate)),
            object,
        }];
        let (projected, _) = constant_candidates(&s, &patterns, &ctx, &Bindings::new())
            .unwrap()
            .unwrap();
        let (original, _) = super::triple::eval_bgp(&s, &patterns, &ctx, &Bindings::new()).unwrap();
        let projected = subject_names(&s, projected);
        assert_eq!(
            projected.len(),
            expected,
            "nonempty or term-distinction control"
        );
        assert_eq!(projected, subject_names(&s, original));
    }
}

#[test]
fn constant_status_projection_retains_unsupported_contexts_and_missing_index_fallback() {
    let patterns = [TriplePattern {
        subject: TermPattern::Variable(var("s")),
        predicate: NamedNodePattern::NamedNode(node(&format!("{EX}status"))),
        object: TermPattern::Literal(spargebra::term::Literal::new_simple_literal("closed")),
    }];
    let s = store();
    let current = TemporalContext {
        graph: GraphScope::Named(vec![s.lookup(&format!("{EX}g")).unwrap().unwrap()]),
        ..TemporalContext::default()
    };
    let (positive, _) = constant_candidates(&s, &patterns, &current, &Bindings::new())
        .unwrap()
        .unwrap();
    assert_eq!(positive.len(), 2);
    let mut historical = current.clone();
    historical.valid_at = Some("2026-01-01T00:00:00Z".into());
    let mut as_of = current.clone();
    as_of.as_of_tx = Some(1);
    let mut limited = current.clone();
    limited.row_limit = Some(1);
    let mut narrowed = current.clone();
    narrowed.string_narrows = Some(Default::default());
    let mut reasoning = current.clone();
    reasoning.entails_rdfs = true;
    for context in [
        TemporalContext::default(),
        historical,
        as_of,
        limited,
        narrowed,
        reasoning,
    ] {
        assert!(
            constant_candidates(&s, &patterns, &context, &Bindings::new())
                .unwrap()
                .is_none()
        );
    }
    let seed = Bindings::from([("other".into(), Value::Str("bound".into()))]);
    assert!(
        constant_candidates(&s, &patterns, &current, &seed)
            .unwrap()
            .is_none()
    );
    let mut variable_object = patterns.clone();
    variable_object[0].object = TermPattern::Variable(var("v"));
    assert!(
        constant_candidates(&s, &variable_object, &current, &Bindings::new())
            .unwrap()
            .is_none()
    );
    for index in ["idx_active_vge", "idx_current_aev"] {
        let s = store();
        let current = TemporalContext {
            graph: GraphScope::Named(vec![s.lookup(&format!("{EX}g")).unwrap().unwrap()]),
            ..TemporalContext::default()
        };
        s.conn.execute(&format!("DROP INDEX {index}"), []).unwrap();
        assert!(
            constant_candidates(&s, &patterns, &current, &Bindings::new())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            super::triple::eval_bgp(&s, &patterns, &current, &Bindings::new())
                .unwrap()
                .0
                .len(),
            2
        );
    }
}
