//! Tests for the code-entity SHACL shapes (shapes/code-entities.ttl).

use crate::shacl::{Validator, validate_shapes};

const SHAPES: &str = include_str!("../shapes/code-entities.ttl");

/// Assert the data fails validation FOR THE STATED REASON.
///
/// A negative fixture asserting only `!conforms` passes as soon as *anything* is
/// wrong with it, which makes it silently useless the moment a new constraint is
/// added to the shape. That is not hypothetical here: when `rdfs:label` became
/// required on these four classes, every fixture in this file lost its label and
/// the five positive tests went red — while the negative ones stayed green,
/// having quietly stopped testing the field named in their own function name.
/// The red tests were the lucky half; these are the half that would not have told
/// anyone.
fn fails_on(shapes: &str, data: &str, path_suffix: &str) {
    let fb = validate_shapes(shapes, data).unwrap();
    assert!(!fb.conforms, "expected a violation on {path_suffix}");
    let paths: Vec<String> = fb.results.iter().filter_map(|r| r.path.clone()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with(path_suffix)),
        "expected a violation on `{path_suffix}`, got {paths:?} — the fixture is \
         failing for a DIFFERENT reason than the one this test names"
    );
}

#[test]
fn code_entity_shapes_parse() {
    Validator::from_turtle(SHAPES).expect("shapes should parse");
}

#[test]
fn valid_code_module_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "main" ;
    bobbin:filePath "src/main.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(
        fb.conforms,
        "valid CodeModule should conform: {:#?}",
        fb.results
    );
}

#[test]
fn code_module_missing_required_field_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "main.rs" ;
    bobbin:filePath "src/main.rs" ;
    bobbin:repo "quipu" .
"#;
    // Missing language
    fails_on(SHAPES, data, "language");
}

#[test]
fn valid_code_symbol_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "lib" ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
    rdfs:label "validate" ;
    bobbin:name "validate" ;
    bobbin:symbolKind "function" ;
    bobbin:definedIn bobbin:mod1 .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(
        fb.conforms,
        "valid CodeSymbol should conform: {:#?}",
        fb.results
    );
}

#[test]
fn code_symbol_missing_name_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "lib.rs" ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
    rdfs:label "validate" ;
    bobbin:symbolKind "function" ;
    bobbin:definedIn bobbin:mod1 .
"#;
    fails_on(SHAPES, data, "name");
}

#[test]
fn code_symbol_invalid_kind_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "lib.rs" ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
    rdfs:label "Foo" ;
    bobbin:name "Foo" ;
    bobbin:symbolKind "banana" ;
    bobbin:definedIn bobbin:mod1 .
"#;
    fails_on(SHAPES, data, "symbolKind");
}

#[test]
fn valid_document_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:doc1 a bobbin:Document ;
    rdfs:label "README" ;
    bobbin:filePath "docs/README.md" .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(
        fb.conforms,
        "valid Document should conform: {:#?}",
        fb.results
    );
}

#[test]
fn document_missing_filepath_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:doc1 a bobbin:Document ;
    rdfs:label "README" .
"#;
    fails_on(SHAPES, data, "filePath");
}

#[test]
fn valid_section_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .
bobbin:sec1 a bobbin:Section ;
    rdfs:label "Getting Started" ;
    bobbin:heading "Getting Started" ;
    bobbin:headingDepth "2"^^xsd:integer .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(
        fb.conforms,
        "valid Section should conform: {:#?}",
        fb.results
    );
}

#[test]
fn section_missing_heading_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .
bobbin:sec1 a bobbin:Section ;
    rdfs:label "Intro" ;
    bobbin:headingDepth "1"^^xsd:integer .
"#;
    fails_on(SHAPES, data, "heading");
}

#[test]
fn valid_bundle_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "lib" ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:bundle1 a bobbin:Bundle ;
    rdfs:label "core library" ;
    bobbin:contains bobbin:mod1 .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(
        fb.conforms,
        "valid Bundle should conform: {:#?}",
        fb.results
    );
}

#[test]
fn bundle_missing_label_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "lib.rs" ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:bundle1 a bobbin:Bundle ;
    bobbin:contains bobbin:mod1 .
"#;
    fails_on(SHAPES, data, "label");
}

#[test]
fn bundle_missing_contains_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:bundle1 a bobbin:Bundle ;
    rdfs:label "empty bundle" .
"#;
    fails_on(SHAPES, data, "contains");
}

// The `rdfs:label` requirement covers FOUR classes; only Bundle had a
// missing-label test. Without these, dropping the constraint from any of the
// other three would leave this whole file green.

#[test]
fn code_module_missing_label_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:mod1 a bobbin:CodeModule ;
    bobbin:filePath "src/main.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
"#;
    fails_on(SHAPES, data, "label");
}

#[test]
fn code_symbol_missing_label_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
    rdfs:label "lib.rs" ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
    bobbin:name "validate" ;
    bobbin:symbolKind "function" ;
    bobbin:definedIn bobbin:mod1 .
"#;
    fails_on(SHAPES, data, "label");
}

#[test]
fn document_missing_label_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:doc1 a bobbin:Document ;
    bobbin:filePath "docs/README.md" .
"#;
    fails_on(SHAPES, data, "label");
}

#[test]
fn section_missing_label_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .
bobbin:sec1 a bobbin:Section ;
    bobbin:heading "Getting Started" ;
    bobbin:headingDepth "2"^^xsd:integer .
"#;
    fails_on(SHAPES, data, "label");
}

/// A language-TAGGED label is `rdf:langString`, not `xsd:string`, so it does NOT
/// satisfy the constraint. The shape says that is intentional; nothing tested it.
#[test]
fn a_language_tagged_label_does_not_satisfy_the_datatype() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:doc1 a bobbin:Document ;
    rdfs:label "README"@en ;
    bobbin:filePath "docs/README.md" .
"#;
    fails_on(SHAPES, data, "label");
}
