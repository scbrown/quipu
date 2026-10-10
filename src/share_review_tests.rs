use super::*;
use crate::share::{ShareOptions, share};
use crate::share_import::{PromoteImportRequest, ShareImportRequest};
const TS: &str = "2026-10-09T00:00:00Z";
fn request() -> ShareImportRequest {
    let mut source = Store::open_in_memory().unwrap();
    crate::share_scrub::seed_test_catalogue(&mut source);
    crate::rdf::ingest_rdf(&mut source,&b"<https://example.org/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://foreign.example/Unknown> .\n"[..],oxrdfio::RdfFormat::NTriples,None,TS,None,None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("share");
    share(
        &source,
        path.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let read = |name: &str| std::fs::read_to_string(path.join(name)).unwrap();
    ShareImportRequest {
        manifest: serde_json::from_str(&read("manifest.json")).unwrap(),
        export_ntriples: read("export.nt"),
        shapes_turtle: read("shapes.ttl"),
        queries_turtle: None,
        query_namespace: None,
        replace_queries: false,
        source: "https://example.org/share".into(),
        actor: Some("claimed-producer".into()),
        accept_exact: false,
        destination: Default::default(),
        #[cfg(not(target_arch = "wasm32"))]
        attestation: None,
    }
}
fn receiver() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    crate::share_scrub::seed_test_catalogue(&mut store);
    store
}
fn events(store: &Store, kind: &str) -> Vec<crate::store::events::EventRow> {
    store
        .events_after(0, 100, Some(&[kind.to_string()]), None)
        .unwrap()
}
fn adopt(store: &Store) {
    store.load_shapes("foreign-reviewed", "@prefix sh: <http://www.w3.org/ns/shacl#> . <https://example.org/shape> a sh:NodeShape ; sh:targetClass <https://foreign.example/Unknown> .",TS).unwrap();
}
#[test]
fn quarantine_replay_ages_once_then_adoption_and_promotion_resolve() {
    let mut store = receiver();
    let request = request();
    let initial = import_share(&mut store, &request, TS, Some("authenticated-receiver")).unwrap();
    assert!(!initial.promotion.eligible);
    let first = events(&store, "import.quarantined");
    assert_eq!(first.len(), 1);
    let payload: Value = serde_json::from_str(&first[0].payload).unwrap();
    assert_eq!(payload["source"], request.source);
    assert_eq!(payload["actor"], "authenticated-receiver");
    assert_eq!(payload["claimed_actor"], "claimed-producer");
    assert_eq!(
        payload["unknown_types"][0],
        "https://foreign.example/Unknown"
    );
    assert!(payload["size_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        import_share(&mut store, &request, TS, None)
            .unwrap()
            .outcome,
        "unchanged"
    );
    assert_eq!(events(&store, "import.quarantined").len(), 1);
    notify_due(&mut store, "", 10, "2026-10-09T00:00:59Z", 60, "reviewer").unwrap();
    assert!(events(&store, "import.review_due").is_empty());
    notify_due(&mut store, "", 10, "2026-10-09T00:01:00Z", 60, "reviewer").unwrap();
    notify_due(&mut store, "", 10, "2026-10-09T00:02:00Z", 60, "reviewer").unwrap();
    assert_eq!(events(&store, "import.review_due").len(), 1);
    assert!(
        promote_import(
            &mut store,
            &PromoteImportRequest {
                share_id: request.manifest.share_id.clone(),
                actor: None
            },
            TS,
            None
        )
        .is_err()
    );
    assert!(events(&store, "import.promoted").is_empty());
    adopt(&store);
    assert!(
        import_share(&mut store, &request, TS, None)
            .unwrap()
            .promotion
            .eligible
    );
    assert_eq!(events(&store, "import.adopted").len(), 1);
    let promote = PromoteImportRequest {
        share_id: request.manifest.share_id.clone(),
        actor: Some("claim".into()),
    };
    promote_import(&mut store, &promote, TS, Some("reviewed-actor")).unwrap();
    promote_import(&mut store, &promote, TS, Some("reviewed-actor")).unwrap();
    assert_eq!(events(&store, "import.promoted").len(), 1);
    import_share(&mut store, &request, TS, None).unwrap();
    assert_eq!(
        state(&store, &request.manifest.share_id)
            .unwrap()
            .as_deref(),
        Some("promoted")
    );
    assert!(
        pending(&store, "", 10, TS).unwrap()["reviews"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn event_failure_rolls_back_import_graph_review_and_cached_terms() {
    let mut store = receiver();
    let request = request();
    store.conn.execute_batch("CREATE TRIGGER fail_import_event BEFORE INSERT ON events WHEN NEW.type='import.quarantined' BEGIN SELECT RAISE(ABORT,'event failure'); END").unwrap();
    assert!(import_share(&mut store, &request, TS, None).is_err());
    assert!(state(&store, &request.manifest.share_id).unwrap().is_none());
    assert!(store.lookup("https://example.org/a").unwrap().is_none());
    assert!(events(&store, "import.quarantined").is_empty());
    store
        .conn
        .execute_batch("DROP TRIGGER fail_import_event")
        .unwrap();
    assert_eq!(
        import_share(&mut store, &request, TS, None)
            .unwrap()
            .outcome,
        "quarantined"
    );
    assert_eq!(events(&store, "import.quarantined").len(), 1);
}
#[test]
fn rejected_expired_reopened_state_survives_event_pruning_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reviews.db");
    let request = request();
    let id = &request.manifest.share_id;
    {
        let mut store = Store::open(path.to_str().unwrap()).unwrap();
        crate::share_scrub::seed_test_catalogue(&mut store);
        import_share(&mut store, &request, TS, None).unwrap();
        assert!(decide(&mut store, id, "rejected", "", "reason", TS).is_err());
        decide(
            &mut store,
            id,
            "rejected",
            "reviewer",
            "untrusted authority",
            TS,
        )
        .unwrap();
        assert!(import_share(&mut store, &request, TS, None).is_err());
        assert_eq!(events(&store, "import.rejected").len(), 1);
        assert!(
            store
                .lookup(&format!("urn:quipu:import:quarantine:{}", &id[7..]))
                .unwrap()
                .is_some()
        );
        store.prune_events("2026-10-10T00:00:00Z").unwrap();
    }
    let mut store = Store::open(path.to_str().unwrap()).unwrap();
    assert_eq!(state(&store, id).unwrap().as_deref(), Some("rejected"));
    decide(
        &mut store,
        id,
        "reopen",
        "reviewer",
        "reconsider authority",
        TS,
    )
    .unwrap();
    let page = pending(&store, "", 1, "2026-10-09T00:01:00Z").unwrap();
    assert_eq!(page["reviews"].as_array().unwrap().len(), 1);
    notify_due(&mut store, "", 1, "2026-10-09T00:01:00Z", 60, "desk").unwrap();
    assert_eq!(events(&store, "import.review_due").len(), 1);
    decide(
        &mut store,
        id,
        "expired",
        "reviewer",
        "policy expiry decision",
        TS,
    )
    .unwrap();
    assert_eq!(events(&store, "import.expired").len(), 1);
    assert!(
        pending(&store, "", 1, TS).unwrap()["reviews"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn promotion_event_failure_leaves_root_unchanged_and_can_retry() {
    let mut store = receiver();
    let request = request();
    adopt(&store);
    import_share(&mut store, &request, TS, None).unwrap();
    let before = store.current_facts().unwrap().len();
    let promote = PromoteImportRequest {
        share_id: request.manifest.share_id.clone(),
        actor: Some("operator".into()),
    };
    store.conn.execute_batch("CREATE TRIGGER fail_promotion_event BEFORE INSERT ON events WHEN NEW.type='import.promoted' BEGIN SELECT RAISE(ABORT,'event failure'); END").unwrap();
    assert!(promote_import(&mut store, &promote, TS, None).is_err());
    assert_eq!(store.current_facts().unwrap().len(), before);
    assert_eq!(
        state(&store, &request.manifest.share_id)
            .unwrap()
            .as_deref(),
        Some("staged")
    );
    assert!(events(&store, "import.promoted").is_empty());
    store
        .conn
        .execute_batch("DROP TRIGGER fail_promotion_event")
        .unwrap();
    promote_import(&mut store, &promote, TS, None).unwrap();
    assert!(store.current_facts().unwrap().len() > before);
    let payload: Value =
        serde_json::from_str(&events(&store, "import.promoted")[0].payload).unwrap();
    assert_eq!(payload["source"], request.source);
    assert_eq!(payload["claimed_actor"], "operator");
    assert!(payload["size_bytes"].as_u64().unwrap() > 0);
}
#[test]
fn notification_failure_does_not_advance_policy_and_invalid_policy_is_refused() {
    let mut store = receiver();
    let request = request();
    import_share(&mut store, &request, TS, None).unwrap();
    assert!(notify_due(&mut store, "", 1, TS, 0, "reviewer").is_err());
    assert!(notify_due(&mut store, "", 1, TS, 60, "").is_err());
    store.conn.execute_batch("CREATE TRIGGER fail_age_event BEFORE INSERT ON events WHEN NEW.type='import.review_due' BEGIN SELECT RAISE(ABORT,'event failure'); END").unwrap();
    assert!(notify_due(&mut store, "", 1, "2026-10-09T00:01:00Z", 60, "reviewer").is_err());
    store
        .conn
        .execute_batch("DROP TRIGGER fail_age_event")
        .unwrap();
    notify_due(&mut store, "", 1, "2026-10-09T00:01:00Z", 60, "reviewer").unwrap();
    assert_eq!(events(&store, "import.review_due").len(), 1);
    let payload: Value =
        serde_json::from_str(&events(&store, "import.review_due")[0].payload).unwrap();
    assert_eq!(payload["route"], "reviewer");
    assert_eq!(payload["waiting_seconds"], 60);
}
