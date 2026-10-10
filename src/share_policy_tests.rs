//! Real snapshot staging tests for locally authorized emit policy.
use super::*;
use crate::share::{ShareDestination, ShareOptions};

const TS: &str = "2026-10-09T00:00:00Z";
const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix quipu: <http://quipu.dev/ns#> .
<urn:PersonShape> a sh:NodeShape ;
 quipu:onViolation "emit" ;
 sh:targetClass <urn:Person> ;
 sh:property [ sh:path <urn:name> ; sh:minCount 1 ; sh:message "name required" ] .
"#;

fn pack(data: &str, shapes: &str) -> ShareImportRequest {
    let mut source = Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut source,
        data.as_bytes(),
        RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )
    .unwrap();
    source.load_shapes("producer", shapes, TS).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pack");
    crate::share::share(
        &source,
        path.to_str().unwrap(),
        &ShareOptions {
            destination: ShareDestination::Internal,
            ..Default::default()
        },
    )
    .unwrap();
    let mut request = crate::share_transport::read_local(path.to_str().unwrap()).unwrap();
    request.destination = ShareDestination::Internal;
    request
}

fn snapshots() -> Vec<ShareImportRequest> {
    vec![
        pack("<urn:alice> a <urn:Person> .", SHAPES),
        pack("<urn:bob> a <urn:Person> .", SHAPES),
    ]
}

#[test]
fn import_local_emit_stages_with_report_but_never_promotes_root() {
    let request = pack("<urn:alice> a <urn:Person> .", SHAPES);
    let mut store = Store::open_in_memory().unwrap();
    store.load_shapes("local", SHAPES, TS).unwrap();
    let result = crate::share_import::import_share(&mut store, &request, TS, None).unwrap();
    assert_eq!(result.outcome, "staged");
    assert!(!result.validation.conforms);
    assert!(!result.validation.blocking);
    assert!(result.promotion.eligible);
    assert_eq!(result.validation.report["advisory_count"], 1);
    assert_eq!(result.validation.report["violations"], 1);
    assert_eq!(
        result.validation.report["results"][0]["message"],
        "name required"
    );
    assert!(store.current_facts().unwrap().is_empty());
    let graph = store.lookup(&result.staging_graph).unwrap().unwrap();
    assert!(!store.current_facts_in_graph(graph).unwrap().is_empty());
}

#[test]
fn hostile_bundle_emit_cannot_downgrade_local_reject_on_import_or_compose() {
    let requests = snapshots();
    let reject = SHAPES.replace(" quipu:onViolation \"emit\" ;\n", "");
    let mut store = Store::open_in_memory().unwrap();
    store.load_shapes("local", &reject, TS).unwrap();
    let before = store.get_combined_shapes().unwrap();
    let imported = crate::share_import::import_share(&mut store, &requests[0], TS, None).unwrap();
    assert_eq!(imported.outcome, "quarantined");
    assert!(!imported.promotion.eligible);
    assert!(imported.validation.blocking);
    assert_eq!(imported.validation.report["advisory_count"], 0);
    assert_eq!(store.get_combined_shapes().unwrap(), before);
    let composed = compose(&mut store, &requests, Some(0), TS, None).unwrap();
    assert_eq!(composed.outcome, "quarantined");
    assert_eq!(
        composed.validation["admission_policy"]["source"],
        "local_loaded"
    );
    assert_eq!(composed.validation["advisory_count"], 0);
    assert_eq!(store.get_combined_shapes().unwrap(), before);
    assert!(store.current_facts().unwrap().is_empty());
}

