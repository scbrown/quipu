//! Tests for the code-entity SHACL shapes (shapes/code-entities.ttl).

use crate::shacl::{Validator, validate_shapes};

const SHAPES: &str = include_str!("../shapes/code-entities.ttl");

#[test]
fn code_entity_shapes_parse() {
    Validator::from_turtle(SHAPES).expect("shapes should parse");
}

#[test]
fn valid_code_module_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:mod1 a bobbin:CodeModule ;
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
bobbin:mod1 a bobbin:CodeModule ;
    bobbin:filePath "src/main.rs" ;
    bobbin:repo "quipu" .
"#;
    // Missing language
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "missing language should fail");
}

#[test]
fn valid_code_symbol_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:mod1 a bobbin:CodeModule ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
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
bobbin:mod1 a bobbin:CodeModule ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
    bobbin:symbolKind "function" ;
    bobbin:definedIn bobbin:mod1 .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "missing name should fail");
}

#[test]
fn code_symbol_invalid_kind_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:mod1 a bobbin:CodeModule ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:sym1 a bobbin:CodeSymbol ;
    bobbin:name "Foo" ;
    bobbin:symbolKind "banana" ;
    bobbin:definedIn bobbin:mod1 .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "invalid symbolKind should fail");
}

#[test]
fn valid_document_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:doc1 a bobbin:Document ;
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
bobbin:doc1 a bobbin:Document .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "missing filePath should fail");
}

#[test]
fn valid_section_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .
bobbin:sec1 a bobbin:Section ;
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
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .
bobbin:sec1 a bobbin:Section ;
    bobbin:headingDepth "1"^^xsd:integer .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "missing heading should fail");
}

#[test]
fn valid_bundle_conforms() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:mod1 a bobbin:CodeModule ;
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
bobbin:mod1 a bobbin:CodeModule ;
    bobbin:filePath "src/lib.rs" ;
    bobbin:repo "quipu" ;
    bobbin:language "rust" .
bobbin:bundle1 a bobbin:Bundle ;
    bobbin:contains bobbin:mod1 .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "missing label should fail");
}

#[test]
fn bundle_missing_contains_fails() {
    let data = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
bobbin:bundle1 a bobbin:Bundle ;
    rdfs:label "empty bundle" .
"#;
    let fb = validate_shapes(SHAPES, data).unwrap();
    assert!(!fb.conforms, "missing contains should fail");
}
