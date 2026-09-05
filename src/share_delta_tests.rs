use super::*;

fn store(value: &str) -> crate::Store {
    let mut store = crate::Store::open_in_memory().unwrap();
    let rdf = format!("<http://example.test/a> <http://example.test/p> \"{value}\" .");
    crate::rdf::ingest_rdf(
        &mut store,
        rdf.as_bytes(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-03",
        None,
        None,
    )
    .unwrap();
    store
}

/// Shapes-free, matching the existing round-trip test: the fixture store
/// has no shapes registry, and a share that demands one would fail for a
/// reason unrelated to what these tests assert.
fn opts() -> ShareOptions {
    ShareOptions {
        no_shapes: true,
        ..ShareOptions::default()
    }
}

#[test]
fn delta_round_trip_is_parent_bound_and_hash_checked() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    crate::share::share(
        &store("old"),
        parent.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            ..ShareOptions::default()
        },
    )
    .unwrap();
    let delta = temp.path().join("delta");
    let written = write_delta(
        &store("new"),
        parent.to_str().unwrap(),
        delta.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            ..ShareOptions::default()
        },
    )
    .unwrap();
    let materialized = materialize(parent.to_str().unwrap(), delta.to_str().unwrap()).unwrap();
    assert_eq!(materialized.manifest.share_id, written.result.share_id);
    assert!(materialized.export_ntriples.contains("new"));
    assert!(!materialized.export_ntriples.contains("old"));
    let triples = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle)
        .for_reader(std::fs::File::open(delta.join("manifest.ttl")).unwrap())
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(!triples.is_empty());
}

// --- build_delta is the SAME producer write_delta uses (aegis-8fdp8d) ----
//
// The wasm page calls build_delta directly because it has no filesystem.
// These pin that it is genuinely the same artifact: if the two ever diverge,
// the repo would have two producers of one format, which is the duplication
// this extraction exists to prevent.

#[test]
fn build_delta_and_write_delta_produce_the_same_artifact() {
    let parent_store = store("before");
    let root = tempfile::tempdir().unwrap();
    let parent_dir = root.path().join("parent");
    crate::share::share(&parent_store, parent_dir.to_str().unwrap(), &opts()).unwrap();

    let child = store("after");
    let out = root.path().join("delta");
    let written = write_delta(
        &child,
        parent_dir.to_str().unwrap(),
        out.to_str().unwrap(),
        &opts(),
    )
    .unwrap();

    let parent = crate::share_transport::read_reference(parent_dir.to_str().unwrap()).unwrap();
    let built = build_delta(
        &child,
        &parent.manifest.share_id,
        &parent.manifest.graph_hash,
        &parent.export_ntriples,
        &opts(),
    )
    .unwrap();

    assert_eq!(built.manifest.delta_id, written.delta_id);
    assert_eq!(built.manifest.delta_hash, written.delta_hash);
    assert_eq!(built.manifest.parent_share, written.parent_share);
    assert_eq!(
        built.update,
        std::fs::read_to_string(out.join("delta.ru")).unwrap(),
        "the in-memory update must be byte-identical to the written delta.ru"
    );
}

#[test]
fn a_delta_retracts_before_it_asserts() {
    // The order is the contract, not an implementation detail: a delta that
    // removes and re-adds the same triple means different things under the
    // two orders, so whichever the applier happened to do would otherwise
    // become the de facto spec.
    let parent_store = store("before");
    let root = tempfile::tempdir().unwrap();
    let parent_dir = root.path().join("parent");
    crate::share::share(&parent_store, parent_dir.to_str().unwrap(), &opts()).unwrap();
    let parent = crate::share_transport::read_reference(parent_dir.to_str().unwrap()).unwrap();

    let built = build_delta(
        &store("after"),
        &parent.manifest.share_id,
        &parent.manifest.graph_hash,
        &parent.export_ntriples,
        &opts(),
    )
    .unwrap();

    let del = built.update.find("DELETE DATA").expect("a retraction");
    let ins = built.update.find("INSERT DATA").expect("an assertion");
    assert!(
        del < ins,
        "DELETE DATA must precede INSERT DATA:\n{}",
        built.update
    );
}

#[test]
fn an_unchanged_store_yields_an_empty_update_not_a_spurious_delta() {
    // The page branches on this: "propose a PR" with nothing edited must say
    // so rather than open GitHub with a blank file.
    let s = store("same");
    let root = tempfile::tempdir().unwrap();
    let parent_dir = root.path().join("parent");
    crate::share::share(&s, parent_dir.to_str().unwrap(), &opts()).unwrap();
    let parent = crate::share_transport::read_reference(parent_dir.to_str().unwrap()).unwrap();

    let built = build_delta(
        &s,
        &parent.manifest.share_id,
        &parent.manifest.graph_hash,
        &parent.export_ntriples,
        &opts(),
    )
    .unwrap();
    assert!(built.update.is_empty(), "got: {}", built.update);
}

