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

// ── Entity-grounded and inexact-tier predicate vocabulary (quipu-3aj) ─────────
// Design: docs/design/semantic-grounded-edit-policies.md. The shapes admit the
// vocabulary; the definition-time rules in src/governance/placement.rs enforce
// the cross-field requirements (grounded matchType => groundingQuery, inexact
// tier => nonzero OperatingPoint, no hard PAG deny) that SHACL core cannot.

#[test]
fn a_grounded_predicate_conforms() {
    // Both grounded directions, carrying the full grounded vocabulary: token
    // candidates, the id-set query, and the exact grounded tier.
    for mt in ["must-ground", "must-not-ground"] {
        let data = format!(
            "{NS}\naegis:pr a aegis:Predicate ; aegis:name \"no-ticket-in-comment\" ; \
             aegis:evidenceSource \"graph:work-item-id-set\" ; \
             aegis:candidateSource \"token\" ; \
             aegis:groundingQuery \"SELECT ?id WHERE {{ ?w a aegis:WorkItem ; aegis:identifier ?id }}\" ; \
             aegis:matchType \"{mt}\" ; aegis:tier \"tree-sitter+graph\" .\n"
        );
        let fb = validate_shapes(SHAPES, &data).unwrap();
        assert!(
            fb.conforms,
            "a grounded predicate ({mt}) should conform: {:#?}",
            fb.results
        );
    }
}

#[test]
fn an_inexact_tier_predicate_conforms() {
    // The similarity and generative tiers are admitted by the SHAPE; their
    // placement discipline (no hard PAG deny, nonzero OperatingPoint) is the
    // placement check's job, not this enum's.
    for tier in ["embedding", "model"] {
        let data = format!(
            "{NS}\naegis:pr a aegis:Predicate ; aegis:name \"references-tracked-work\" ; \
             aegis:evidenceSource \"bobbin:beads-index\" ; aegis:tier \"{tier}\" .\n"
        );
        let fb = validate_shapes(SHAPES, &data).unwrap();
        assert!(
            fb.conforms,
            "an {tier}-tier predicate should conform: {:#?}",
            fb.results
        );
    }
}

