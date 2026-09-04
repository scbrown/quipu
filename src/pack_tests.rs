//! Tests for knowledge packs (quipu #81) — one per acceptance criterion.

use super::*;
use crate::lattice::Freshness;
use crate::store::labels::GraphLabel;
use crate::types::{Op, Value};

const TS: &str = "2026-08-06T00:00:00Z";

/// A temp path whose directory is removed when the guard is dropped — which a
/// `remove_file` at the end of a test cannot do, because a panicking test never
/// reaches it. Both helpers below used to hand back a bare `String` and leave
/// the directory behind: 28 per run of this module, which reached 5,442
/// directories / 7.8G on the crew host before anyone looked, since cargo's
/// TMPDIR there is a disk cache nothing sweeps (aegis-t4oyjy).
struct Tmp {
    _dir: tempfile::TempDir,
    path: String,
}

impl std::ops::Deref for Tmp {
    type Target = str;
    fn deref(&self) -> &str {
        &self.path
    }
}

impl std::fmt::Display for Tmp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.path)
    }
}

impl AsRef<std::path::Path> for Tmp {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.path)
    }
}

impl AsRef<std::ffi::OsStr> for Tmp {
    fn as_ref(&self) -> &std::ffi::OsStr {
        std::ffi::OsStr::new(&self.path)
    }
}

/// A pack FILE inside a temp directory that dies with the test.
fn tmp(name: &str) -> Tmp {
    let dir = tempfile::Builder::new()
        .prefix(&format!("quipu-pack-{name}-"))
        .tempdir()
        .unwrap();
    let path = dir
        .path()
        .join("out.qpack.db")
        .to_string_lossy()
        .into_owned();
    Tmp { _dir: dir, path }
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
    assert_eq!(manifest.term_space, 0, "no --space means space 0");
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
}

#[test]
fn repo_pack_manifest_is_complete_and_incremental_load_is_idempotent() {
    let source = producer(0);
    let artifact = tmp("repo-pack");
    let opts = PackOptions {
        repository: Some("scbrown/example".into()),
        repository_sha: Some("base123".into()),
        model_id: Some("model".into()),
        model_version: Some("v1".into()),
        ..Default::default()
    };
    let manifest = pack(&source, "urn:g:pack", &artifact, &opts, TS).unwrap();
    let producer: serde_json::Value = serde_json::from_str(&manifest.producer).unwrap();
    assert_eq!(producer["repository"], "scbrown/example");
    assert_eq!(producer["repository_sha"], "base123");
    assert_eq!(producer["model_id"], "model");
    assert_eq!(producer["model_version"], "v1");
    assert!(producer["version"].is_string());
    assert!(producer["git_sha"].is_string());
    assert_eq!(producer["pack_schema_version"], 1);

    let destination = tmp("repo-pack-dest");
    let load = LoadOptions {
        into: None,
        expect_repository: Some("scbrown/example"),
        head_sha: Some("head456"),
    };
    let first = unpack_verified(&artifact, &destination, &load, TS).unwrap();
    assert_eq!(first.outcome, "loaded");
    assert!(first.facts > 0);
    assert_eq!(first.repository_sha.as_deref(), Some("base123"));
    assert_eq!(first.head_sha.as_deref(), Some("head456"));
    let second = unpack_verified(&artifact, &destination, &load, TS).unwrap();
    assert_eq!(second.outcome, "unchanged");
    assert_eq!(second.facts, 0);
}

#[test]
fn repo_pack_refuses_partial_provenance_and_wrong_repository_before_writing() {
    let source = producer(0);
    let artifact = tmp("partial-repo-pack");
    let err = pack(
        &source,
        "urn:g:pack",
        &artifact,
        &PackOptions {
            repository: Some("scbrown/example".into()),
            ..Default::default()
        },
        TS,
    )
    .unwrap_err();
    assert!(err.to_string().contains("must be supplied together"));

    let complete = PackOptions {
        repository: Some("scbrown/example".into()),
        repository_sha: Some("base123".into()),
        model_id: Some("model".into()),
        model_version: Some("v1".into()),
        ..Default::default()
    };
    pack(&source, "urn:g:pack", &artifact, &complete, TS).unwrap();
    let destination = tmp("wrong-repo-dest");
    let err = unpack_verified(
        &artifact,
        &destination,
        &LoadOptions {
            expect_repository: Some("scbrown/other"),
            ..Default::default()
        },
        TS,
    )
    .unwrap_err();
    assert!(err.to_string().contains("repository mismatch"));
    assert!(!std::path::Path::new(&destination).exists());
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
}

// --- --space: ship the pack in a designated term space (quipu #74) ---