// --- what `delta_hash` actually covers, and why it matters (aegis-8fdp8d)
//
// `delta_hash` is sha256 over the delta.ru BYTES, and `materialize` verifies
// it the same way. That is a different thing from the share's `graph_hash`,
// which is RDFC-1.0 over a canonicalized graph.
//
// The distinction is load-bearing for the PR flow. malcolm's ruling on the
// page design — that a `#` retract section sits OUTSIDE the hash because
// comments are lexical rather than graph content — is correct about a GRAPH
// hash and does not transfer to this one: a byte hash covers every byte in
// the file, comments included. So a provenance header inside delta.ru is
// already inside v1's integrity envelope, provided the producer emits it so
// the manifest's delta_hash is computed over the same bytes.
//
// Pinned because a future change to make `delta_hash` graph-derived would
// silently invalidate that reasoning, and this is the test that would say so.
#[test]
fn delta_hash_is_over_file_bytes_so_a_comment_header_is_inside_the_envelope() {
    let body = "DELETE DATA {\n  <urn:s> <urn:p> \"a\" .\n};\n\
                INSERT DATA {\n  <urn:s> <urn:p> \"b\" .\n};\n";
    let header = "# quipu-delta-provenance/1\n# parent_share: sha256:aaa\n";
    let with_header = format!("{header}{body}");

    // A comment header is valid SPARQL Update — the parser accepts it, so a
    // headered delta still applies.
    spargebra::SparqlParser::new()
        .parse_update(&with_header)
        .expect("a leading comment header must not break SPARQL parsing");

    // And it is inside the envelope: the byte hash changes when it is added,
    // which is exactly what "covered by the hash" means.
    assert_ne!(
        sha256(body.as_bytes()),
        sha256(with_header.as_bytes()),
        "delta_hash must change when a header is added, or the header would \
         sit outside the integrity envelope"
    );
}

#[test]
fn files_matches_what_write_delta_puts_on_disk() {
    // The claim that makes a browser-produced delta the SAME artifact as a
    // CLI-produced one: same names, same bytes. If these drift, a reviewer
    // gets a near-miss that materialize may still accept, which is worse
    // than a mismatch it rejects.
    let root = tempfile::tempdir().unwrap();
    let parent_dir = root.path().join("parent");
    crate::share::share(&store("before"), parent_dir.to_str().unwrap(), &opts()).unwrap();
    let out = root.path().join("delta");
    let child = store("after");
    write_delta(
        &child,
        parent_dir.to_str().unwrap(),
        out.to_str().unwrap(),
        &opts(),
    )
    .unwrap();

    let parent = crate::share_transport::read_reference(parent_dir.to_str().unwrap()).unwrap();
    let built = build_delta(
        &child,
        &parent.manifest.share_id,
        &parent.manifest.graph_hash,
        &parent.export_ntriples,
        &opts(),
    )
    .unwrap();

    let mut names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    let mut got: Vec<String> = built.files().unwrap().into_iter().map(|(n, _)| n).collect();
    got.sort();
    assert_eq!(got, names, "the page must emit exactly the CLI's file set");

    for (name, contents) in built.files().unwrap() {
        assert_eq!(
            contents,
            std::fs::read_to_string(out.join(&name)).unwrap(),
            "{name} differs between build_delta and write_delta"
        );
    }
}

#[test]
fn embedded_delta_budget_does_not_raise_the_default_ceiling() {
    let child = store(&"x".repeat(crate::share::SHARE_PAYLOAD_MAX_BYTES));
    let default = build_delta(&child, "parent", "hash", "", &opts());
    assert!(
        default
            .unwrap_err()
            .to_string()
            .contains("exceeding max_bytes")
    );
    let embedded = build_delta_with_limit(
        &child,
        "parent",
        "hash",
        "",
        &opts(),
        4 * crate::share::SHARE_PAYLOAD_MAX_BYTES,
    )
    .unwrap();
    assert!(embedded.update.len() > crate::share::SHARE_PAYLOAD_MAX_BYTES);
    let bounded = build_delta_with_limit(&child, "parent", "hash", "", &opts(), 100);
    assert!(
        bounded
            .unwrap_err()
            .to_string()
            .contains("exceeding max_bytes")
    );
}
