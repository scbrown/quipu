//! Without SHACL, library review reports unavailable validation explicitly.
#![cfg(not(feature = "shacl"))]

use quipu::share_pack_review::{ReviewInput, ShaclReview, render_report_markdown, review};

#[test]
fn unavailable_shacl_is_not_a_zero_violation_pass() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let old = root.join("tests/fixtures/share-review/base");
    let new = root.join("tests/fixtures/share-review/introduced");
    let old_quads = quipu::share_diff::read_payload(&old).unwrap();
    let new_quads = quipu::share_diff::read_payload(&new).unwrap();
    let shapes = std::fs::read_to_string(new.join("shapes.ttl")).unwrap();
    let report = review(&ReviewInput {
        old: &old_quads,
        new: &new_quads,
        old_shapes: Some(&shapes),
        new_shapes: Some(&shapes),
        decisions: None,
    })
    .unwrap();
    assert!(
        !report.diff.entities.is_empty(),
        "facts are the positive control"
    );
    assert!(matches!(
        report.shacl,
        ShaclReview::NotChecked {
            gate_must_fail: true,
            ..
        }
    ));
    assert_eq!(report.shacl.introduced(), None);
    assert!(render_report_markdown(&report).contains("NOT CHECKED"));
}
