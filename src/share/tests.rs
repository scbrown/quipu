use super::*;

fn fixture() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut store,
        &b"<urn:z> <urn:p> \"last\" .\n<urn:a> <urn:p> \"first\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-08-29T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
        .load_shapes(
            "fixture-shapes",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
            "2026-08-29",
        )
        .unwrap();
    store
}

#[test]
fn unchanged_state_produces_byte_identical_share_payloads() {
    let store = fixture();
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    let opts = ShareOptions {
        turtle_view: true,
        ..Default::default()
    };
    let ma = share(&store, a.to_str().unwrap(), &opts).unwrap();
    let mb = share(&store, b.to_str().unwrap(), &opts).unwrap();
    assert_eq!(ma, mb);
    for file in ["manifest.json", "export.nt", "shapes.ttl", "export.ttl"] {
        assert_eq!(
            std::fs::read(a.join(file)).unwrap(),
            std::fs::read(b.join(file)).unwrap()
        );
    }
}

#[test]
fn manifest_hashes_match_exact_payload_bytes() {
    let store = fixture();
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("share");
    let manifest = share(&store, out.to_str().unwrap(), &ShareOptions::default()).unwrap();
    assert_eq!(
        manifest.graph_hash,
        sha256(&std::fs::read(out.join("export.nt")).unwrap())
    );
    assert_eq!(
        manifest.shapes_hash,
        sha256(&std::fs::read(out.join("shapes.ttl")).unwrap())
    );
    let stored: ShareManifest =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(
        stored.share_id,
        sha256(&manifest_bytes(&stored, false).unwrap())
    );
    assert_eq!(stored.scope, ShareScope::Root);
}

#[test]
fn payload_reconstructs_directory_byte_for_byte() {
    let store = fixture();
    let opts = ShareOptions {
        turtle_view: true,
        ..Default::default()
    };
    let payload = share_payload(&store, &opts, SHARE_PAYLOAD_MAX_BYTES).unwrap();
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("share");
    let manifest = share(&store, out.to_str().unwrap(), &opts).unwrap();

    assert_eq!(payload.manifest, manifest);
    for (name, contents) in &payload.files {
        assert_eq!(contents.as_bytes(), std::fs::read(out.join(name)).unwrap());
    }
    assert_eq!(payload.files.len(), 5);
    assert_eq!(
        payload.manifest.canonicalization.as_deref(),
        Some("RDFC-1.0")
    );
    let manifest_quads = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle)
        .for_reader(payload.files["manifest.ttl"].as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(!manifest_quads.is_empty());
    assert_eq!(
        payload.manifest.graph_hash,
        sha256(payload.files["export.nt"].as_bytes())
    );
    assert_eq!(
        payload.manifest.shapes_hash,
        sha256(payload.files["shapes.ttl"].as_bytes())
    );
    let returned_manifest: ShareManifest =
        serde_json::from_str(&payload.files["manifest.json"]).unwrap();
    assert_eq!(returned_manifest, payload.manifest);
    assert_eq!(
        returned_manifest.share_id,
        sha256(&manifest_bytes(&returned_manifest, false).unwrap())
    );
}

#[test]
fn payload_limit_refuses_oversized_response() {
    let store = fixture();
    let error = share_payload(&store, &ShareOptions::default(), 1).unwrap_err();
    assert!(error.to_string().contains("exceeding max_bytes 1"));
}

#[test]
fn outward_share_refuses_block_tier_internal_identifier_without_rewriting() {
    let mut store = fixture();
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
        "2026-08-29T00:00:01Z",
        None,
        None,
    )
    .unwrap();

    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("refused");
    let error = share(&store, out.to_str().unwrap(), &ShareOptions::default()).unwrap_err();
    assert!(error.to_string().contains("private host"));
    assert!(!error.to_string().contains("private.example"));
    assert!(!out.exists(), "a refused share left a partial directory");
    assert!(matches!(
        crate::sparql::query(&store, "ASK { <urn:leak> <urn:p> \"private.example\" }").unwrap(),
        crate::sparql::QueryResult::Ask(true)
    ));
}

#[test]
fn outward_share_ignores_warn_tier_and_absent_catalog() {
    let mut store = fixture();
    crate::rdf::ingest_rdf(
        &mut store,
        &br#"@prefix aegis: <http://aegis.gastown.local/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
aegis:warning-only a aegis:InternalIdentifierPattern ;
    rdfs:label "warning only" ;
    aegis:regex "first" ;
    aegis:enforcementTier "warn" .
"#[..],
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-29T00:00:01Z",
        None,
        None,
    )
    .unwrap();
    let payload = share_payload(&store, &ShareOptions::default(), SHARE_PAYLOAD_MAX_BYTES).unwrap();
    assert!(payload.files["export.nt"].contains("first"));
}

