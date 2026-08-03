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
    assert!(
        fb.conforms,
        "a policy assigning a well-formed workflow should conform: {:#?}",
        fb.results
    );
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
    assert!(
        !fb.conforms,
        "assignsWorkflow must reference an aegis:Workflow"
    );
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

// ── Tree-sitter-tier policy catalog (shapes/policies/treesitter.ttl) ──────────

const TREESITTER_CATALOG: &str = include_str!("../shapes/policies/treesitter.ttl");

#[test]
fn treesitter_policy_catalog_conforms() {
    // The shipped tree-sitter-tier catalog — the canonical source Hank projects —
    // must validate against the governance shapes, the same in-graph check Hank
    // runs before it promotes. A drift here is caught at `cargo test`, not at load.
    let fb = validate_shapes(SHAPES, TREESITTER_CATALOG).unwrap();
    assert!(
        fb.conforms,
        "tree-sitter policy catalog should conform: {:#?}",
        fb.results
    );
}

// ── SARC constraint metadata (SARC arXiv:2605.07728 §3.1, §4.2) ──────────────

/// Every SARC field the shape types, on one policy, so the happy path is a
/// single readable fixture the rejection tests can perturb.
fn sarc_policy(extra: &str) -> String {
    format!(
        "{NS}\n\
         aegis:op a aegis:OperatingPoint ; aegis:kind \"exact_predicate\" ; \
             aegis:falsePositiveTolerance 0.0 ; aegis:falseNegativeTolerance 0.0 .\n\
         aegis:p a aegis:Policy ; rdfs:label \"p\" ; aegis:targets \"CodeModule\" ; \
             aegis:claim \"c\" ; aegis:boundary \"action\" ; aegis:effect \"deny\" ; \
             aegis:constraintClass \"hard\" ; aegis:verificationPoint \"PAG\" ; \
             aegis:hostedAtLayer \"tool\" ; aegis:sourceType \"regulatory\" ; \
             aegis:operatingPoint aegis:op ; aegis:latencyBudgetMs 5 {extra} .\n"
    )
}

#[test]
fn a_policy_carrying_the_full_sarc_constraint_object_conforms() {
    let fb = validate_shapes(SHAPES, &sarc_policy("")).unwrap();
    assert!(fb.conforms, "{:#?}", fb.results);
}

#[test]
fn constraint_class_out_of_enum_is_rejected() {
    let data = sarc_policy("").replace("\"hard\"", "\"catastrophic\"");
    assert!(
        !validate_shapes(SHAPES, &data).unwrap().conforms,
        "a constraintClass outside {{hard,soft,escalation}} must be rejected"
    );
}

#[test]
fn verification_point_out_of_enum_is_rejected() {
    let data = sarc_policy("").replace("\"PAG\"", "\"vibes\"");
    assert!(!validate_shapes(SHAPES, &data).unwrap().conforms);
}

#[test]
fn the_prompt_layer_is_not_a_hostable_layer() {
    // SARC I6, made unrepresentable rather than merely discouraged: a hard
    // constraint hosted only at the prompt layer is an aspiration, and the
    // vocabulary declines to express it.
    let data = sarc_policy("").replace("\"tool\"", "\"prompt\"");
    assert!(
        !validate_shapes(SHAPES, &data).unwrap().conforms,
        "hostedAtLayer must have no \"prompt\" value"
    );
}

#[test]
fn on_timeout_admits_only_deny() {
    // SARC §5.3: default-allow under operator unavailability turns an
    // escalation into a no-op exactly when load makes it matter. The one-value
    // enum is the point.
    let ok = sarc_policy("; aegis:onTimeout \"deny\"");
    assert!(validate_shapes(SHAPES, &ok).unwrap().conforms);
    let bad = sarc_policy("; aegis:onTimeout \"allow\"");
    assert!(
        !validate_shapes(SHAPES, &bad).unwrap().conforms,
        "onTimeout \"allow\" must be unrepresentable"
    );
}

