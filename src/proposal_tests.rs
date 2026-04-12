use super::*;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

#[test]
fn insert_and_get_proposal() {
    let store = test_store();
    let id = store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Shape,
            target: "ex:PersonShape",
            diff: "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:PersonShape a sh:NodeShape .",
            rationale: Some("Need Person shape for validation"),
            proposer: "agent/tester",
            trigger_ref: Some("val-fail-001"),
            created_at: "2026-04-12T00:00:00Z",
        })
        .unwrap();

    let proposal = store.get_proposal(id).unwrap().unwrap();
    assert_eq!(proposal.kind, ProposalKind::Shape);
    assert_eq!(proposal.target, "ex:PersonShape");
    assert_eq!(proposal.proposer, "agent/tester");
    assert_eq!(proposal.status, ProposalStatus::Pending);
    assert_eq!(proposal.trigger_ref.as_deref(), Some("val-fail-001"));
}

#[test]
fn list_proposals_all_and_filtered() {
    let store = test_store();
    store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Shape,
            target: "ex:A",
            diff: "turtle-a",
            rationale: None,
            proposer: "agent/a",
            trigger_ref: None,
            created_at: "2026-04-12T01:00:00Z",
        })
        .unwrap();
    store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Property,
            target: "ex:B",
            diff: "turtle-b",
            rationale: None,
            proposer: "agent/b",
            trigger_ref: None,
            created_at: "2026-04-12T02:00:00Z",
        })
        .unwrap();

    let all = store.list_proposals(None).unwrap();
    assert_eq!(all.len(), 2);

    let pending = store
        .list_proposals(Some(&ProposalStatus::Pending))
        .unwrap();
    assert_eq!(pending.len(), 2);

    let accepted = store
        .list_proposals(Some(&ProposalStatus::Accepted))
        .unwrap();
    assert!(accepted.is_empty());
}

#[test]
fn accept_shape_proposal_validates_turtle() {
    let store = test_store();
    let valid_turtle = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .
"#;
    let id = store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Shape,
            target: "PersonShape",
            diff: valid_turtle,
            rationale: Some("Add person validation"),
            proposer: "agent/tester",
            trigger_ref: None,
            created_at: "2026-04-12T00:00:00Z",
        })
        .unwrap();

    let accepted = store
        .accept_proposal(id, "aegis/crew/braino", "2026-04-12T01:00:00Z", None)
        .unwrap();
    assert_eq!(accepted.status, ProposalStatus::Accepted);
    assert_eq!(accepted.decided_by.as_deref(), Some("aegis/crew/braino"));

    // Shape should now be in the shapes table.
    let shapes = store.list_shapes().unwrap();
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].0, "PersonShape");
}

#[test]
fn accept_invalid_turtle_keeps_pending() {
    let store = test_store();
    let id = store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Shape,
            target: "BadShape",
            diff: "this is not valid turtle {{{",
            rationale: None,
            proposer: "agent/tester",
            trigger_ref: None,
            created_at: "2026-04-12T00:00:00Z",
        })
        .unwrap();

    let err = store
        .accept_proposal(id, "approver", "2026-04-12T01:00:00Z", None)
        .unwrap_err();
    assert!(err.to_string().contains("invalid"));

    // Proposal should still be pending.
    let proposal = store.get_proposal(id).unwrap().unwrap();
    assert_eq!(proposal.status, ProposalStatus::Pending);
}

#[test]
fn reject_proposal() {
    let store = test_store();
    let id = store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Class,
            target: "ex:Widget",
            diff: "turtle-diff",
            rationale: None,
            proposer: "agent/tester",
            trigger_ref: None,
            created_at: "2026-04-12T00:00:00Z",
        })
        .unwrap();

    let rejected = store
        .reject_proposal(
            id,
            "aegis/crew/braino",
            "2026-04-12T01:00:00Z",
            "Not needed",
        )
        .unwrap();
    assert_eq!(rejected.status, ProposalStatus::Rejected);
    assert_eq!(rejected.decision_note.as_deref(), Some("Not needed"));

    // Shape table should be unchanged.
    let shapes = store.list_shapes().unwrap();
    assert!(shapes.is_empty());
}

#[test]
fn cannot_accept_already_rejected() {
    let store = test_store();
    let id = store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Shape,
            target: "ex:X",
            diff: "turtle",
            rationale: None,
            proposer: "agent/a",
            trigger_ref: None,
            created_at: "2026-04-12T00:00:00Z",
        })
        .unwrap();
    store
        .reject_proposal(id, "approver", "2026-04-12T01:00:00Z", "no")
        .unwrap();
    let err = store
        .accept_proposal(id, "approver", "2026-04-12T02:00:00Z", None)
        .unwrap_err();
    assert!(err.to_string().contains("already"));
}

#[test]
fn accept_then_write_passes_validation() {
    let store = test_store();
    let shape_turtle = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .
"#;
    // Submit and accept the proposal.
    let id = store
        .insert_proposal(&NewProposal {
            kind: &ProposalKind::Shape,
            target: "PersonShape",
            diff: shape_turtle,
            rationale: None,
            proposer: "agent/tester",
            trigger_ref: None,
            created_at: "2026-04-12T00:00:00Z",
        })
        .unwrap();
    store
        .accept_proposal(id, "approver", "2026-04-12T01:00:00Z", None)
        .unwrap();

    // Now validate data that conforms to the accepted shape.
    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:name "Alice" .
"#;
    let combined = store.get_combined_shapes().unwrap().unwrap();
    let feedback = crate::shacl::validate_shapes(&combined, data).unwrap();
    assert!(
        feedback.conforms,
        "data should conform after accepting shape proposal"
    );

    // Data that violates should fail.
    let bad_data = r#"
@prefix ex: <http://example.org/> .
ex:bob a ex:Person .
"#;
    let feedback = crate::shacl::validate_shapes(&combined, bad_data).unwrap();
    assert!(!feedback.conforms, "data missing name should fail");
}
