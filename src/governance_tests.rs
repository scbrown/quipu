//! Tests for the governance-plane ("the loom") SHACL shapes
//! (shapes/governance.ttl) — Phase 1 definition-time verification. A workflow /
//! policy / step / verdict definition that violates its shape is rejected at
//! write; well-formed ones conform. See the governance-plane design.

use crate::shacl::{Validator, validate_shapes};

const SHAPES: &str = include_str!("../shapes/governance.ttl");
const NS: &str = "@prefix aegis: <http://aegis.gastown.local/ontology/> .\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n";

#[test]
fn governance_shapes_parse() {
    Validator::from_turtle(SHAPES).expect("governance shapes should parse");
}

#[test]
fn well_formed_workflow_and_step_conform() {
    let data = format!(
        "{NS}\n\
         aegis:sdlc a aegis:Workflow ; rdfs:label \"feature-sdlc\" ; aegis:hasStep aegis:implement .\n\
         aegis:implement a aegis:Step ; rdfs:label \"implement\" ; aegis:gatedBy aegis:cov ; aegis:actor \"agent-fix-3\" .\n\
         aegis:cov a aegis:Policy ; rdfs:label \"plan-covers-blast-radius\" ; \
             aegis:targets \"Step\" ; aegis:claim \"all s in blast-radius(change): s in plan.targets\" ; \
             aegis:boundary \"transition\" ; aegis:effect \"deny\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        fb.conforms,
        "well-formed workflow should conform: {:#?}",
        fb.results
    );
}

#[test]
fn workflow_without_a_step_is_rejected() {
    // aegis:hasStep minCount 1 — a workflow with no entry step is malformed.
    let data = format!("{NS}\naegis:w a aegis:Workflow ; rdfs:label \"empty\" .\n");
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(!fb.conforms, "a workflow with no step must be rejected");
}

#[test]
fn policy_missing_target_or_claim_is_rejected() {
    let data =
        format!("{NS}\naegis:p a aegis:Policy ; rdfs:label \"p\" ; aegis:targets \"Step\" .\n");
    // missing aegis:claim (minCount 1)
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(!fb.conforms, "a policy without a claim must be rejected");
}

#[test]
fn policy_effect_out_of_enum_is_rejected() {
    let data = format!(
        "{NS}\naegis:p a aegis:Policy ; rdfs:label \"p\" ; \
         aegis:targets \"Step\" ; aegis:claim \"c\" ; aegis:effect \"nuke\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        !fb.conforms,
        "an effect outside the allowed set must be rejected"
    );
}

#[test]
fn well_formed_verdict_conforms_and_unknown_outcome_is_valid() {
    // 'unknown' (no evidence) is a distinct, valid outcome — not a soft fail.
    let data = format!(
        "{NS}\naegis:v1 a aegis:Verdict ; rdfs:label \"tests-green?\" ; \
         aegis:predicateId \"tests-green\" ; aegis:targetRef \"commit:abc\" ; \
         aegis:outcome \"unknown\" ; aegis:evidenceHash \"sha256:00\" ; \
         aegis:verifier \"ci\" ; aegis:signature \"sig:..\" ; aegis:tier \"attested\" ; aegis:freshness \"fresh\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        fb.conforms,
        "well-formed verdict should conform: {:#?}",
        fb.results
    );
}

#[test]
fn verdict_without_signature_or_evidence_hash_is_rejected() {
    // A verdict is an attestation, not a claim: signature + evidence binding
    // are mandatory (verdict integrity).
    let data = format!(
        "{NS}\naegis:v a aegis:Verdict ; rdfs:label \"v\" ; \
         aegis:predicateId \"p\" ; aegis:targetRef \"r\" ; aegis:outcome \"satisfied\" ; aegis:verifier \"hank\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        !fb.conforms,
        "an unsigned / unbound verdict must be rejected"
    );
}

#[test]
fn decision_outcome_enum_enforced() {
    let ok = format!(
        "{NS}\naegis:d a aegis:Decision ; rdfs:label \"d\" ; \
         aegis:outcome \"approve\" ; aegis:by \"stiwi\" ; aegis:evidenceHash \"h\" .\n"
    );
    assert!(
        validate_shapes(SHAPES, &ok).unwrap().conforms,
        "valid decision should conform"
    );
    let bad = format!(
        "{NS}\naegis:d a aegis:Decision ; rdfs:label \"d\" ; \
         aegis:outcome \"maybe\" ; aegis:by \"stiwi\" ; aegis:evidenceHash \"h\" .\n"
    );
    assert!(
        !validate_shapes(SHAPES, &bad).unwrap().conforms,
        "a non-enum decision outcome must be rejected"
    );
}

#[test]
fn transition_requires_on_and_to() {
    let data = format!("{NS}\naegis:t a aegis:Transition ; aegis:on \"approve\" .\n");
    // missing aegis:to
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        !fb.conforms,
        "a transition without a target Step must be rejected"
    );
}

#[test]
fn policy_can_assign_a_workflow() {
    // A policy's require-approval / escalate effect names the workflow to run via
    // aegis:assignsWorkflow -> a well-formed aegis:Workflow. This is the seam a
    // Hank guard / Shantytown subscriber reads.
    let data = format!(
        "{NS}\n\
         aegis:uidemo a aegis:Workflow ; rdfs:label \"ui-demo-required\" ; aegis:hasStep aegis:demo .\n\
         aegis:demo a aegis:Step ; rdfs:label \"demo the UI effect\" ; aegis:actor \"agent\" .\n\
         aegis:p a aegis:Policy ; rdfs:label \"ui-surface-demo\" ; \
             aegis:targets \"CodeModule\" ; aegis:claim \"c\" ; aegis:boundary \"transition\" ; \
             aegis:effect \"require-approval\" ; aegis:assignsWorkflow aegis:uidemo .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(fb.conforms, "a policy assigning a well-formed workflow should conform: {:#?}", fb.results);
}

#[test]
fn policy_assigns_workflow_must_point_at_a_workflow() {
    // sh:class aegis:Workflow — assignsWorkflow pointing at a non-Workflow
    // (here an untyped node) is a malformed policy, rejected at write.
    let data = format!(
        "{NS}\naegis:p a aegis:Policy ; rdfs:label \"p\" ; \
         aegis:targets \"Step\" ; aegis:claim \"c\" ; aegis:assignsWorkflow aegis:ghost .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(!fb.conforms, "assignsWorkflow must reference an aegis:Workflow");
}

#[test]
fn verifier_registration_shape() {
    let ok = format!(
        "{NS}\naegis:r a aegis:VerifierRegistration ; aegis:verifier \"hank\" ; aegis:attests \"has-test\" .\n"
    );
    assert!(
        validate_shapes(SHAPES, &ok).unwrap().conforms,
        "valid registration should conform"
    );
    let bad = format!("{NS}\naegis:r a aegis:VerifierRegistration ; aegis:verifier \"hank\" .\n");
    assert!(
        !validate_shapes(SHAPES, &bad).unwrap().conforms,
        "registration without an attested predicate must be rejected"
    );
}