#[test]
fn throttle_is_an_effect() {
    // The soft-class PAA response (SARC §4.2, Table 3). Absent from the enum,
    // a soft constraint has only warn/record — it can say something happened
    // and cannot change what happens next.
    let data = sarc_policy("").replace("\"deny\"", "\"throttle\"");
    assert!(validate_shapes(SHAPES, &data).unwrap().conforms);
}

#[test]
fn operating_point_requires_a_kind_and_types_its_tolerances() {
    let no_kind = format!("{NS}\naegis:op a aegis:OperatingPoint ; aegis:threshold 0.5 .\n");
    assert!(
        !validate_shapes(SHAPES, &no_kind).unwrap().conforms,
        "an operating point with no kind is not a calibration, it is a number"
    );
    let bad_kind = format!("{NS}\naegis:op a aegis:OperatingPoint ; aegis:kind \"eyeballed\" .\n");
    assert!(!validate_shapes(SHAPES, &bad_kind).unwrap().conforms);
}

#[test]
fn policy_operating_point_must_point_at_an_operating_point() {
    let data = format!(
        "{NS}\n\
         aegis:notop a aegis:Selector ; aegis:name \"s\" ; aegis:evidenceSource \"x\" .\n\
         aegis:p a aegis:Policy ; rdfs:label \"p\" ; aegis:targets \"T\" ; aegis:claim \"c\" ; \
             aegis:operatingPoint aegis:notop .\n"
    );
    assert!(!validate_shapes(SHAPES, &data).unwrap().conforms);
}

#[test]
fn the_shipped_catalog_declares_a_class_and_a_placement_for_every_action_policy() {
    // The catalog is what Hank projects. A policy in it that carries no class
    // projects as an unplaced constraint, which is the state this whole change
    // exists to make impossible — so assert on the shipped file, not a fixture.
    for (policy, class, point) in [
        ("policy_no_ticket_in_comment", "hard", "PAG"),
        ("policy_todo_needs_ticket", "soft", "PAA"),
    ] {
        let block = TREESITTER_CATALOG
            .split(&format!("aegis:{policy} a aegis:Policy"))
            .nth(1)
            .unwrap_or_else(|| panic!("catalog no longer defines {policy}"))
            .split(" .\n")
            .next()
            .unwrap();
        assert!(
            block.contains(&format!("aegis:constraintClass \"{class}\"")),
            "{policy} must declare constraintClass \"{class}\""
        );
        assert!(
            block.contains(&format!("aegis:verificationPoint \"{point}\"")),
            "{policy} must declare verificationPoint \"{point}\""
        );
        assert!(
            block.contains("aegis:operatingPoint"),
            "{policy} must declare an operating point (SARC I2)"
        );
    }
}

#[test]
fn a_policy_may_name_a_selector_and_predicate() {
    // The additive congruence link: a structural policy composes its two atoms.
    let data = format!(
        "{NS}\n\
         aegis:s a aegis:Selector ; aegis:name \"c\" ; aegis:evidenceSource \"(line_comment) @c\" ; aegis:tier \"tree-sitter\" .\n\
         aegis:pr a aegis:Predicate ; aegis:name \"t\" ; aegis:evidenceSource \"X-1\" ; aegis:matchType \"must-not-match\" ; aegis:tier \"tree-sitter\" .\n\
         aegis:p a aegis:Policy ; rdfs:label \"p\" ; aegis:targets \"CodeModule\" ; aegis:claim \"c\" ; \
             aegis:boundary \"action\" ; aegis:effect \"deny\" ; aegis:selector aegis:s ; aegis:predicate aegis:pr .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        fb.conforms,
        "a policy naming a selector + predicate should conform: {:#?}",
        fb.results
    );
}

