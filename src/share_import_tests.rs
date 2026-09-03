#[cfg(test)]
mod tests {
    use super::*;

    const TS: &str = "2026-08-29T00:00:00Z";

    fn request(store: &Store) -> (tempfile::TempDir, ShareImportRequest) {
        let dir = tempfile::tempdir().unwrap();
        let share_dir = dir.path().join("share");
        crate::share::share(
            store,
            share_dir.to_str().unwrap(),
            &crate::share::ShareOptions {
                no_shapes: true,
                ..Default::default()
            },
        )
        .unwrap();
        let read = |name: &str| std::fs::read_to_string(share_dir.join(name)).unwrap();
        let request = ShareImportRequest {
            manifest: serde_json::from_str(&read("manifest.json")).unwrap(),
            export_ntriples: read("export.nt"),
            shapes_turtle: read("shapes.ttl"),
            source: "https://example.org/alice/share".into(),
            actor: Some("alice".into()),
        };
        (dir, request)
    }

    #[test]
    fn verified_share_stages_then_promotes_explicitly() {
        let mut source = Store::open_in_memory().unwrap();
        crate::rdf::ingest_rdf(
            &mut source,
            &b"<https://example.org/alice> <http://www.w3.org/2000/01/rdf-schema#label> \"Alice\" .\n"[..],
            RdfFormat::NTriples,
            None,
            TS,
            None,
            Some("test"),
        ).unwrap();
        let (_dir, request) = request(&source);
        let mut target = Store::open_in_memory().unwrap();
        let staged = import_share(&mut target, &request, TS, Some("legacy-shared-bearer")).unwrap();
        assert_eq!(staged.outcome, "staged");
        assert!(staged.promotion.eligible);
        assert!(target.lookup(&staged.staging_graph).unwrap().is_some());

        let promoted = promote_import(
            &mut target,
            &PromoteImportRequest {
                share_id: staged.share_id,
                actor: Some("reviewer".into()),
            },
            TS,
            Some("legacy-shared-bearer"),
        )
        .unwrap();
        assert_eq!(promoted.outcome, "promoted");
        assert_eq!(promoted.triples, 1);
        let transactions = target.list_transactions().unwrap();
        let import_tx = transactions
            .iter()
            .find(|tx| {
                tx.source
                    .as_deref()
                    .is_some_and(|s| s.starts_with("share-import:"))
            })
            .unwrap();
        assert_eq!(import_tx.actor.as_deref(), Some("legacy-shared-bearer"));
        assert!(
            import_tx
                .source
                .as_deref()
                .unwrap()
                .contains("claimed-actor=alice")
        );
        let promotion_tx = transactions
            .iter()
            .find(|tx| {
                tx.source
                    .as_deref()
                    .is_some_and(|s| s.starts_with("share-promotion:"))
            })
            .unwrap();
        assert_eq!(promotion_tx.actor.as_deref(), Some("legacy-shared-bearer"));
        assert!(
            promotion_tx
                .source
                .as_deref()
                .unwrap()
                .contains("claimed-actor=reviewer")
        );
    }

    #[test]
    fn hash_mismatch_writes_nothing() {
        let source = Store::open_in_memory().unwrap();
        let (_dir, mut request) = request(&source);
        request.export_ntriples.push_str("# tampered\n");
        let mut target = Store::open_in_memory().unwrap();
        assert!(import_share(&mut target, &request, TS, None).is_err());
        assert!(
            target
                .lookup(&staging_graph(&request.manifest.share_id, false).unwrap())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn off_vocabulary_share_is_quarantined_and_cannot_promote() {
        let mut source = Store::open_in_memory().unwrap();
        crate::rdf::ingest_rdf(
            &mut source,
            &b"<https://example.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://foreign.example/Unknown> .\n"[..],
            RdfFormat::NTriples,
            None,
            TS,
            None,
            Some("test"),
        ).unwrap();
        let (_dir, request) = request(&source);
        let mut target = Store::open_in_memory().unwrap();
        let result = import_share(&mut target, &request, TS, None).unwrap();
        assert_eq!(result.outcome, "quarantined");
        assert_eq!(result.triples.quarantined, 1);
        assert_eq!(result.promotion.blockers, vec!["off_vocabulary"]);
        assert!(
            promote_import(
                &mut target,
                &PromoteImportRequest {
                    share_id: result.share_id,
                    actor: None,
                },
                TS,
                None,
            )
            .is_err()
        );
    }
}
