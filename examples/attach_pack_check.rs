//! Native consumer check for a browser-produced pack (quipu-2l5).
//!
//! Takes a pack `.db` (typically written from `pack_to_bytes` output shipped
//! out of a browser tab by `wasm/harness/roundtrip.mjs`), respaces it out of
//! the consumer's term space, attaches it to a fresh native store, verifies
//! the manifest hash, and queries across the attachment. Exits non-zero on
//! any failure — the wasm round-trip driver treats this binary's exit code
//! as the acceptance.
//!
//! ```bash
//! cargo run --release --no-default-features --example attach_pack_check -- pack.db urn:g:browser-pack
//! ```

fn main() {
    let mut args = std::env::args().skip(1);
    let pack_path = args
        .next()
        .expect("usage: attach_pack_check <pack.db> <graph-iri>");
    let graph_iri = args
        .next()
        .expect("usage: attach_pack_check <pack.db> <graph-iri>");

    let dir = std::env::temp_dir().join(format!("quipu-attach-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let respaced = dir.join("pack-respaced.db");
    let consumer = dir.join("consumer.db");
    for p in [&respaced, &consumer] {
        let _ = std::fs::remove_file(p);
    }

    // Same composition rule as any pack file: out of the consumer's term
    // space before attaching (docs/design/multi-db-composition.md).
    quipu::store::respace::respace_file(std::path::Path::new(&pack_path), &respaced, 9)
        .expect("respace the pack");

    let store = quipu::Store::open_with_attachments(
        consumer.to_str().unwrap(),
        &[quipu::store::attach::Attachment::read_only(
            "pack",
            respaced.to_str().unwrap(),
        )],
    )
    .expect("attach the pack");

    assert_eq!(store.pack_manifests().len(), 1, "manifest must surface");
    let verified = store
        .verify_attached_pack_hashes()
        .expect("verify pack hashes");
    assert_eq!(
        verified,
        vec![("pack".to_string(), true)],
        "content hash must recompute clean"
    );

    let sparql = format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{graph_iri}> {{ ?s ?p ?o }} }} LIMIT 10");
    let rows = match quipu::sparql::query(&store, &sparql).expect("query the attachment") {
        quipu::sparql::QueryResult::Select { rows, .. } => rows.len(),
        _ => panic!("expected SELECT"),
    };
    assert!(rows > 0, "the attached pack graph must be queryable");

    println!(
        "attach_pack_check: ok ({rows} rows via GRAPH <{graph_iri}>, hash verified, manifest surfaced)"
    );
}
