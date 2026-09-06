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
            accept_exact: false,
            destination: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            attestation: None,
        };
        (dir, request)
    }

    /// aegis-i48b9w: an exact canonical-name match must be PROPOSED, not applied.
    ///
    /// Both arms in one test on purpose. A test that only proved the new default could
    /// not tell a working flag from a flag that is ignored — and the flag is the whole
    /// reversibility argument for landing this.
    #[test]
    fn exact_name_match_is_proposed_by_default_and_applied_only_on_request() {
        // Two stores that independently named an entity "Alice" — different IRIs, same
        // label. This is the collision the rewrite used to resolve silently.
        let plant = |iri: &str| {
            let mut st = Store::open_in_memory().unwrap();
            let nt = format!(
                "<{iri}> <http://www.w3.org/2000/01/rdf-schema#label> \"Alice\" .\n"
            );
            crate::rdf::ingest_rdf(
                &mut st, nt.as_bytes(), RdfFormat::NTriples, None, TS, None, Some("test"),
            )
            .unwrap();
            st
        };
        let source = plant("https://example.org/alice");
        let (_dir, mut request) = request(&source);

        // ARM 1 — default. The foreign IRI SURVIVES and the match is reported.
        let mut target = plant("https://local.example/alice");
        let staged = import_share(&mut target, &request, TS, Some("legacy-shared-bearer")).unwrap();
        let exact = &staged.resolution.exact_merges;
        assert_eq!(exact.len(), 1, "the exact match must still be REPORTED");
        assert_eq!(exact[0].foreign, "https://example.org/alice");
        assert_eq!(exact[0].score, Some(1.0), "reported with its score, for bulk-accept");
        assert_eq!(exact[0].matched_on.as_deref(), Some("canonical_name:exact"));
        // `lookup` answers with the TERM ID for an IRI, so "is this IRI known to the
        // store" is exactly `is_some()`. (My first spelling formatted the returned i64
        // and searched it for the IRI — an assertion that could never pass, and would
        // have read as the fix not working.)
        assert!(
            target.lookup("https://example.org/alice").unwrap().is_some(),
            "the FOREIGN IRI must survive import — rewriting it destroys the identity \
             before anything can record it (aegis-i48b9w)"
        );

        // ARM 2 — opt in. Today's fast path is still reachable, explicitly.
        request.accept_exact = true;
        let mut target2 = plant("https://local.example/alice");
        import_share(&mut target2, &request, TS, Some("legacy-shared-bearer")).unwrap();
        assert!(
            target2.lookup("https://example.org/alice").unwrap().is_none(),
            "with accept_exact the foreign IRI IS rewritten away — otherwise the flag is \
             inert and arm 1 proves nothing"
        );
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

    /// A store that both carries the block-tier rule and violates it.
    fn leaky_store() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        crate::rdf::ingest_rdf(
            &mut store,
            &br#"@prefix aegis: <http://aegis.gastown.local/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
aegis:private-host-rule a aegis:InternalIdentifierPattern ;
    rdfs:label "private host" ;
    aegis:regex "private[.]example" ;
    aegis:enforcementTier "block" .
<urn:leak> <urn:p> "private.example" .
"#[..],
            oxrdfio::RdfFormat::Turtle,
            None,
            TS,
            None,
            None,
        )
        .unwrap();
        store
    }

    /// Build an import request from a share of `source`, produced internally.
    fn internal_request(source: &Store) -> (tempfile::TempDir, ShareImportRequest) {
        let dir = tempfile::tempdir().unwrap();
        let share_dir = dir.path().join("share");
        crate::share::share(
            source,
            share_dir.to_str().unwrap(),
            &crate::share::ShareOptions {
                no_shapes: true,
                destination: crate::share::ShareDestination::Internal,
                ..Default::default()
            },
        )
        .unwrap();
        let request = crate::share_transport::read_local(share_dir.to_str().unwrap()).unwrap();
        assert_eq!(
            request.manifest.destination,
            Some(crate::share::ShareDestination::Internal),
            "fixture must produce a stamped share, or the gate under test is untested"
        );
        (dir, request)
    }

    /// The marker travels: an internal share does not import silently into a
    /// store that would later publish it (aegis-auw0o7).
    #[test]
    fn importing_an_internal_share_is_refused_unless_the_operator_declares_internal() {
        let (_dir, request) = internal_request(&leaky_store());
        let mut store = leaky_store();

        let error = import_share(&mut store, &request, TS, None).unwrap_err();
        assert!(error.to_string().contains("private host"), "{error}");
        assert!(
            error.to_string().contains("destination=internal"),
            "the refusal must say WHY this share was checked: {error}"
        );
        assert!(
            error.to_string().contains("--destination internal"),
            "the refusal must name the flag: {error}"
        );
        // A refusal before staging leaves nothing behind.
        assert!(
            store.lookup(&request.manifest.share_id).unwrap().is_none(),
            "a refused import staged a graph"
        );

        // The paired arm: the same bytes, with the operator declaring internal.
        let mut declared = request.clone();
        declared.destination = crate::share::ShareDestination::Internal;
        let result = import_share(&mut store, &declared, TS, None).unwrap();
        assert_eq!(result.share_id, request.manifest.share_id);
    }

    /// The gate checks the BYTES, not the marker.
    ///
    /// A share stamped internal out of caution, carrying nothing an outward
    /// share could not carry, imports normally. Refusing it would make the flag
    /// a quarantine rather than a scrub exemption, and would push producers
    /// toward not stamping at all — which is the one outcome that loses the
    /// marker entirely.
    #[test]
    fn an_internal_share_whose_payload_is_clean_imports_without_the_flag() {
        let mut clean = Store::open_in_memory().unwrap();
        crate::rdf::ingest_rdf(
            &mut clean,
            &b"<urn:ok> <urn:p> \"nothing internal here\" .\n"[..],
            RdfFormat::NTriples,
            None,
            TS,
            None,
            None,
        )
        .unwrap();
        let (_dir, request) = internal_request(&clean);

        let mut store = leaky_store();
        let result = import_share(&mut store, &request, TS, None).unwrap();
        assert_eq!(result.share_id, request.manifest.share_id);
    }

    /// CONTROL for the pair above: an UNSTAMPED share of the same leaky store
    /// imports without the flag.
    ///
    /// Without this, both tests above would pass if the gate refused every
    /// import that happened to contain the pattern — which is a different and
    /// much broader rule than the one that was ratified. The gate fires on the
    /// MARKER; the scrub is what it does once it has fired.
    #[test]
    fn an_unstamped_share_is_not_checked_even_when_it_carries_the_pattern() {
        let source = leaky_store();
        let dir = tempfile::tempdir().unwrap();
        let share_dir = dir.path().join("share");
        // Produced outward would be refused, so write the payload by hand with
        // no marker: this is precisely the pre-auw0o7 share, and the shape a
        // laundering attempt would take.
        let payload = crate::share::share_payload(
            &source,
            &crate::share::ShareOptions {
                no_shapes: true,
                destination: crate::share::ShareDestination::Internal,
                ..Default::default()
            },
            crate::share::SHARE_PAYLOAD_MAX_BYTES,
        )
        .unwrap();
        std::fs::create_dir_all(&share_dir).unwrap();
        let mut manifest = payload.manifest.clone();
        manifest.destination = None;
        // Re-derive the id, as a laundering producer would have to: the marker
        // is inside the hash, so it cannot merely be deleted.
        manifest.share_id =
            crate::share::sha256(&crate::share::manifest_bytes(&manifest, false).unwrap());
        std::fs::write(
            share_dir.join("manifest.json"),
            crate::share::manifest_bytes(&manifest, true).unwrap(),
        )
        .unwrap();
        for name in ["export.nt", "shapes.ttl"] {
            std::fs::write(share_dir.join(name), &payload.files[name]).unwrap();
        }

        let request = crate::share_transport::read_local(share_dir.to_str().unwrap()).unwrap();
        assert_eq!(request.manifest.destination, None);
        let mut store = leaky_store();
        import_share(&mut store, &request, TS, None).unwrap();
    }
}