#[test]
fn parent_share_changes_envelope_identity_not_graph_identity() {
    let store = fixture();
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    let first = share(&store, a.to_str().unwrap(), &ShareOptions::default()).unwrap();
    let second = share(
        &store,
        b.to_str().unwrap(),
        &ShareOptions {
            parent_share: Some(first.share_id.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(first.graph_hash, second.graph_hash);
    assert_ne!(first.share_id, second.share_id);
    assert_eq!(
        second.parent_share.as_deref(),
        Some(first.share_id.as_str())
    );
}

#[test]
fn rdfc_canonicalization_erases_blank_node_labels() {
    let left = b"_:alice <http://example.test/p> _:bob .\n_:bob <http://example.test/p> \"v\" .\n";
    let right = b"_:x <http://example.test/p> _:y .\n_:y <http://example.test/p> \"v\" .\n";
    let left = super::canonicalize_ntriples(left).unwrap();
    let right = super::canonicalize_ntriples(right).unwrap();
    assert_eq!(left, right);
    assert!(String::from_utf8(left).unwrap().contains("_:c14n"));
}

#[test]
fn shape_selection_is_sorted_and_missing_names_fail_without_output() {
    let store = fixture();
    store
        .load_shapes(
            "z-shape",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .",
            "2026-08-29",
        )
        .unwrap();
    store
        .load_shapes(
            "a-shape",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .",
            "2026-08-29",
        )
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("sorted");
    share(
        &store,
        out.to_str().unwrap(),
        &ShareOptions {
            shapes: vec!["z-shape".into(), "a-shape".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let text = std::fs::read_to_string(out.join("shapes.ttl")).unwrap();
    assert!(text.find("a-shape").unwrap() < text.find("z-shape").unwrap());

    let missing = root.path().join("missing");
    assert!(
        share(
            &store,
            missing.to_str().unwrap(),
            &ShareOptions {
                shapes: vec!["does-not-exist".into()],
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        !missing.exists(),
        "a refused share left a partial directory"
    );
}

#[test]
fn defaults_to_all_loaded_shapes_and_refuses_silent_empty_output() {
    let store = fixture();
    store
        .load_shapes(
            "another-shape",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
            "2026-08-29",
        )
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("all-shapes");
    share(&store, out.to_str().unwrap(), &ShareOptions::default()).unwrap();
    let text = std::fs::read_to_string(out.join("shapes.ttl")).unwrap();
    assert!(text.contains("# --- another-shape ---"));
    assert!(text.contains("# --- fixture-shapes ---"));

    let empty_store = Store::open_in_memory().unwrap();
    let refused = root.path().join("refused");
    let error = share(
        &empty_store,
        refused.to_str().unwrap(),
        &ShareOptions::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("no loaded shape sets"));
    assert!(
        !refused.exists(),
        "a refused share left a partial directory"
    );

    let explicit = root.path().join("explicit-no-shapes");
    share(
        &empty_store,
        explicit.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(explicit.join("shapes.ttl"))
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn store_identity_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("store.db");
    let first = Store::open(db.to_str().unwrap())
        .unwrap()
        .store_id()
        .unwrap();
    let second = Store::open(db.to_str().unwrap())
        .unwrap()
        .store_id()
        .unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("urn:uuid:"));
}

// --- pack_dir: the one setting the page cannot see (aegis-8fdp8d) ------------
//
// Built from a REAL share rather than a hand-written struct literal, so these
// exercise the manifest the producer actually emits — a literal would keep
// passing if the field were dropped from the serialized form.

fn sample_manifest() -> ShareManifest {
    share_payload(&fixture(), &ShareOptions::default(), SHARE_PAYLOAD_MAX_BYTES)
        .expect("share payload")
        .manifest
}

//
// The default lives in exactly one place, and the accessor is the only reader,
// because a caller that unwraps the Option would silently pick its own default
// and the page would then target a directory the producer never named.

#[test]
fn pack_dir_defaults_when_unset() {
    let mut m = sample_manifest();
    m.pack_dir = None;
    assert_eq!(m.pack_dir_or_default(), super::DEFAULT_PACK_DIR);
    assert_eq!(m.pack_dir_or_default(), "qpack");
}

#[test]
fn pack_dir_is_used_when_set() {
    let mut m = sample_manifest();
    m.pack_dir = Some("knowledge".into());
    assert_eq!(m.pack_dir_or_default(), "knowledge");
}

#[test]
fn pack_dir_trims_a_trailing_slash_so_callers_can_always_join_with_one() {
    let mut m = sample_manifest();
    m.pack_dir = Some("qpack/".into());
    assert_eq!(m.pack_dir_or_default(), "qpack");
}

#[test]
fn an_empty_pack_dir_falls_back_rather_than_targeting_the_repository_root() {
    // A share asserting its packs live at "" is malformed. Reading that as the
    // repo root would put a delta at `/deltas/<id>.ru` — silently the wrong
    // place, and the damaging reading of a malformed value.
    for empty in ["", "   ", "/"] {
        let mut m = sample_manifest();
        m.pack_dir = Some(empty.into());
        assert_eq!(
            m.pack_dir_or_default(),
            "qpack",
            "empty pack_dir {empty:?} must fall back"
        );
    }
}

#[test]
fn pack_dir_is_omitted_from_json_when_unset_so_existing_shares_stay_byte_identical() {
    let mut m = sample_manifest();
    m.pack_dir = None;
    let json = serde_json::to_string(&m).expect("serialize");
    assert!(
        !json.contains("pack_dir"),
        "an unset pack_dir must not appear in the manifest, or every existing \
         share's bytes — and therefore its share_id — would change: {json}"
    );
}

#[test]
fn a_manifest_without_pack_dir_still_deserializes() {
    // The compatibility claim, exercised rather than asserted: every share
    // produced before this field existed must still load.
    let mut value = serde_json::to_value(sample_manifest()).expect("to value");
    value.as_object_mut().expect("object").remove("pack_dir");
    let back: super::ShareManifest = serde_json::from_value(value).expect("deserialize");
    assert_eq!(back.pack_dir, None);
    assert_eq!(back.pack_dir_or_default(), "qpack");
}
