use super::*;
fn atom(s: &str) -> Expr {
    Expr::Atom(s.into())
}
#[test]
fn precedence_and_implicit_and() {
    assert_eq!(
        parse("a OR b c").unwrap(),
        Expr::Or(
            Box::new(atom("a")),
            Box::new(Expr::And(Box::new(atom("b")), Box::new(atom("c"))))
        )
    );
    assert_eq!(parse("a b").unwrap(), parse("a AND b").unwrap());
}
#[test]
fn fields_phrases_ranges_prefixes_remain_atoms() {
    assert_eq!(
        lex("owned_by:\"Alice Brown\" created>=2026-09-01 quip* \"NOT (literal)\"")
            .unwrap()
            .into_iter()
            .map(|t| t.1)
            .collect::<Vec<_>>(),
        vec![
            Token::Atom("owned_by:\"Alice Brown\"".into()),
            Token::Atom("created>=2026-09-01".into()),
            Token::Atom("quip*".into()),
            Token::Atom("\"NOT (literal)\"".into())
        ]
    );
}
#[test]
fn unicode_offsets_and_escaped_quotes() {
    assert!(parse("label:\"café \\\"quoted\\\"\"").is_ok());
    assert_eq!(parse("café OR )").unwrap_err().offset, 9);
}
#[test]
fn no_partial_parse_on_bad_syntax() {
    for s in [
        "",
        "a OR",
        "AND a",
        "a)",
        "(a",
        "()",
        "a AND OR b",
        "\"bad",
        "field:\"bad",
    ] {
        assert!(parse(s).is_err(), "{s}");
    }
}
#[test]
fn parentheses_and_minus() {
    assert_eq!(
        parse("a -(b OR c)").unwrap(),
        Expr::And(
            Box::new(atom("a")),
            Box::new(Expr::Not(Box::new(Expr::Or(
                Box::new(atom("b")),
                Box::new(atom("c"))
            ))))
        )
    );
    assert_eq!(parse("a -b").unwrap(), parse("a NOT b").unwrap());
}
#[test]
fn bounded_negation_is_branch_sensitive() {
    for s in [
        "a NOT b",
        "NOT b a",
        "a (b OR NOT c)",
        "(a NOT b) OR (c NOT d)",
    ] {
        assert!(bounded(&parse(s).unwrap(), false), "{s}");
    }
    for s in ["NOT a", "a OR NOT b", "NOT (a OR b)", "(a OR NOT b) NOT c"] {
        assert!(!bounded(&parse(s).unwrap(), false), "{s}");
    }
}
#[test]
fn complexity_limits_refuse() {
    assert!(parse(&"a ".repeat(65)).is_err());
    assert!(parse(&format!("{}a{}", "(".repeat(9), ")".repeat(9))).is_err());
    assert!(parse(&"a".repeat(4097)).is_err());
    assert!(parse(&"a ".repeat(64)).is_ok());
    assert!(parse(&format!("{}a{}", "(".repeat(8), ")".repeat(8))).is_ok());
}