#[test]
fn predicate_tier_out_of_enum_is_rejected() {
    // The enum was EXTENDED, not unhinged: a value outside the widened set is
    // still refused, which is what keeps "the tier rides the verdict" a claim
    // with teeth.
    let data = format!(
        "{NS}\naegis:pr a aegis:Predicate ; aegis:name \"t\" ; \
         aegis:evidenceSource \"x\" ; aegis:tier \"vibes\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(!fb.conforms, "a tier outside the enum must be rejected");
}

#[test]
fn the_catalog_v2_grounded_predicate_names_its_id_set() {
    // The shipped exemplar has to model the discipline it documents: token
    // candidates (never a pattern as authority), the grounding query, the
    // grounded direction, and the exact grounded tier.
    let block = TREESITTER_CATALOG
        .split("aegis:pred_no_ticket_in_comment_v2 a aegis:Predicate")
        .nth(1)
        .expect("catalog no longer defines pred_no_ticket_in_comment_v2")
        .split(" .\n")
        .next()
        .unwrap();
    assert!(block.contains("aegis:candidateSource \"token\""));
    assert!(block.contains("aegis:groundingQuery"));
    assert!(block.contains("aegis:matchType \"must-not-ground\""));
    assert!(block.contains("aegis:tier \"tree-sitter+graph\""));
}

// ── Claimed-linkage verification (quipu-508) ─────────────────────────────────
// Design #1: an aegis:implements claim becomes checkable. The catalog ships
// the embedding-tier must-ground predicate; the Verdict shape gains the
// similarity seal (score/threshold/model/watermark) and the closed
// three-outcome aegis:linkageOutcome; the verifier is
// src/governance/linkage.rs.

const LINKAGE_CATALOG: &str = include_str!("../shapes/policies/linkage.ttl");

#[test]
fn linkage_policy_catalog_conforms() {
    // Shipped data exercises the shapes, not only fixtures — same contract as
    // the tree-sitter catalog.
    let fb = validate_shapes(SHAPES, LINKAGE_CATALOG).unwrap();
    assert!(
        fb.conforms,
        "linkage policy catalog should conform: {:#?}",
        fb.results
    );
}

#[test]
fn the_linkage_predicate_models_the_embedding_grounded_discipline() {
    // The exemplar has to model what it documents: grounded direction over
    // the work-item id set, the honest inexact tier, and a calibration that
    // does NOT claim exactness.
    let pred = LINKAGE_CATALOG
        .split("aegis:pred_implements_grounded a aegis:Predicate")
        .nth(1)
        .expect("catalog no longer defines pred_implements_grounded")
        .split(" .\n")
        .next()
        .unwrap();
    assert!(pred.contains("aegis:matchType \"must-ground\""));
    assert!(pred.contains("aegis:tier \"embedding\""));
    assert!(
        pred.contains("aegis:groundingQuery"),
        "must-ground without an id set is refused at definition time"
    );
    let op = LINKAGE_CATALOG
        .split("aegis:op_implements_grounded a aegis:OperatingPoint")
        .nth(1)
        .expect("catalog no longer defines op_implements_grounded")
        .split(" .\n")
        .next()
        .unwrap();
    assert!(op.contains("aegis:kind \"threshold\""));
    // The declared tolerance, read as a number: substring checks would read
    // "0.05" as containing the "0.0" they forbid.
    let fp: f64 = op
        .split("aegis:falsePositiveTolerance ")
        .nth(1)
        .expect("the operating point must declare an FP tolerance")
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .expect("the FP tolerance must be numeric");
    assert!(
        fp > 0.0,
        "a classifier's operating point must declare a NONZERO FP tolerance, got {fp}"
    );
    let policy = LINKAGE_CATALOG
        .split("aegis:policy_implements_claim_grounded a aegis:Policy")
        .nth(1)
        .expect("catalog no longer defines policy_implements_claim_grounded")
        .split(" .\n")
        .next()
        .unwrap();
    assert!(
        policy.contains("aegis:verificationPoint \"PAA\"")
            && !policy.contains("aegis:effect \"deny\""),
        "the embedding tier is advisory/escalate — never a hard PAG deny"
    );
}

#[test]
fn the_linkage_catalog_lands_through_the_write_path_with_placement_on() {
    // The whole composition — embedding-tier must-ground predicate, threshold
    // operating point, soft PAA policy — must clear the definition-time
    // placement rules on the REAL write path, or the catalog documents a
    // policy no store would accept.
    use crate::store::Store;
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    crate::ingest_rdf(
        &mut store,
        LINKAGE_CATALOG.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .expect("the shipped linkage catalog must clear the placement rules");
}

#[test]
fn a_verdict_sealing_a_similarity_score_conforms() {
    // The similarity seal riding a verdict: score, threshold, model identity,
    // corpus watermark, the honest tier, and the typed linkage outcome. This
    // is the record shape design #1 requires of every similarity verdict.
    let data = format!(
        "{NS}\naegis:v a aegis:Verdict ; \
         aegis:predicateId \"implements-claim-grounded\" ; \
         aegis:targetRef \"commit:9cbc747\" ; aegis:outcome \"unsatisfied\" ; \
         aegis:evidenceHash \"sha256:00\" ; aegis:verifier \"quipu\" ; \
         aegis:signature \"sig:..\" ; aegis:tier \"embedding\" ; \
         aegis:similarityScore 0.42 ; aegis:similarityThreshold 0.75 ; \
         aegis:embeddingModel \"all-MiniLM-L6-v2\" ; \
         aegis:corpusWatermark \"tx:41\" ; \
         aegis:linkageOutcome \"cited-but-dissimilar\" .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        fb.conforms,
        "a sealed similarity verdict should conform: {:#?}",
        fb.results
    );
}

#[test]
fn linkage_outcome_out_of_enum_is_rejected() {
    // The three-outcome set is CLOSED, and deliberately has no "unevaluated"
    // value: a check that could not run records outcome "unknown" and no
    // linkageOutcome, keeping the existing unknown semantics single.
    for bad in ["unevaluated", "similar-ish"] {
        let data = format!(
            "{NS}\naegis:v a aegis:Verdict ; aegis:predicateId \"p\" ; \
             aegis:targetRef \"t\" ; aegis:outcome \"unknown\" ; \
             aegis:evidenceHash \"h\" ; aegis:verifier \"q\" ; \
             aegis:signature \"s\" ; aegis:linkageOutcome \"{bad}\" .\n"
        );
        assert!(
            !validate_shapes(SHAPES, &data).unwrap().conforms,
            "linkageOutcome \"{bad}\" must be outside the closed set"
        );
    }
}

#[test]
fn verdict_tier_was_widened_not_unhinged() {
    // The verdict tier now admits the grounded ladder (the tier rides the
    // verdict into the graph) — and still rejects everything else.
    let data = format!(
        "{NS}\naegis:v a aegis:Verdict ; aegis:predicateId \"p\" ; \
         aegis:targetRef \"t\" ; aegis:outcome \"satisfied\" ; \
         aegis:evidenceHash \"h\" ; aegis:verifier \"q\" ; \
         aegis:signature \"s\" ; aegis:tier \"vibes\" .\n"
    );
    assert!(
        !validate_shapes(SHAPES, &data).unwrap().conforms,
        "a verdict tier outside the widened enum must still be rejected"
    );
}

// ── Escalation precedent (quipu-8dk) ─────────────────────────────────────────
// Design #4: a minted DecisionRequest carries its nearest decided priors as
// reified, scored PrecedentLink nodes. The shape holds the falsifiability
// contract: no precedent without its score, no score without its method.

/// A decided prior + a fresh request carrying one precedent link, with the
/// link's fields drawn from `fields`. The happy path the rejections perturb.
fn precedent_fixture(link_fields: &str) -> String {
    format!(
        "{NS}\n\
         aegis:prior a aegis:DecisionRequest ; aegis:forPolicy \"http://ex/P1\" ; \
             aegis:forTarget \"http://ex/alpha-1\" ; aegis:expiresAt 1700000600 ; \
             aegis:evidenceHash \"sha256:aa\" .\n\
         aegis:req a aegis:DecisionRequest ; aegis:forPolicy \"http://ex/P1\" ; \
             aegis:forTarget \"http://ex/alpha-2\" ; aegis:expiresAt 1700000600 ; \
             aegis:evidenceHash \"sha256:bb\" ; aegis:precedent aegis:link .\n\
         aegis:link a aegis:PrecedentLink {link_fields} .\n"
    )
}

#[test]
fn a_decision_request_carrying_scored_precedent_conforms() {
    let data = precedent_fixture(
        "; aegis:precedentRequest aegis:prior ; aegis:similarityScore 0.83 ; \
         aegis:similarityMethod \"embedding:model.onnx\"",
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        fb.conforms,
        "a request with a fully-sealed precedent link should conform: {:#?}",
        fb.results
    );
}

#[test]
fn a_precedent_link_missing_any_seal_field_is_rejected() {
    // A precedent without its score is an insinuation; a score without its
    // method is unfalsifiable; a link pointing at nothing scores nothing.
    // Each omission must fail on its own.
    for missing in [
        // no precedentRequest
        "; aegis:similarityScore 0.83 ; aegis:similarityMethod \"embedding:m\"",
        // no similarityScore
        "; aegis:precedentRequest aegis:prior ; aegis:similarityMethod \"embedding:m\"",
        // no similarityMethod
        "; aegis:precedentRequest aegis:prior ; aegis:similarityScore 0.83",
    ] {
        let fb = validate_shapes(SHAPES, &precedent_fixture(missing)).unwrap();
        assert!(
            !fb.conforms,
            "a precedent link missing a seal field must be rejected: {missing}"
        );
    }
}

#[test]
fn precedent_must_point_at_a_precedent_link() {
    // sh:class aegis:PrecedentLink — a request citing an untyped node as
    // precedent is malformed, same discipline as assignsWorkflow.
    let data = format!(
        "{NS}\naegis:req a aegis:DecisionRequest ; aegis:forPolicy \"p\" ; \
         aegis:forTarget \"t\" ; aegis:expiresAt 1 ; aegis:evidenceHash \"h\" ; \
         aegis:precedent aegis:ghost .\n"
    );
    let fb = validate_shapes(SHAPES, &data).unwrap();
    assert!(
        !fb.conforms,
        "precedent must reference an aegis:PrecedentLink"
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

// ── Q-SARC-VOCAB: the whole governance plane, held closed ────────────────────

const DISPATCH_INVENTORY_SHAPES: &str = include_str!("../shapes/governance.ttl");

/// Every `aegis:` property a shape file constrains via `sh:path`.
fn constrained_properties(shapes: &str) -> Vec<String> {
    let mut out: Vec<String> = shapes
        .split("sh:path aegis:")
        .skip(1)
        .filter_map(|rest| {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether a declaration block ASSERTS an `rdfs:domain`.
///
/// Matches the predicate at the start of a line, not the substring: several
/// comments say "so no rdfs:domain" in prose, and a naive `contains` reads those
/// as the very declaration they are explaining the absence of.
fn declares_domain(block: &str) -> bool {
    block
        .lines()
        .any(|l| l.trim_start().starts_with("rdfs:domain "))
}

/// The declaration block for `aegis:<property>`, or `None` if undeclared.
fn declaration(property: &str) -> Option<&'static str> {
    AEGIS_PROPERTIES
        .split(&format!("aegis:{property} a owl:"))
        .nth(1)?
        .split(" .\n")
        .next()
}

#[test]
fn governance_plane_properties_are_all_described() {
    // The generalisation the SARC-field list above could not give: adding a
    // constrained property to the governance shapes without describing it fails
    // the build. A shape says what is VALID and never what a term MEANS, and the
    // authoring surface — an agent composing a policy over this catalog — reads
    // the second, not the first.
    //
    // Scope is the governance plane, deliberately. The estate ontology in
    // shapes/aegis-ontology.shapes.ttl is ~100 more properties whose intended
    // subjects are not all obvious from their shapes; sweeping them in would
    // mean asserting domains by guess, and a wrong domain MATERIALISES a wrong
    // rdf:type rather than merely documenting badly. Named here so the omission
    // is a scope decision rather than something nobody noticed.
    let mut undescribed: Vec<String> = Vec::new();
    for property in constrained_properties(SHAPES) {
        if declaration(&property).is_none() {
            undescribed.push(property);
        }
    }
    assert!(
        undescribed.is_empty(),
        "constrained by shapes/governance.ttl but not DECLARED in \
         shapes/aegis-properties.ttl: {undescribed:?}. Add each with a range, an \
         rdfs:comment saying what it MEANS, and a domain only if the name could \
         not reasonably belong to another class."
    );
    // Non-vacuity: if the extractor silently found nothing, the assertion above
    // would pass over an empty set and this test would be checking air.
    assert!(
        constrained_properties(SHAPES).len() > 40,
        "the property extractor found suspiciously few properties"
    );
    assert_eq!(
        DISPATCH_INVENTORY_SHAPES, SHAPES,
        "the ToolClass shape lives in governance.ttl; if it moves, widen this test"
    );
}

/// The `rdfs:comment` text of a declaration block, with escaped quotes intact.
///
/// Taken to the END of the block rather than to the next `"`, because several
/// comments quote enum values (`\"PAG\"`) and stopping at the first quote would
/// measure a fragment.
fn comment_of(block: &str) -> &str {
    block
        .split("rdfs:comment \"")
        .nth(1)
        .unwrap_or("")
        .trim_end()
        .trim_end_matches('"')
}

#[test]
fn every_description_says_more_than_its_own_label() {
    // A comment that restates the label documents nothing, and would satisfy the
    // test above while leaving the gap exactly where it was. The floor is low on
    // purpose — this is a check against empty gestures, not a word count.
    for property in constrained_properties(SHAPES) {
        let Some(block) = declaration(&property) else {
            continue;
        };
        assert!(
            block.contains("rdfs:comment"),
            "aegis:{property} must carry an rdfs:comment saying what it MEANS"
        );
        let comment = comment_of(block);
        let label = block
            .split("rdfs:label \"")
            .nth(1)
            .unwrap_or("")
            .split('"')
            .next()
            .unwrap_or("");
        assert!(
            !comment.trim_end_matches('.').eq_ignore_ascii_case(label),
            "aegis:{property}'s comment is its label restated"
        );
        assert!(
            comment.len() >= 80,
            "aegis:{property}'s comment is too short to be saying anything a \
             reader could not get from the name: {comment:?}"
        );
    }
}

#[test]
fn a_property_shared_across_classes_carries_no_domain() {
    // rdfs:domain is materialised. `aegis:tier` is on Selector, Predicate AND
    // Verdict, and `aegis:outcome` on Verdict AND Decision — a domain on either
    // would assert a type that is wrong two thirds of the time.
    for shared in [
        "tier",
        "outcome",
        "evidenceHash",
        "verifier",
        "name",
        "claim",
    ] {
        let block = declaration(shared).unwrap_or_else(|| panic!("aegis:{shared} is undeclared"));
        assert!(
            !declares_domain(block),
            "aegis:{shared} is carried by more than one class; a domain would \
             materialise a wrong rdf:type"
        );
    }
}

#[test]
fn the_two_meanings_of_gate_are_recorded_rather_than_hidden() {
    // `aegis:gate` is a Predicate's applicability condition AND the gate that
    // produced a Decision. That is a defect in the vocabulary; the declaration
    // has to say so, because a reader who meets only one of the two will write
    // code assuming it is the only one.
    let block = declaration("gate").expect("aegis:gate is declared");
    assert!(
        block.contains("TWO senses") || block.contains("two senses"),
        "aegis:gate's comment must record that the name carries two meanings"
    );
    assert!(!declares_domain(block), "and therefore carry no domain");
}

#[cfg(feature = "owl")]
#[test]
fn materialising_the_declarations_types_the_shipped_catalog_correctly() {
    // THE RISK THIS BATCH CARRIES. rdfs:domain is not documentation — quipu's
    // reasoner materialises it, so declaring `aegis:effect rdfs:domain
    // aegis:Policy` asserts `?x a aegis:Policy` for every subject holding an
    // effect. Get one domain wrong and the placement check starts demanding a
    // verification point of a Selector.
    //
    // So the test is not "do the axioms parse" — it is "run them over the
    // SHIPPED catalog and check what they inferred".
    use crate::store::Store;
    const TS: &str = "2026-01-01T00:00:00Z";
    const CATALOG: &str = include_str!("../shapes/policies/treesitter.ttl");

    let mut store = Store::open_in_memory().unwrap();
    crate::ingest_rdf(
        &mut store,
        CATALOG.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )
    .unwrap();

    let ontology = crate::owl::Ontology::from_turtle(AEGIS_PROPERTIES).unwrap();
    ontology.materialize(&mut store, TS).unwrap();

    let ns = crate::namespace::DEFAULT_BASE_NS;
    let typed = |class: &str| {
        let q = format!("PREFIX a: <{ns}> SELECT ?s WHERE {{ ?s a a:{class} }}");
        match crate::sparql::query(&store, &q).unwrap() {
            crate::sparql::QueryResult::Select { rows, .. } => rows.len(),
            _ => 0,
        }
    };

    // Selectors and Predicates must NOT have become Policies. They carry
    // aegis:name, aegis:evidenceSource and aegis:tier — three properties
    // deliberately left undomained — and if any of those had picked up a domain
    // by tidiness, every evidence atom in the catalog would now be a Policy and
    // the placement check would start demanding a verification point of it.
    let q = format!("PREFIX a: <{ns}> SELECT ?s WHERE {{ ?s a a:Selector . ?s a a:Policy }}");
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(&store, &q).unwrap()
    else {
        panic!("expected a SELECT result");
    };
    assert!(
        rows.is_empty(),
        "materialisation typed {} Selector(s) as Policies — a domain was \
         asserted on a property that more than one class carries",
        rows.len()
    );

    // ... and the same for Predicates, which carry a different overlapping set.
    let q = format!("PREFIX a: <{ns}> SELECT ?s WHERE {{ ?s a a:Predicate . ?s a a:Policy }}");
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(&store, &q).unwrap()
    else {
        panic!("expected a SELECT result");
    };
    assert!(rows.is_empty(), "a Predicate was materialised as a Policy");

    // NON-VACUITY: the run has to have inferred something, or the two assertions
    // above pass over an empty graph and prove nothing about the axioms.
    assert!(
        typed("Selector") > 0 && typed("Predicate") > 0 && typed("Policy") > 0,
        "the shipped catalog must hold policies AND evidence atoms for this \
         test to be measuring anything"
    );
}
