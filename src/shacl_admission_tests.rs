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

#[test]
fn wu_compact_emit_cannot_absorb_adjacent_reject_shape() {
    let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix quipu: <http://quipu.dev/ns#> .
<urn:Emit> a sh:NodeShape ; quipu:onViolation "emit" ; sh:targetClass <urn:Person> ; sh:property [ sh:path <urn:name> ; sh:minCount 1 ] . <urn:Reject> a sh:NodeShape ; quipu:onViolation "reject" ; sh:targetClass <urn:Person> ; sh:property [ sh:path <urn:mandatory> ; sh:minCount 1 ] .
"#;
    let result = validate_policy(shapes);
    assert!(
        result.blocking,
        "explicit reject must block even beside emit in valid compact Turtle"
    );
}

#[test]
fn compact_and_aliased_emit_preserve_strict_report() {
    for shapes in [
        SHAPES.replace('\n', " "),
        SHAPES
            .replace("@prefix quipu:", "@prefix policy:")
            .replace("quipu:onViolation", "policy:onViolation"),
        SHAPES.replace("quipu:onViolation", "<http://quipu.dev/ns#onViolation>"),
        SHAPES.replace("http://quipu.dev/ns#", "http://quipu.dev/ontology/"),
    ] {
        let feedback = validate_policy(&shapes);
        assert!(!feedback.blocking, "{shapes}");
        assert!(!feedback.full.conforms);
        assert_eq!(feedback.advisory.len(), 1);
    }
}
#[test]
fn misleading_policy_comment_or_literal_never_grants_emit() {
    let reject = SHAPES.replace(" quipu:onViolation \"emit\" ;\n", "");
    for shapes in [
        reject.replace(
            " sh:targetClass",
            " # quipu:onViolation \"emit\"\n sh:targetClass",
        ),
        reject.replace("\"name required\"", "'quipu:onViolation \"emit\"'"),
        SHAPES.replace("http://quipu.dev/ns#", "https://example.org/foreign#"),
    ] {
        assert!(validate_policy(&shapes).blocking, "{shapes}");
    }
}
#[test]
fn default_reject_and_shared_property_survive_compact_target_partition() {
    let shapes=r#"@prefix sh:<http://www.w3.org/ns/shacl#>.
        @prefix q:<http://quipu.dev/ns#>.
        <urn:Soft> a sh:NodeShape; q:onViolation "emit"; sh:targetClass <urn:Person>; sh:property <urn:Property>.
        <urn:Hard> a sh:NodeShape; sh:targetClass <urn:Person>; sh:property <urn:Property>.
        <urn:Property> a sh:PropertyShape; sh:path <urn:name>; sh:minCount 1.
    "#.replace('\n'," ");
    let feedback = validate_policy(&shapes);
    assert!(feedback.blocking);
    assert_eq!(feedback.full.results.len(), 2);
    assert_eq!(feedback.advisory.len(), 1);
}
#[test]
fn nested_property_emit_cannot_downgrade_its_default_reject_parent() {
    let shapes = SHAPES
        .replace(" quipu:onViolation \"emit\" ;\n", "")
        .replace(
            "sh:path <urn:name>",
            "quipu:onViolation \"emit\"; sh:path <urn:name>",
        );
    assert!(validate_policy(&shapes).blocking);
}
#[test]
fn implicit_class_emit_is_not_revalidated_as_reject() {
    let shapes = r#"@prefix sh:<http://www.w3.org/ns/shacl#>.
        @prefix q:<http://quipu.dev/ns#>.
        @prefix rdfs:<http://www.w3.org/2000/01/rdf-schema#>.
        <urn:Person> a sh:NodeShape,rdfs:Class; q:onViolation "emit"; sh:property [sh:path <urn:name>;sh:minCount 1].
    "#;
    let feedback = validate_policy(shapes);
    assert!(!feedback.blocking);
    assert_eq!(feedback.full.results.len(), 1);
    assert_eq!(feedback.advisory.len(), 1);
    assert!(validate_policy(&shapes.replace("\"emit\"", "\"reject\"")).blocking);
}
#[test]
fn all_core_target_kinds_mask_only_inactive_shapes() {
    for target in [
        "sh:targetNode <urn:alice>",
        "sh:targetSubjectsOf <urn:touch>",
        "sh:targetObjectsOf <urn:touch>",
    ] {
        let shapes = SHAPES.replace("sh:targetClass <urn:Person>", target);
        let data = "<urn:alice> <urn:touch> <urn:alice>.";
        let feedback = validate(&shapes, data, true, crate::shacl::validate_shapes).unwrap();
        assert!(!feedback.blocking, "{target}");
        assert!(!feedback.full.conforms);
        assert_eq!(feedback.advisory.len(), 1);
    }
}
#[test]
fn custom_targets_and_cross_namespace_policy_conflicts_refuse() {
    let shapes = SHAPES.replace(
        "sh:targetClass <urn:Person>",
        "sh:target <urn:CustomTarget>",
    );
    let error = match validate(&shapes, DATA, true, crate::shacl::validate_shapes) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("custom target policy was admitted"),
    };
    assert!(error.contains("custom SHACL targets"));
    let shapes = SHAPES.replace(
        "quipu:onViolation \"emit\"",
        "quipu:onViolation \"emit\"; <http://quipu.dev/ontology/onViolation> \"reject\"",
    );
    assert!(validate(&shapes, DATA, true, crate::shacl::validate_shapes).is_err());
}

#[test]
fn anonymous_shape_and_rdf_list_constraints_survive_partition() {
    let shapes = r#"@prefix sh:<http://www.w3.org/ns/shacl#>.
        @prefix q:<http://quipu.dev/ns#>.
        [ a sh:NodeShape; q:onViolation "emit"; sh:targetNode <urn:alice>;
          sh:property [sh:path <urn:role>; sh:in ("reader" "writer")] ] .
    "#;
    let data = "<urn:alice> <urn:role> \"rogue\" .";
    let feedback = validate(shapes, data, true, crate::shacl::validate_shapes).unwrap();
    assert!(!feedback.blocking);
    assert!(!feedback.full.conforms);
    assert_eq!(feedback.advisory.len(), 1);
    assert!(
        validate(
            &shapes.replace("\"emit\"", "\"reject\""),
            data,
            true,
            crate::shacl::validate_shapes
        )
        .unwrap()
        .blocking
    );
}
