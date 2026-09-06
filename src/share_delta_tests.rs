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

/// A store carrying a block-tier rule and one fact that violates it.
fn leaky_store() -> crate::Store {
    let mut store = crate::Store::open_in_memory().unwrap();
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
        "2026-09-06T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store
}

/// A DELTA CAN CARRY OUTWARD WHAT THE STORE NO LONGER HOLDS (aegis-auw0o7).
///
/// The result share is built from the store and scrubs clean — the identifier
/// was retracted. `delta.ru` is not built from the store: its DELETE clause is
/// lifted verbatim from the PARENT's `export.nt`, and the parent here is the
/// internal share that was allowed to carry it. So the one path where an
/// internal share's contents legitimately reappear later is exactly the path
/// the producer-side scrub cannot see, and this is the test that says so.
#[test]
fn an_outward_delta_from_an_internal_parent_is_refused_for_the_update_document() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let mut store = leaky_store();
    crate::share::share(
        &store,
        parent.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            destination: crate::share::ShareDestination::Internal,
            ..ShareOptions::default()
        },
    )
    .unwrap();

    // The identifier leaves the store. Everything the store can still say
    // about itself is now publishable.
    let leak = store.lookup("urn:leak").unwrap().unwrap();
    let (_, retracted) = store
        .retract_entity(leak, None, "2026-09-06T00:00:01Z", None)
        .unwrap();
    assert_eq!(retracted, 1, "fixture must actually remove the fact");

    // CONTROL, and it is the whole point of the test: a full outward share of
    // this store SUCCEEDS. The defect is invisible to that check.
    let clean = temp.path().join("clean");
    crate::share::share(
        &store,
        clean.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            ..ShareOptions::default()
        },
    )
    .unwrap();
    assert!(
        !std::fs::read_to_string(clean.join("export.nt"))
            .unwrap()
            .contains("private.example")
    );

    // The delta over the same state is refused.
    let out = temp.path().join("delta");
    let error = crate::share_delta::write_delta(
        &store,
        parent.to_str().unwrap(),
        out.to_str().unwrap(),
        &opts(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("delta scrub"), "{error}");
    assert!(error.to_string().contains("delta.ru"), "{error}");
    assert!(
        error.to_string().contains("--destination internal"),
        "the refusal must name the flag: {error}"
    );
    assert!(!out.exists(), "a refused delta left a directory behind");
}

/// The paired arm: the same delta, declared internal, is produced.
#[test]
fn the_same_delta_declared_internal_is_written() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let mut store = leaky_store();
    crate::share::share(
        &store,
        parent.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            destination: crate::share::ShareDestination::Internal,
            ..ShareOptions::default()
        },
    )
    .unwrap();
    let leak = store.lookup("urn:leak").unwrap().unwrap();
    store
        .retract_entity(leak, None, "2026-09-06T00:00:01Z", None)
        .unwrap();

    let out = temp.path().join("delta");
    crate::share_delta::write_delta(
        &store,
        parent.to_str().unwrap(),
        out.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            destination: crate::share::ShareDestination::Internal,
            ..ShareOptions::default()
        },
    )
    .unwrap();
    assert!(
        std::fs::read_to_string(out.join("delta.ru"))
            .unwrap()
            .contains("private.example"),
        "the internal delta carries the retraction verbatim"
    );
}

/// THE TRIPWIRE FOR THE NEXT FILE A DELTA LEARNS TO WRITE (aegis-auw0o7).
///
/// `delta.ru` went unscrubbed for the whole life of the delta format, and for a
/// structural reason that will recur: the scrub lives with the SHARE producer,
/// so a file the DELTA producer adds is outside it by default. A behavioural
/// test for `delta.ru` proves that one file is covered and says nothing about
/// the next one.
///
/// So this asserts the SET, then proves each payload file is covered
/// INDIVIDUALLY — poisoning exactly one at a time, against a store whose own
/// graph is clean, and requiring the refusal to NAME that file.
///
/// Both of those disciplines were added because the first version had neither.
/// It poisoned everything at once and asserted only that the build refused;
/// with the `delta.ru` scrub deleted it stayed GREEN, because the poisoned
/// `shapes.ttl` refused first through the share producer's own check, and a
/// poisoned graph would have refused on `export.nt` before either. An audit
/// that passes in both worlds reads like coverage and measures something
/// weaker.
#[test]
fn every_file_a_delta_writes_is_either_a_manifest_or_individually_scrubbed() {
    // A store that knows the RULE and does not violate it, so `export.nt` can
    // never be what refuses and each arm below is isolated to one file.
    let mut clean = crate::Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut clean,
        &br#"@prefix aegis: <http://aegis.gastown.local/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
aegis:private-host-rule a aegis:InternalIdentifierPattern ;
    rdfs:label "private host" ;
    aegis:regex "private[.]example" ;
    aegis:enforcementTier "block" .
"#[..],
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-09-06T00:00:00Z",
        None,
        None,
    )
    .unwrap();

    // A PARENT that carried the identifier — the internal share, in the shape
    // that produced this defect. Retracted from the store, still quoted in the
    // update document the delta builds from it.
    const POISONED_PARENT: &str = "<urn:leak> <urn:p> \"private.example\" .\n";

    let delta = |parent_export: &str, no_shapes: bool, destination| {
        crate::share_delta::build_delta(
            &clean,
            "sha256:0",
            "sha256:0",
            parent_export,
            &ShareOptions {
                no_shapes,
                destination,
                ..ShareOptions::default()
            },
        )
    };
    use crate::share::ShareDestination::{Internal, Outward};

    // ---- the SET ----------------------------------------------------------
    let built = delta(POISONED_PARENT, true, Internal).unwrap();
    let names: Vec<String> = built
        .files()
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        vec!["manifest.json", "manifest.ttl", "delta.ru", "shapes.ttl"],
        "a delta writes a file this audit does not know about. Decide here \
         whether it can carry graph content outward: if it can, it must reach \
         `enforce_destination`; if it cannot, add it to this list and say why."
    );

    // ---- delta.ru, ALONE --------------------------------------------------
    // Shapes off, graph clean: the update document is the only file that can
    // carry the identifier.
    assert!(
        built.update.contains("private.example"),
        "the update must carry the identifier or the arm below is vacuous"
    );
    // CONTROL: with a clean parent this same call succeeds outward, so the
    // refusal is the parent's content and not the fixture.
    delta("", true, Outward).unwrap();
    let error = delta(POISONED_PARENT, true, Outward).unwrap_err();
    assert!(
        error.to_string().contains("refused delta.ru"),
        "delta.ru must be scrubbed on its own account: {error}"
    );

    // ---- shapes.ttl, ALONE ------------------------------------------------
    clean
        .load_shapes(
            "poisoned-shapes",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n# private.example\n",
            "2026-09-06",
        )
        .unwrap();
    // CONTROL: shapes omitted, clean parent — succeeds, so the refusal below
    // is the shape text alone.
    delta("", true, Outward).unwrap();
    let error = delta("", false, Outward).unwrap_err();
    assert!(
        error.to_string().contains("refused shapes.ttl"),
        "shapes.ttl must be scrubbed on its own account: {error}"
    );
}
