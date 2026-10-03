use super::*;

#[test]
fn the_label_set_is_closed_and_distinct() {
    // An exhaustive match: a new variant missing here is a compile error, and
    // missing from ALL is the length assertion below.
    let listed = |k: WriteKind| match k {
        WriteKind::Episode
        | WriteKind::Knot
        | WriteKind::Promote
        | WriteKind::Import
        | WriteKind::Update
        | WriteKind::Set
        | WriteKind::Retract
        | WriteKind::Proposal
        | WriteKind::Overlay
        | WriteKind::GraphAdmin
        | WriteKind::Derive
        | WriteKind::Ontology
        | WriteKind::Reasoner
        | WriteKind::Migration
        | WriteKind::Verdict
        | WriteKind::Startup
        | WriteKind::Cli
        | WriteKind::Unclassified => WriteKind::ALL.contains(&k),
    };
    assert!(WriteKind::ALL.iter().all(|k| listed(*k)));
    let names: std::collections::BTreeSet<_> = WriteKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(
        names.len(),
        WriteKind::ALL.len(),
        "label values must be distinct"
    );
}

#[test]
fn every_write_endpoint_has_a_named_kind() {
    // Total over the server's own write list: adding a write route without
    // classifying it fails here instead of silently landing in `unclassified`.
    let missing: Vec<_> = crate::http_auth::WRITE_ENDPOINTS
        .iter()
        .filter(|e| WriteKind::for_route(e).is_none())
        .collect();
    assert!(missing.is_empty(), "unclassified write routes: {missing:?}");
    // Control: a read route is not a write kind.
    assert_eq!(WriteKind::for_route("/query"), None);
    // malcolm: promote is its own kind, for both promotion paths.
    assert_eq!(
        WriteKind::for_route("/knot/promote"),
        Some(WriteKind::Promote)
    );
    assert_eq!(
        WriteKind::for_route("/import/promote"),
        Some(WriteKind::Promote)
    );
}

#[test]
fn engine_writers_override_the_request_scope_and_scopes_nest() {
    scoped(Some(WriteKind::Ontology), || {
        assert_eq!(classify(None, None), WriteKind::Ontology);
        // Inference materialized during an /ontology load is reasoner work.
        assert_eq!(classify(Some("reasoner"), None), WriteKind::Reasoner);
        assert_eq!(
            classify(None, Some(crate::store::inferred::PLANE_SOURCE)),
            WriteKind::Reasoner
        );
        assert_eq!(
            classify(None, Some(crate::store::inferred::MIGRATE_SOURCE)),
            WriteKind::Migration
        );
        // An inner scope wins and is restored on exit; None keeps the outer.
        scoped(Some(WriteKind::Verdict), || {
            assert_eq!(classify(Some("quipu"), None), WriteKind::Verdict);
        });
        scoped(None, || assert_eq!(current(), Some(WriteKind::Ontology)));
        assert_eq!(current(), Some(WriteKind::Ontology));
    });
}
