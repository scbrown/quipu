//! Tests for the reasoner Turtle rule parser.
//!
//! Split out of `parse.rs` to keep that file under the repo's per-file line
//! budget. The tests exercise both the low-level Datalog-surface helpers
//! (`split_top_level_commas`, `parse_term`, `parse_atom`, `parse_body`) and
//! the full `parse_rules` Turtle → `RuleSet` path.

use super::ReasonerError;
use super::ast::{BodyAtom, Term};
use super::parse::{parse_atom, parse_body, parse_rules, parse_term, split_top_level_commas};

const PREFIX: &str = "http://ex/";

// ── raw parser helpers ─────────────────────────────────────

#[test]
fn split_respects_parens_and_quotes() {
    let chunks = split_top_level_commas("p(?x, ?y), q(?y, \"a,b\"), r(?x)");
    assert_eq!(
        chunks,
        vec![
            "p(?x, ?y)".to_string(),
            "q(?y, \"a,b\")".to_string(),
            "r(?x)".to_string(),
        ]
    );
}

#[test]
fn parse_term_recognises_variables() {
    let t = parse_term("?pkg", PREFIX).unwrap();
    assert_eq!(t, Term::Var("pkg".to_string()));
}

#[test]
fn parse_term_recognises_iri_brackets() {
    let t = parse_term("<http://example.org/foo>", PREFIX).unwrap();
    assert_eq!(t, Term::Iri("http://example.org/foo".to_string()));
}

#[test]
fn parse_term_recognises_string_literal() {
    let t = parse_term("\"hello\"", PREFIX).unwrap();
    assert_eq!(t, Term::Str("hello".to_string()));
}

#[test]
fn parse_term_expands_bare_identifier() {
    let t = parse_term("nginx", PREFIX).unwrap();
    assert_eq!(t, Term::Iri("http://ex/nginx".to_string()));
}

#[test]
fn parse_term_rejects_bare_question_mark() {
    let err = parse_term("?", PREFIX).unwrap_err();
    assert!(err.contains("variable name"));
}

#[test]
fn parse_atom_resolves_prefix() {
    let a = parse_atom("installedIn(?pkg, ?c)", PREFIX).unwrap();
    assert_eq!(a.predicate, "http://ex/installedIn");
    assert_eq!(a.args, vec![Term::Var("pkg".into()), Term::Var("c".into())]);
}

#[test]
fn parse_atom_rejects_missing_paren() {
    let err = parse_atom("installedIn ?pkg ?c", PREFIX).unwrap_err();
    assert!(err.contains("expected `(`"));
}

#[test]
fn parse_body_handles_multiple_atoms() {
    let body = parse_body("p(?x, ?y), q(?y, ?z)", PREFIX).unwrap();
    assert_eq!(body.len(), 2);
    assert!(body[0].is_positive());
    assert!(body[1].is_positive());
    assert_eq!(body[0].atom().predicate, "http://ex/p");
    assert_eq!(body[1].atom().predicate, "http://ex/q");
}

#[test]
fn parse_body_handles_negation() {
    let body = parse_body("p(?x), not q(?x)", PREFIX).unwrap();
    assert!(body[0].is_positive());
    assert!(matches!(body[1], BodyAtom::Negative(_)));
    assert_eq!(body[1].atom().predicate, "http://ex/q");
}

// ── Turtle → RuleSet ────────────────────────────────────────

const SIMPLE_TTL: &str = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix : <http://aegis.gastown.local/ontology/> .

:affectsDirect a rule:Rule ;
    rule:id "R1" ;
    rule:head "affects(?pkg, ?svc)" ;
    rule:body "installedIn(?pkg, ?c), runsService(?c, ?svc)" .

:affectsTransitive a rule:Rule ;
    rule:id "R2" ;
    rule:head "affects(?pkg, ?svc)" ;
    rule:body "affects(?pkg, ?x), dependsOn(?x, ?svc)" .
"#;

#[test]
fn parse_rules_extracts_all_rule_resources() {
    let rs = parse_rules(SIMPLE_TTL, None).unwrap();
    assert_eq!(rs.rules.len(), 2);
    let ids: Vec<&str> = rs.rules.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"R1"));
    assert!(ids.contains(&"R2"));
}

#[test]
fn parse_rules_expands_default_prefix() {
    let rs = parse_rules(SIMPLE_TTL, None).unwrap();
    let r1 = rs.rules.iter().find(|r| r.id == "R1").unwrap();
    assert_eq!(
        r1.head.predicate,
        "http://aegis.gastown.local/ontology/affects"
    );
    assert_eq!(
        r1.body[0].atom().predicate,
        "http://aegis.gastown.local/ontology/installedIn"
    );
}

#[test]
fn parse_rules_honours_fallback_prefix_arg() {
    let ttl = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x)" ;
    rule:body "p(?x)" .
"#;
    let rs = parse_rules(ttl, Some("http://custom/")).unwrap();
    assert_eq!(rs.rules[0].head.predicate, "http://custom/h");
}

#[test]
fn parse_rules_honours_per_rule_prefix_override() {
    let ttl = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:prefix "http://custom/" ;
    rule:head "h(?x)" ;
    rule:body "p(?x)" .
"#;
    let rs = parse_rules(ttl, None).unwrap();
    assert_eq!(rs.rules[0].head.predicate, "http://custom/h");
    assert_eq!(rs.rules[0].body[0].atom().predicate, "http://custom/p");
}

#[test]
fn parse_rules_rejects_missing_head() {
    let ttl = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:body "p(?x)" .
"#;
    let err = parse_rules(ttl, None).unwrap_err();
    match err {
        ReasonerError::MissingProperty { property, .. } => {
            assert_eq!(property, "rule:head");
        }
        other => panic!("expected MissingProperty, got {other:?}"),
    }
}

#[test]
fn parse_rules_rejects_unbound_head_variable() {
    let ttl = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x)" .
"#;
    let err = parse_rules(ttl, None).unwrap_err();
    match err {
        ReasonerError::UnboundHeadVariable { variable, .. } => {
            assert_eq!(variable, "y");
        }
        other => panic!("expected UnboundHeadVariable, got {other:?}"),
    }
}

#[test]
fn parse_rules_rejects_variable_only_under_negation_in_head() {
    // The head variable `?y` only appears under negation in the body,
    // which is not a safe range restriction — reject it.
    let ttl = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x), not q(?y)" .
"#;
    let err = parse_rules(ttl, None).unwrap_err();
    assert!(matches!(err, ReasonerError::UnboundHeadVariable { .. }));
}

#[test]
fn parse_rules_honours_ruleset_default_prefix() {
    let ttl = r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix ex: <http://example.org/rules/> .

ex:set a rule:RuleSet ;
    rule:defaultPrefix "http://aegis/" .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x)" ;
    rule:body "p(?x)" .
"#;
    let rs = parse_rules(ttl, None).unwrap();
    assert_eq!(rs.default_prefix, "http://aegis/");
    assert_eq!(rs.rules[0].head.predicate, "http://aegis/h");
}

#[test]
fn parse_rules_handles_empty_source() {
    let rs = parse_rules("", None).unwrap();
    assert!(rs.is_empty());
    assert_eq!(rs.len(), 0);
}