#[test]
fn a_space_pack_owns_the_space_and_still_verifies() {
    let store = producer(5);
    let out = tmp("space7");
    let opts = PackOptions {
        space: Some(7),
        ..Default::default()
    };
    let m = pack(&store, "urn:g:pack", &out, &opts, TS).unwrap();
    assert_eq!(m.term_space, 7, "the manifest records the shipped space");

    // Still ONE file: acceptance 5 holds on the respace path too.
    for suffix in [
        "-wal",
        "-shm",
        ".building",
        ".building-wal",
        ".building-shm",
    ] {
        assert!(
            !std::path::Path::new(&format!("{out}{suffix}")).exists(),
            "a --space pack must still be a single artifact; found {out}{suffix}"
        );
    }

    // The packed store's ids genuinely live in space 7 — the property that
    // makes it attachable to a space-0 consumer without collisions.
    let opened = Store::open(&out).unwrap();
    assert_eq!(opened.local_term_space().unwrap(), 7);
    let lo = 7 * crate::schema::SPACE_SIZE;
    let hi = lo + crate::schema::SPACE_SIZE;
    let strays: i64 = opened
        .conn
        .query_row(
            "SELECT COUNT(*) FROM terms WHERE id <> ?3 AND (id < ?1 OR id >= ?2)",
            rusqlite::params![lo, hi, crate::schema::ROOT_GRAPH],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(strays, 0, "every term id is inside space 7");

    // The content is intact, the object-position Ref included — the BLOB the
    // respace machinery rewrites because SQL cannot.
    let g = opened.lookup("urn:g:pack").unwrap().unwrap();
    let facts = opened.current_facts_in_graph(g).unwrap();
    assert_eq!(facts.len(), 2, "both facts survived the space move");
    let refs: Vec<String> = facts
        .iter()
        .filter_map(|f| match &f.value {
            Value::Ref(id) => opened.resolve(*id).ok(),
            _ => None,
        })
        .collect();
    assert_eq!(
        refs,
        vec!["http://example.org/o".to_string()],
        "the Ref BLOB moved with the space"
    );

    // The hash is content identity, not custody: verify still passes, and a
    // space-0 pack of the same store carries the SAME hash.
    assert!(verify(&out).unwrap().2, "--verify holds after the move");
    let out0 = tmp("space7-control");
    let m0 = pack(&store, "urn:g:pack", &out0, &PackOptions::default(), TS).unwrap();
    assert_eq!(
        m.content_hash, m0.content_hash,
        "a space moves ids, not content"
    );
}

#[test]
fn a_space_pack_attaches_directly_without_a_separate_respace() {
    // The point of --space: the producer ships an artifact the consumer
    // attaches AS-IS, instead of running `quipu db respace` on it first.
    // (Colliding spaces are still refused at attach time —
    // `attach/tests.rs::two_space_zero_databases_are_refused_naming_respace`.)
    let store = producer(0);
    let pack7 = tmp("space-attach");
    let opts = PackOptions {
        space: Some(7),
        ..Default::default()
    };
    pack(&store, "urn:g:pack", &pack7, &opts, TS).unwrap();

    let local = tmp("space-attach-consumer");
    let opened = Store::open_with_attachments(
        &local,
        &[crate::store::attach::Attachment::read_only("pack", &pack7)],
    )
    .unwrap();
    assert_eq!(opened.pack_manifests()[0].1.term_space, 7);
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
    assert_eq!(rows.len(), 1, "the directly-shipped pack is queryable");
}

#[test]
fn an_out_of_range_space_is_refused_and_ships_nothing() {
    let store = producer(0);
    let out = tmp("space-range");
    let opts = PackOptions {
        space: Some(crate::store::respace::MAX_SPACE + 1),
        ..Default::default()
    };
    let err = pack(&store, "urn:g:pack", &out, &opts, TS).expect_err("out of range");
    assert!(err.to_string().contains("out of range"), "{err}");
    assert!(
        !std::path::Path::new(&out).exists(),
        "a refused pack must ship nothing"
    );
    assert!(
        !std::path::Path::new(&format!("{out}.building")).exists(),
        "…and must not leave the build file behind either"
    );
}

#[test]
fn pack_to_bytes_refuses_a_nonzero_space_naming_the_restriction() {
    // Refused, not ignored: quietly shipping space-0 bytes when a space was
    // asked for is the accepted-and-inert-flag defect --with-vectors had.
    let store = producer(0);
    let opts = PackOptions {
        space: Some(7),
        ..Default::default()
    };
    let err = pack_to_bytes(&store, "urn:g:pack", &opts, TS).expect_err("no file to respace");
    let msg = err.to_string();
    assert!(
        msg.contains("file destination"),
        "names the restriction: {msg}"
    );
    assert!(msg.contains("respace"), "says the remedy: {msg}");
}

// --- --format turtle: the interop bundle ---

/// A temp DIRECTORY that dies with the test — the turtle bundle's output dir.
/// `pack_turtle` runs `create_dir_all`, so handing it one that already exists is
/// the same input as the empty path this used to return.
fn tmpdir(name: &str) -> Tmp {
    let dir = tempfile::Builder::new()
        .prefix(&format!("quipu-ttl-{name}-"))
        .tempdir()
        .unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    Tmp { _dir: dir, path }
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
}

#[test]
fn the_turtle_bundle_refuses_a_nonzero_space_naming_the_restriction() {
    // A bundle carries IRIs, not term ids — there is nothing a space could
    // apply to, so the flag is refused rather than accepted-and-inert.
    let store = producer(0);
    let dir = tmpdir("space");
    let opts = PackOptions {
        space: Some(7),
        ..Default::default()
    };
    let err =
        pack_turtle(&store, "urn:g:pack", &dir, &opts, TS).expect_err("no term ids in a bundle");
    assert!(err.to_string().contains("term space"), "{err}");
    assert!(
        !std::path::Path::new(&dir).join("graph.ttl").exists(),
        "a refused export must not leave a partial bundle"
    );
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
}