#[test]
fn compose_default_bundles_do_not_authorize_emit_but_explicit_selection_does() {
    let requests = snapshots();
    let mut store = Store::open_in_memory().unwrap();
    let default = compose(&mut store, &requests, None, TS, None).unwrap();
    assert_eq!(default.outcome, "quarantined");
    assert_eq!(
        default.validation["admission_policy"]["source"],
        "default_bundle"
    );
    assert_eq!(
        default.validation["admission_policy"]["emit_authorized"],
        false
    );
    assert_eq!(default.validation["advisory_count"], 0);
    let explicit = compose(&mut store, &requests, Some(0), TS, None).unwrap();
    assert_eq!(explicit.outcome, "composed");
    assert_eq!(explicit.validation["conforms"], false);
    assert_eq!(explicit.validation["blocking"], false);
    assert_eq!(explicit.validation["advisory_count"], 2);
    assert_eq!(
        explicit.validation["admission_policy"]["source"],
        "explicit_bundle"
    );
    assert_eq!(
        explicit.validation["admission_policy"]["share_id"],
        requests[0].manifest.share_id
    );
    assert_ne!(
        default.dataset, explicit.dataset,
        "admission authority is part of identity"
    );
    assert!(store.current_facts().unwrap().is_empty());
}

#[test]
fn compose_local_emit_authorizes_without_installing_foreign_policy() {
    let requests = snapshots();
    let mut store = Store::open_in_memory().unwrap();
    store.load_shapes("local", SHAPES, TS).unwrap();
    let before = store.get_combined_shapes().unwrap();
    let result = compose(&mut store, &requests, None, TS, None).unwrap();
    assert_eq!(result.outcome, "composed");
    assert_eq!(result.validation["conforms"], false);
    assert_eq!(result.validation["advisory_count"], 2);
    assert_eq!(
        result.validation["admission_policy"]["source"],
        "local_loaded"
    );
    assert_eq!(store.get_combined_shapes().unwrap(), before);
    assert!(store.current_facts().unwrap().is_empty());
}

#[test]
fn emit_does_not_bypass_vocabulary_with_a_governed_positive_control() {
    let mut store = Store::open_in_memory().unwrap();
    store.load_shapes("local", SHAPES, TS).unwrap();
    let known = pack("<urn:alice> a <urn:Person> .", SHAPES);
    let good = crate::share_import::import_share(&mut store, &known, TS, None).unwrap();
    assert_eq!(good.outcome, "staged");
    let unknown = pack("<urn:outsider> a <urn:Unknown> .", SHAPES);
    let bad = crate::share_import::import_share(&mut store, &unknown, TS, None).unwrap();
    assert_eq!(bad.outcome, "quarantined");
    assert!(!bad.promotion.eligible);
    assert!(!bad.validation.off_vocabulary.is_empty());
    let composed = compose(&mut store, &[known, unknown], Some(0), TS, None).unwrap();
    assert_eq!(composed.outcome, "quarantined");
    assert_eq!(composed.validation["blocking"], true);
    assert!(store.current_facts().unwrap().is_empty());
}

#[test]
fn invalid_policy_refuses_atomically_even_when_the_data_conforms() {
    let invalid = SHAPES.replace("\"emit\"", "\"typo\"");
    let requests = vec![pack(
        "<urn:alice> a <urn:Person>; <urn:name> \"Alice\" .",
        &invalid,
    )];
    let mut store = Store::open_in_memory().unwrap();
    let before = crate::tool_graph_list(&store, &serde_json::json!({})).unwrap();
    assert!(compose(&mut store, &requests, Some(0), TS, None).is_err());
    assert!(store.current_facts().unwrap().is_empty());
    assert_eq!(
        crate::tool_graph_list(&store, &serde_json::json!({})).unwrap(),
        before
    );
}

#[test]
fn large_reports_keep_all_diagnostics_alongside_the_bounded_display() {
    let data = (0..70)
        .map(|id| format!("<urn:person-{id}> a <urn:Person> .\n"))
        .collect::<String>();
    let requests = vec![
        pack(&data, SHAPES),
        pack("<urn:good> a <urn:Person>; <urn:name> \"Good\" .", SHAPES),
    ];
    let mut store = Store::open_in_memory().unwrap();
    let result = compose(&mut store, &requests, Some(0), TS, None).unwrap();
    let report = &result.validation;
    assert_eq!(result.outcome, "composed");
    assert_eq!(report["results_total"], 70);
    assert_eq!(report["results_truncated"], true);
    assert_eq!(report["results"].as_array().unwrap().len(), 40);
    assert_eq!(report["complete_results"].as_array().unwrap().len(), 70);
    assert_eq!(report["advisory_results"].as_array().unwrap().len(), 70);
    assert!(store.current_facts().unwrap().is_empty());
}
