//! Tests for knowledge packs (quipu #81) — one per acceptance criterion.

use super::*;
use crate::lattice::Freshness;
use crate::store::labels::GraphLabel;
use crate::types::{Op, Value};

const TS: &str = "2026-08-06T00:00:00Z";

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("quipu-pack-{name}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("out.qpack.db").to_string_lossy().into_owned()
}

/// A store with one labelled graph holding two triples, one of them an
/// object-position REFERENCE (the `Ref` BLOB that cannot be remapped in SQL).
fn producer(extra_terms: usize) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    // Intern junk first so a second store assigns DIFFERENT ids to the same
    // IRIs — which is exactly what the hash must be insensitive to.
    for i in 0..extra_terms {
        store.intern(&format!("urn:junk:{i}")).unwrap();
    }
    let g = store.overlay_create("urn:g:pack", 0).unwrap();
    let s = store.intern("http://example.org/s").unwrap();
    let p = store.intern("http://example.org/p").unwrap();
    let q = store.intern("http://example.org/q").unwrap();
    let o = store.intern("http://example.org/o").unwrap();
    store
        .overlay_write(g, Op::Assert, s, p, Value::Str("literal".into()), TS)
        .unwrap();
    store
        .overlay_write(g, Op::Assert, s, q, Value::Ref(o), TS)
        .unwrap();
    store
        .set_graph_label(
            "urn:g:pack",
            &GraphLabel {
                freshness: Some(Freshness::Fresh),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
    store
}

// --- Acceptance 2: the hash describes CONTENT, not the producer ---

#[test]
fn the_hash_is_identical_across_different_term_id_assignment() {
    // The acceptance that justifies sorting: `current_facts_in_graph` orders by
    // TERM ID, which differs between these two stores because one interned junk
    // first. Hashing emission order would make the hash a property of the
    // producer.
    let a = producer(0);
    let b = producer(37);

    let ha = content_hash(&canonical_content(&a, "urn:g:pack", &[], &[]).unwrap());
    let hb = content_hash(&canonical_content(&b, "urn:g:pack", &[], &[]).unwrap());
    assert_eq!(ha, hb, "same triples, different ids -> same hash");
    assert!(ha.starts_with("sha256:"));

    // Control: DIFFERENT content must hash differently, or the test above
    // passes because the hash ignores everything.
    let mut c = producer(0);
    let g = c.lookup("urn:g:pack").unwrap().unwrap();
    let s = c.intern("http://example.org/s").unwrap();
    let extra = c.intern("http://example.org/extra").unwrap();
    c.overlay_write(g, Op::Assert, s, extra, Value::Str("more".into()), TS)
        .unwrap();
    let hc = content_hash(&canonical_content(&c, "urn:g:pack", &[], &[]).unwrap());
    assert_ne!(ha, hc, "an added triple MUST change the hash");
}

#[test]
fn the_label_is_part_of_the_content() {
    // A pack whose label changed is different content — a consumer composes it.
    let a = producer(0);
    let ha = content_hash(&canonical_content(&a, "urn:g:pack", &[], &[]).unwrap());

    let mut b = producer(0);
    b.set_graph_label(
        "urn:g:pack",
        &GraphLabel {
            freshness: Some(Freshness::Stale),
            ..Default::default()
        },
        "2026-08-07T00:00:00Z",
        None,
    )
    .unwrap();
    let hb = content_hash(&canonical_content(&b, "urn:g:pack", &[], &[]).unwrap());
    assert_ne!(ha, hb, "the label travels with the content");
}

// --- Acceptance 1, 3, 5 ---

#[test]
fn a_pack_round_trips_and_verifies_and_leaves_no_wal_siblings() {
    let store = producer(0);
    let out = tmp("roundtrip");
    let manifest = pack(&store, "urn:g:pack", &out, &PackOptions::default(), TS).unwrap();

    assert_eq!(manifest.source_graph, "urn:g:pack");
    assert_eq!(
        manifest.term_space, 0,
        "quipu #74 is gated; space 0 for now"
    );
    assert!(manifest.content_hash.starts_with("sha256:"));

    // Acceptance 5: ONE file.
    assert!(std::path::Path::new(&out).exists());
    for suffix in [
        "-wal",
        "-shm",
        ".building",
        ".building-wal",
        ".building-shm",
    ] {
        assert!(
            !std::path::Path::new(&format!("{out}{suffix}")).exists(),
            "a pack must be a single attachable artifact; found {out}{suffix}"
        );
    }

    // Acceptance 1: --verify recomputes and matches.
    let (stored, recomputed, ok) = verify(&out).unwrap();
    assert!(ok, "stored {stored} != recomputed {recomputed}");

    // Acceptance 3: the packed store opens standalone with its content intact.
    let opened = Store::open(&out).unwrap();
    let g = opened.lookup("urn:g:pack").unwrap().expect("graph present");
    assert_eq!(
        opened.current_facts_in_graph(g).unwrap().len(),
        2,
        "both facts travelled"
    );
    assert_eq!(
        opened.label_of("urn:g:pack").unwrap().freshness.value,
        Some(Freshness::Fresh),
        "the label travelled"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn an_object_position_reference_survives_re_interning() {
    // The `Ref` BLOB is the whole reason packs re-intern rather than copy rows:
    // SQL cannot rewrite it. If re-interning were wrong, this would resolve to
    // a different IRI — or to nothing — in the packed store.
    let store = producer(11);
    let out = tmp("refs");
    pack(&store, "urn:g:pack", &out, &PackOptions::default(), TS).unwrap();

    let opened = Store::open(&out).unwrap();
    let g = opened.lookup("urn:g:pack").unwrap().unwrap();
    let refs: Vec<String> = opened
        .current_facts_in_graph(g)
        .unwrap()
        .iter()
        .filter_map(|f| match &f.value {
            Value::Ref(id) => opened.resolve(*id).ok(),
            _ => None,
        })
        .collect();
    assert_eq!(
        refs,
        vec!["http://example.org/o".to_string()],
        "the reference resolves to the SAME IRI in the packed store"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn shapes_and_queries_named_on_the_command_travel_with_the_pack() {
    let store = producer(0);
    store.load_shapes("s1", "# a shape", TS).unwrap();
    store
        .query_load(
            &crate::store::queries::StoredQuery {
                name: "q1".into(),
                description: "d".into(),
                template: "SELECT ?s WHERE { ?s ?p ?o }".into(),
                dataset: None,
                params: vec![],
            },
            TS,
        )
        .unwrap();

    let out = tmp("bundle");
    let opts = PackOptions {
        shapes: vec!["s1".into()],
        queries: vec!["q1".into()],
        ..Default::default()
    };
    pack(&store, "urn:g:pack", &out, &opts, TS).unwrap();

    let opened = Store::open(&out).unwrap();
    assert_eq!(opened.list_shapes().unwrap().len(), 1);
    assert_eq!(opened.query_list().unwrap().len(), 1);
    assert!(
        verify(&out).unwrap().2,
        "hash covers shapes and queries too"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn unpack_materializes_and_versions_registries_beside_existing_entries() {
    let source = producer(0);
    source
        .load_shapes("shared", "# producer shape", TS)
        .unwrap();
    source
        .query_load(
            &crate::store::queries::StoredQuery {
                name: "shared".into(),
                description: "producer".into(),
                template: "SELECT ?s WHERE { ?s ?p ?o }".into(),
                dataset: None,
                params: vec![],
            },
            TS,
        )
        .unwrap();
    let artifact = tmp("unpack-pack");
    pack(
        &source,
        "urn:g:pack",
        &artifact,
        &PackOptions {
            shapes: vec!["shared".into()],
            queries: vec!["shared".into()],
            ..Default::default()
        },
        TS,
    )
    .unwrap();

    let destination = tmp("unpack-dest");
    let consumer = Store::open(&destination).unwrap();
    consumer
        .load_shapes("shared", "# consumer shape", "2025-01-01T00:00:00Z")
        .unwrap();
    consumer
        .query_load(
            &crate::store::queries::StoredQuery {
                name: "shared".into(),
                description: "consumer".into(),
                template: "SELECT ?o WHERE { ?s ?p ?o }".into(),
                dataset: None,
                params: vec![],
            },
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
    drop(consumer);

    let report = unpack(
        &artifact,
        &destination,
        Some("urn:g:materialized"),
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    assert!(report.facts > 0);
    let opened = Store::open(&destination).unwrap();
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(
        &opened,
        "SELECT ?o WHERE { GRAPH <urn:g:materialized> { <http://example.org/s> <http://example.org/p> ?o } }",
    )
    .unwrap() else { panic!("expected SELECT") };
    assert_eq!(
        rows.len(),
        1,
        "materialized pack graph is locally queryable"
    );
    assert_eq!(opened.list_shapes().unwrap()[0].1, "# producer shape");
    assert_eq!(
        opened.query_get("shared").unwrap().unwrap().description,
        "producer"
    );

    let as_of = crate::store::AsOf {
        tx: None,
        valid_at: Some("2025-06-01T00:00:00Z".into()),
    };
    assert_eq!(
        opened.list_shapes_as_of(&as_of).unwrap()[0].1,
        "# consumer shape"
    );
    for path in [artifact, destination] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn a_respaced_pack_attaches_surfaces_its_manifest_and_spans_queries() {
    let mut source = producer(0);
    source.embedding_config_mut().model_path = Some("models/producer.onnx".into());
    source.embedding_config_mut().dimension = 768;
    let pack0 = tmp("attach-pack0");
    pack(&source, "urn:g:pack", &pack0, &PackOptions::default(), TS).unwrap();
    let pack7 = tmp("attach-pack7");
    crate::store::respace::respace_file(
        std::path::Path::new(&pack0),
        std::path::Path::new(&pack7),
        7,
    )
    .unwrap();
    let local = tmp("attach-consumer");
    let opened = Store::open_with_attachments(
        &local,
        &[crate::store::attach::Attachment::read_only("pack", &pack7)],
    )
    .unwrap();
    assert_eq!(opened.pack_manifests().len(), 1);
    assert_eq!(opened.pack_manifests()[0].0, "pack");
    assert_eq!(opened.pack_manifests()[0].1.term_space, 7);
    assert_eq!(opened.pack_embedding_warnings().len(), 1);
    assert!(opened.pack_embedding_warnings()[0].contains("not converted"));
    assert_eq!(
        opened.verify_attached_pack_hashes().unwrap(),
        vec![("pack".into(), true)]
    );
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(
        &opened,
        "SELECT ?o WHERE { GRAPH <urn:g:pack> { <http://example.org/s> <http://example.org/p> ?o } }",
    )
    .unwrap() else { panic!("expected SELECT") };
    assert_eq!(
        rows.len(),
        1,
        "acceptance requires the attached pack to be queryable"
    );
    for path in [pack0, pack7, local] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn pack_to_bytes_agrees_with_pack_and_the_bytes_are_a_pack_file() {
    // quipu-2l5: the browser-side pack is the SAME build as the file-side
    // pack, so its manifest — content hash included — must be identical, and
    // writing the bytes to disk must yield a file `verify` accepts.
    let store = producer(0);
    let out = tmp("bytes-vs-file");
    let file_manifest = pack(&store, "urn:g:pack", &out, &PackOptions::default(), TS).unwrap();
    let (bytes_manifest, bytes) =
        pack_to_bytes(&store, "urn:g:pack", &PackOptions::default(), TS).unwrap();

    assert_eq!(bytes_manifest.content_hash, file_manifest.content_hash);
    assert_eq!(bytes_manifest.source_graph, file_manifest.source_graph);
    assert_eq!(bytes_manifest.counts, file_manifest.counts);

    let from_bytes = tmp("bytes-written");
    std::fs::write(&from_bytes, &bytes).unwrap();
    let (stored, recomputed, ok) = verify(&from_bytes).unwrap();
    assert!(ok, "stored {stored} != recomputed {recomputed}");

    for path in [out, from_bytes] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn a_pack_built_as_bytes_attaches_to_a_native_store() {
    // quipu-2l5 acceptance: "a pack produced in a browser attaches to a
    // native store." This is the native leg of that claim — the wasm leg
    // (wasm/harness roundtrip) produces bytes through the identical
    // `pack_to_bytes` call and ships them out of the tab.
    let store = producer(0);
    let (_, bytes) = pack_to_bytes(&store, "urn:g:pack", &PackOptions::default(), TS).unwrap();

    let pack0 = tmp("bytes-attach-pack0");
    std::fs::write(&pack0, &bytes).unwrap();
    // Same composition rule as any pack file: respace out of the consumer's
    // term space before attaching (multi-db-composition.md).
    let pack9 = tmp("bytes-attach-pack9");
    crate::store::respace::respace_file(
        std::path::Path::new(&pack0),
        std::path::Path::new(&pack9),
        9,
    )
    .unwrap();

    let local = tmp("bytes-attach-consumer");
    let opened = Store::open_with_attachments(
        &local,
        &[crate::store::attach::Attachment::read_only("pack", &pack9)],
    )
    .unwrap();
    assert_eq!(opened.pack_manifests().len(), 1);
    assert_eq!(
        opened.verify_attached_pack_hashes().unwrap(),
        vec![("pack".into(), true)]
    );
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(
        &opened,
        "SELECT ?o WHERE { GRAPH <urn:g:pack> { <http://example.org/s> <http://example.org/p> ?o } }",
    )
    .unwrap() else {
        panic!("expected SELECT")
    };
    assert_eq!(rows.len(), 1, "the byte-built pack must be queryable");

    for path in [pack0, pack9, local] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn naming_a_shape_that_does_not_exist_is_refused() {
    let store = producer(0);
    let out = tmp("badshape");
    let opts = PackOptions {
        shapes: vec!["nope".into()],
        ..Default::default()
    };
    let err = pack(&store, "urn:g:pack", &out, &opts, TS).expect_err("no such shape");
    assert!(err.to_string().contains("no such shape"), "{err}");
}

#[test]
fn packing_an_unknown_graph_is_refused() {
    let store = producer(0);
    let out = tmp("badgraph");
    let err = pack(&store, "urn:g:missing", &out, &PackOptions::default(), TS)
        .expect_err("unknown graph");
    assert!(err.to_string().contains("unknown graph"), "{err}");
}

// --- Acceptance 4 ---

#[test]
fn with_vectors_refuses_on_a_non_sqlite_backend_naming_the_restriction() {
    let mut store = producer(0);
    store.set_vector_search_delegate(std::sync::Arc::new(NoopDelegate));
    assert!(!store.has_sqlite_vector_backend());

    let out = tmp("vectors");
    let opts = PackOptions {
        with_vectors: true,
        ..Default::default()
    };
    let err = pack(&store, "urn:g:pack", &out, &opts, TS).expect_err("delegate cannot enumerate");
    let msg = err.to_string();
    assert!(
        msg.contains("SQLite vector backend"),
        "names the restriction: {msg}"
    );
    assert!(msg.contains("enumerated"), "says WHY: {msg}");
}

/// A delegate that does nothing — enough to make the backend non-SQLite, which
/// is the only property this test needs.
struct NoopDelegate;

impl crate::vector_delegate::VectorSearchDelegate for NoopDelegate {
    fn vector_search(
        &self,
        _query: &[f32],
        _limit: usize,
        _valid_at: Option<&str>,
    ) -> Result<Vec<crate::vector::VectorMatch>> {
        Ok(vec![])
    }

    fn vector_search_filtered(
        &self,
        _query: &[f32],
        _limit: usize,
        _filter: Option<&str>,
        _valid_at: Option<&str>,
    ) -> Result<Vec<crate::vector::VectorMatch>> {
        Ok(vec![])
    }

    fn vector_count(&self) -> Result<usize> {
        Ok(0)
    }
}

#[test]
fn with_vectors_actually_carries_the_embeddings() {
    // REGRESSION. When #81 first landed, `with_vectors` was CHECKED (it refuses
    // a delegated backend) and then never acted on — so asking for vectors on
    // the ordinary SQLite path produced a pack with none, silently. The refusal
    // existed specifically to avoid "silently missing the vectors that were
    // asked for", and the other path did exactly that.
    //
    // A flag that is accepted and inert is the same class of defect as an
    // unwired config key: the only detector is asserting an OBSERVABLE EFFECT,
    // never that the call succeeded.
    let store = producer(0);
    let s_id = store.lookup("http://example.org/s").unwrap().unwrap();
    store
        .conn
        .execute(
            "INSERT INTO vectors (entity_id, text, embedding, valid_from) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![s_id, "some text", vec![0u8, 1, 2, 3], TS],
        )
        .unwrap();

    let out = tmp("withvectors");
    let opts = PackOptions {
        with_vectors: true,
        ..Default::default()
    };
    let m = pack(&store, "urn:g:pack", &out, &opts, TS).unwrap();
    assert!(
        m.counts.contains("\"vectors\":1"),
        "the manifest must report what travelled: {}",
        m.counts
    );

    let opened = Store::open(&out).unwrap();
    let (n, text): (i64, String) = opened
        .conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(text), '') FROM vectors",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(n, 1, "the embedding travelled");
    assert_eq!(text, "some text");

    // RE-KEYED BY IRI, not copied: the packed store's entity_id is its OWN
    // term id for that IRI, which need not equal the producer's.
    let packed_entity: i64 = opened
        .conn
        .query_row("SELECT entity_id FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        opened.resolve(packed_entity).unwrap(),
        "http://example.org/s",
        "the vector points at the right IRI in the packed store"
    );

    let _ = std::fs::remove_file(&out);
}

#[test]
fn without_the_flag_no_vectors_travel() {
    // The control: the test above must pass because the flag WORKS, not because
    // vectors leak into every pack.
    let store = producer(0);
    let s_id = store.lookup("http://example.org/s").unwrap().unwrap();
    store
        .conn
        .execute(
            "INSERT INTO vectors (entity_id, text, embedding, valid_from) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![s_id, "t", vec![0u8], TS],
        )
        .unwrap();

    let out = tmp("novectors");
    pack(&store, "urn:g:pack", &out, &PackOptions::default(), TS).unwrap();
    let opened = Store::open(&out).unwrap();
    let n: i64 = opened
        .conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "no flag, no vectors");
    let _ = std::fs::remove_file(&out);
}

// --- --format turtle: the interop bundle ---

fn tmpdir(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("quipu-ttl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

#[test]
fn the_turtle_bundle_writes_all_four_files() {
    let store = producer(0);
    store.load_shapes("s1", "# a shape", TS).unwrap();
    store
        .query_load(
            &crate::store::queries::StoredQuery {
                name: "q1".into(),
                description: "d".into(),
                template: "SELECT ?s WHERE { ?s ?p ?o }".into(),
                dataset: None,
                params: vec![],
            },
            TS,
        )
        .unwrap();

    let dir = tmpdir("bundle");
    let opts = PackOptions {
        shapes: vec!["s1".into()],
        queries: vec!["q1".into()],
        ..Default::default()
    };
    let m = pack_turtle(&store, "urn:g:pack", &dir, &opts, TS).unwrap();
    assert_eq!(m.pack_format, "1-turtle");

    for f in ["graph.ttl", "shapes.ttl", "queries.json", "manifest.json"] {
        let p = std::path::Path::new(&dir).join(f);
        assert!(p.exists(), "missing {f}");
        assert!(
            std::fs::metadata(&p).unwrap().len() > 0,
            "{f} is empty — a bundle file that exists but says nothing is worse than an absent one"
        );
    }

    // The manifest is real JSON with the hash in it, not a string blob.
    let mf: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::Path::new(&dir).join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(mf["content_hash"], m.content_hash);
    assert_eq!(mf["counts"]["facts"], 2);
    assert!(mf["producer"]["tool"].as_str().unwrap().contains("turtle"));

    // The query bundle carries the TEMPLATE — a catalog entry without it is not
    // executable by the tool receiving the bundle.
    let qs: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::Path::new(&dir).join("queries.json")).unwrap(),
    )
    .unwrap();
    assert!(
        qs["queries"][0]["template"]
            .as_str()
            .is_some_and(|t| t.contains("SELECT"))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn both_formats_agree_on_the_content_hash() {
    // The property that makes the hash an IDENTITY rather than a checksum of
    // bytes: the same graph packs to the same hash whether it is shipped as a
    // store file or as an interop bundle. Hashing emitted bytes would have made
    // the FORMAT part of the identity — two renderings of the same knowledge
    // with different hashes, which is what a content hash exists to rule out.
    let store = producer(0);
    let db = tmp("agree");
    let dir = tmpdir("agree");

    let a = pack(&store, "urn:g:pack", &db, &PackOptions::default(), TS).unwrap();
    let b = pack_turtle(&store, "urn:g:pack", &dir, &PackOptions::default(), TS).unwrap();
    assert_eq!(
        a.content_hash, b.content_hash,
        "one graph, one identity, two renderings"
    );

    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_turtle_bundle_refuses_an_unknown_graph_and_writes_nothing() {
    let store = producer(0);
    let dir = tmpdir("badgraph");
    assert!(pack_turtle(&store, "urn:g:missing", &dir, &PackOptions::default(), TS).is_err());
    assert!(
        !std::path::Path::new(&dir).join("graph.ttl").exists(),
        "a refused export must not leave a partial bundle"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
