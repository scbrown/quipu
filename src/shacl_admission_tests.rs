use super::*;

const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix quipu: <http://quipu.dev/ns#> .
<urn:PersonShape> a sh:NodeShape ;
 quipu:onViolation "emit" ;
 sh:targetClass <urn:Person> ;
 sh:property [ sh:path <urn:name> ; sh:minCount 1 ; sh:message "name required" ] .
"#;
const DATA: &str = "<urn:alice> a <urn:Person> .";

fn validate_policy(shapes: &str) -> PolicyValidation {
    validate(shapes, DATA, true, crate::shacl::validate_shapes).unwrap()
}

#[test]
fn explicit_emit_preserves_strict_nonconformance_and_diagnostics() {
    let feedback = validate_policy(SHAPES);
    assert!(!feedback.full.conforms);
    assert!(!feedback.blocking);
    assert_eq!(feedback.full.violations, 1);
    assert_eq!(feedback.advisory.len(), 1);
    let report = feedback.report().unwrap();
    assert_eq!(report["conforms"], false);
    assert_eq!(report["blocking"], false);
    assert_eq!(report["advisory_count"], 1);
    assert_eq!(report["results"].as_array().unwrap().len(), 1);
    for issue in [&report["results"][0], &report["advisory_results"][0]] {
        assert!(issue["focus_node"].as_str().unwrap().contains("urn:alice"));
        assert!(issue["path"].as_str().unwrap().contains("urn:name"));
        assert_eq!(issue["message"], "name required");
        assert!(!issue["severity"].as_str().unwrap().is_empty());
        assert!(!issue["source_shape"].as_str().unwrap().is_empty());
    }
}

#[test]
fn default_and_explicit_reject_still_block() {
    for shapes in [
        SHAPES.replace(" quipu:onViolation \"emit\" ;\n", ""),
        SHAPES.replace("\"emit\"", "\"reject\""),
    ] {
        let feedback = validate_policy(&shapes);
        assert!(feedback.blocking);
        assert!(!feedback.full.conforms);
        assert!(feedback.advisory.is_empty());
    }
}

#[test]
fn warning_is_nonblocking_but_strict_nonconforming() {
    let shapes = SHAPES.replace(" quipu:onViolation \"emit\" ;", " sh:severity sh:Warning ;");
    let shapes = shapes.replace(
        "sh:minCount 1 ;",
        "sh:minCount 1 ; sh:severity sh:Warning ;",
    );
    let feedback = validate_policy(&shapes);
    assert!(!feedback.blocking);
    assert!(!feedback.full.conforms);
    assert_eq!(feedback.full.warnings, 1);
    assert!(feedback.advisory.is_empty());
}

#[test]
fn unknown_and_conflicting_policy_values_refuse_even_conforming_data() {
    let valid = "<urn:alice> a <urn:Person> ; <urn:name> \"Alice\" .";
    for shapes in [
        SHAPES.replace("\"emit\"", "\"typo\""),
        SHAPES.replace(
            " quipu:onViolation \"emit\" ;",
            " quipu:onViolation \"emit\", \"reject\" ;",
        ),
    ] {
        assert!(validate(&shapes, valid, true, crate::shacl::validate_shapes).is_err());
    }
    assert!(
        !validate(SHAPES, valid, true, crate::shacl::validate_shapes)
            .unwrap()
            .blocking
    );
}

#[test]
fn mixed_reject_emit_and_warning_keep_the_hard_gate_and_all_results() {
    let shapes = format!(
        "{SHAPES}\n{}\n{}",
        SHAPES
            .replace("PersonShape", "HardShape")
            .replace("\"emit\"", "\"reject\""),
        SHAPES
            .replace(
                "sh:minCount 1 ;",
                "sh:minCount 1 ; sh:severity sh:Warning ;"
            )
            .replace("PersonShape", "WarningShape")
            .replace(" quipu:onViolation \"emit\" ;", " sh:severity sh:Warning ;")
    );
    let feedback = validate_policy(&shapes);
    assert!(feedback.blocking);
    assert!(!feedback.full.conforms);
    assert_eq!(feedback.full.results.len(), 3);
    assert_eq!(feedback.full.violations, 2);
    assert_eq!(feedback.full.warnings, 1);
    assert_eq!(feedback.advisory.len(), 1);
}