#[test]
fn predicate_match_type_out_of_enum_is_rejected() {
    let data = format!(
        "{NS}\naegis:pr a aegis:Predicate ; aegis:name \"t\" ; \
         aegis:evidenceSource \"x\" ; aegis:matchType \"must-explode\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        !fb.conforms,
        "a matchType outside the enum must be rejected"
    );
}

#[test]
fn policy_selector_must_point_at_a_selector() {
    // sh:class aegis:Selector — selector pointing at a non-Selector is malformed.
    let data = format!(
        "{NS}\naegis:p a aegis:Policy ; rdfs:label \"p\" ; aegis:targets \"CodeModule\" ; \
         aegis:claim \"c\" ; aegis:selector aegis:ghost .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(!fb.conforms, "selector must reference an aegis:Selector");
}

// ── Property declarations (shapes/aegis-properties.ttl) ──────────────────────

const AEGIS_PROPERTIES: &str = include_str!("../shapes/aegis-properties.ttl");

#[test]
fn the_property_declarations_parse() {
    // A vocabulary file that does not parse is a vocabulary nothing can read,
    // and it would fail silently — the reasoner simply loads no axioms.
    let fb = validate_shapes(SHAPES, AEGIS_PROPERTIES).unwrap();
    assert!(
        fb.conforms,
        "property declarations must parse and not violate the governance shapes: {:#?}",
        fb.results
    );
}

#[test]
fn every_sarc_property_the_shape_constrains_is_also_described() {
    // The gap this file closes: a shape says what is VALID, and said nothing
    // about what a term MEANS. If a future field is added to the shape without
    // a declaration here, it is back to being defined only by its constraints —
    // so the two files are checked against each other rather than trusted to
    // drift together.
    for property in [
        "constraintClass",
        "verificationPoint",
        "hostedAtLayer",
        "sourceType",
        "operatingPoint",
        "latencyBudgetMs",
        "reversibilityWindowSeconds",
        "onTimeout",
        "backoffFormula",
    ] {
        assert!(
            SHAPES.contains(&format!("aegis:{property}")),
            "{property} should be constrained by the governance shapes"
        );
        assert!(
            AEGIS_PROPERTIES.contains(&format!("aegis:{property} a owl:")),
            "aegis:{property} is constrained by a shape but not DECLARED — \
             add it to shapes/aegis-properties.ttl with a domain, range and comment"
        );
        // A declaration with no comment is a declaration that documents nothing.
        let block = AEGIS_PROPERTIES
            .split(&format!("aegis:{property} a owl:"))
            .nth(1)
            .unwrap()
            .split(" .\n")
            .next()
            .unwrap();
        assert!(
            block.contains("rdfs:comment"),
            "aegis:{property} must carry an rdfs:comment saying what it MEANS"
        );
    }
}

#[test]
fn domains_are_declared_only_where_the_subject_is_unambiguous() {
    // rdfs:domain is an INFERENCE RULE the reasoner materialises, not
    // documentation: declaring it asserts rdf:type on every subject carrying
    // the property. The generically-named OperatingPoint fields deliberately
    // carry no domain, because the first other thing in the estate to use
    // `aegis:kind` would otherwise be silently typed an OperatingPoint.
    for generic in ["kind", "threshold", "calibrationBasis"] {
        let block = AEGIS_PROPERTIES
            .split(&format!("aegis:{generic} a owl:"))
            .nth(1)
            .unwrap()
            .split(" .\n")
            .next()
            .unwrap();
        assert!(
            !block.contains("rdfs:domain"),
            "aegis:{generic} is too generic a name to carry a materialising domain"
        );
    }
    // The policy-specific ones do carry it, because their subject is not in doubt.
    let block = AEGIS_PROPERTIES
        .split("aegis:constraintClass a owl:")
        .nth(1)
        .unwrap()
        .split(" .\n")
        .next()
        .unwrap();
    assert!(block.contains("rdfs:domain aegis:Policy"));
}
