use super::*;

#[test]
fn brackets_keywords_and_operators_count() {
    let q = "DELETE { ?s ?p ?o } WHERE { ?s ?p ?o FILTER NOT EXISTS { ?s ?p ?o } }";
    // { { FILTER NOT EXISTS { = 6
    assert_eq!(structural_cost(q), 6);
    assert_eq!(structural_cost("FILTER((1))"), 3);
    // FILTER ( + * = | | ! = 8
    assert_eq!(structural_cost("FILTER(1+2*3 = 7 || !?b)"), 8);
}

#[test]
fn strings_names_and_iris_do_not_count_their_ordinary_characters() {
    let q = r#"INSERT DATA { <http://ex.org/a-b/c?d=e&f=g#h> ex:my-name "{ FILTER ( UNION + - / |" ;
               <urn:q> '''multi { ( line''' ; <urn:r> "x"@en-US ; <urn:s> "1"^^xsd:integer }"#;
    assert_eq!(structural_cost(q), 1, "only the opening brace");
    assert_eq!(
        structural_cost("?filter ?union ex:optional-thing $minus"),
        0
    );
    assert_eq!(structural_cost(r#""esc \" { (" "#), 0);
}

#[test]
fn brackets_inside_an_iri_shaped_region_still_count() {
    // The bypass: `<` followed by an IRI-shaped run hid every parenthesis in it.
    let q = format!("FILTER(?o<{}1{}>?o)", "(".repeat(50), ")".repeat(50));
    assert!(structural_cost(&q) >= 50, "{}", structural_cost(&q));
}

#[test]
fn operator_chains_count() {
    assert!(structural_cost(&vec!["1"; 101].join("+")) >= 100);
    assert!(structural_cost(&vec!["<urn:p>"; 101].join("|")) >= 100);
    assert!(structural_cost(&"!".repeat(100)) >= 100);
    assert!(structural_cost(&format!("{}<urn:p>", "^".repeat(100))) >= 100);
}

#[test]
fn a_long_iri_shaped_region_counts_its_operators() {
    let short = format!("<{}>", ["a"; 10].join("-"));
    assert_eq!(structural_cost(&short), 0);
    let long = format!("<{}>", vec!["1"; 400].join("-"));
    assert!(structural_cost(&long) >= 399, "{}", structural_cost(&long));
}

#[test]
fn comments_are_not_skipped() {
    // Over-counting a comment is acceptable; letting a comment marker hide
    // structure from the bound would not be.
    assert_eq!(structural_cost("# ((\n"), 2);
}

#[test]
fn keywords_match_case_insensitively() {
    assert_eq!(structural_cost("filter Union OPTIONAL minus exists not"), 6);
}

#[test]
fn check_accepts_at_the_limit_and_refuses_one_over() {
    assert!(check(&"{".repeat(STRUCTURE_LIMIT)).is_ok());
    let err = check(&"{".repeat(STRUCTURE_LIMIT + 1))
        .unwrap_err()
        .to_string();
    assert!(err.contains("exceeds the limit"), "{err}");
}

#[test]
fn unterminated_input_does_not_panic() {
    for s in ["\"abc", "'''abc", "<urn:abc", "\\", "\"x\"@", "\"x\"^"] {
        let _ = structural_cost(s);
    }
}

#[test]
fn parse_reports_syntax_errors_separately_from_the_bound() {
    let ok = parse_query(
        spargebra::SparqlParser::new(),
        "SELECT * WHERE { ?s ?p ?o }",
    )
    .unwrap();
    assert!(ok.is_ok());
    let bad = parse_query(spargebra::SparqlParser::new(), "SELECT WHERE").unwrap();
    assert!(bad.is_err());
    assert!(
        parse_query(
            spargebra::SparqlParser::new(),
            &"{".repeat(STRUCTURE_LIMIT + 1)
        )
        .is_err()
    );
}
