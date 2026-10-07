use super::*;

#[test]
fn counts_braces_parens_and_nesting_keywords() {
    let q = "DELETE { ?s ?p ?o } WHERE { ?s ?p ?o FILTER NOT EXISTS { ?s ?p ?o } }";
    // { { FILTER EXISTS { = 5
    assert_eq!(structural_cost(q), 5);
    assert_eq!(
        structural_cost("SELECT * WHERE { { ?a ?b ?c } UNION { ?a ?b ?c } }"),
        4
    );
    assert_eq!(structural_cost("FILTER((1))"), 3);
}

#[test]
fn literals_iris_comments_and_names_do_not_count() {
    let q = r#"INSERT DATA { <urn:x:a> <urn:p> "{ FILTER ( UNION" ;
               <urn:q> '''multi { ( line''' } # FILTER { ( UNION
               "#;
    assert_eq!(structural_cost(q), 1);
    assert_eq!(structural_cost("?filter ?union ex:optional $minus"), 0);
    assert_eq!(structural_cost(r#""esc \" { (" "#), 0);
}

#[test]
fn less_than_is_an_operator_not_an_iri() {
    assert_eq!(structural_cost("FILTER(?x < 5 && (?y > 2))"), 3);
}

#[test]
fn keywords_match_case_insensitively() {
    assert_eq!(structural_cost("filter Union OPTIONAL minus exists"), 5);
}

#[test]
fn check_accepts_at_the_limit_and_refuses_one_over() {
    let at = "{".repeat(STRUCTURE_LIMIT);
    assert!(check(&at).is_ok());
    let over = "{".repeat(STRUCTURE_LIMIT + 1);
    let err = check(&over).unwrap_err().to_string();
    assert!(err.contains("nests too deeply"), "{err}");
}

#[test]
fn an_unterminated_literal_or_iri_does_not_panic() {
    let _ = structural_cost("\"abc");
    let _ = structural_cost("'''abc");
    let _ = structural_cost("<urn:abc");
    let _ = structural_cost("\\");
}
