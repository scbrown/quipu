//! aegis-yavo9c: emit / propose / apply on the merge that cannot auto-merge.

use std::path::{Path, PathBuf};

use oxrdfio::RdfFormat;

use super::*;
use crate::store::Store;

const S: &str = "https://example.org/s";
const STATUS: &str = "https://example.org/status";
const TAG: &str = "https://example.org/tag";
const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
[] sh:path <https://example.org/status> ; sh:maxCount 1 .
"#;

fn put(store: &mut Store, nt: &str, ts: &str) {
    crate::rdf::ingest_rdf(
        store,
        nt.as_bytes(),
        RdfFormat::NTriples,
        None,
        ts,
        None,
        Some("t"),
    )
    .unwrap();
}

fn set_status(store: &mut Store, value: &str, ts: &str) {
    let e = store.lookup(S).unwrap().unwrap();
    let a = store.lookup(STATUS).unwrap().unwrap();
    store.retract_entity(e, Some(a), ts, None).unwrap();
    put(store, &format!("<{S}> <{STATUS}> \"{value}\" .\n"), ts);
}

fn store_with_base() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    crate::share_scrub::seed_test_catalogue(&mut store);
    store.load_shapes("merge", SHAPES, "2026-10-07").unwrap();
    put(
        &mut store,
        &format!("<{S}> <{STATUS}> \"open\" .\n<{S}> <{TAG}> \"a\" .\n"),
        "2026-10-07T00:00:00Z",
    );
    store
}

/// base "open"; theirs "closed" + a new tag "theirs"; ours "blocked" + "ours".
fn scenario(root: &Path) -> (Store, PathBuf) {
    let mut source = store_with_base();
    let base = crate::share::share(
        &source,
        root.join("base").to_str().unwrap(),
        &crate::share::ShareOptions::default(),
    )
    .unwrap();
    set_status(&mut source, "closed", "2026-10-07T00:01:00Z");
    put(
        &mut source,
        &format!("<{S}> <{TAG}> \"theirs\" .\n"),
        "2026-10-07T00:01:00Z",
    );
    let incoming = root.join("incoming");
    crate::share::share(
        &source,
        incoming.to_str().unwrap(),
        &crate::share::ShareOptions {
            parent_share: Some(base.share_id),
            ..Default::default()
        },
    )
    .unwrap();
    let mut local = store_with_base();
    set_status(&mut local, "blocked", "2026-10-07T00:02:00Z");
    put(
        &mut local,
        &format!("<{S}> <{TAG}> \"ours\" .\n"),
        "2026-10-07T00:02:00Z",
    );
    (local, incoming)
}

fn values(store: &Store, predicate: &str) -> Vec<String> {
    let (graph, _) = crate::share_merge::root_graph(store).unwrap();
    let mut out: Vec<String> = graph
        .iter()
        .filter(|t| t.predicate.as_str() == predicate)
        .map(|t| t.object.to_string())
        .collect();
    out.sort();
    out
}

fn decide(file: &mut DecisionFile, decision: &Decision) -> Vec<u8> {
    for row in &mut file.rows {
        row.decision = Some(decision.clone());
    }
    serde_json::to_vec_pretty(file).unwrap()
}

fn fact_count(store: &Store) -> usize {
    crate::share_merge::root_graph(store).unwrap().0.len()
}

#[test]
fn emit_writes_nothing_and_binds_the_conflict_to_its_inputs() {
    let root = tempfile::tempdir().unwrap();
    let (local, incoming) = scenario(root.path());
    let before = fact_count(&local);
    let file = emit(&local, &incoming).unwrap();
    assert_eq!(fact_count(&local), before);
    assert_eq!(file.rows.len(), 1);
    let row = &file.rows[0];
    assert_eq!(row.record.base, ["\"open\""]);
    assert_eq!(row.record.ours, ["\"blocked\""]);
    assert_eq!(row.record.theirs, ["\"closed\""]);
    assert_eq!(row.provenance.ours.len(), 1);
    assert_eq!(row.provenance.ours[0].value, "\"blocked\"");
    assert!(row.decision.is_none() && row.proposal.is_none());
}

#[test]
fn propose_fills_evidence_and_never_decides() {
    let root = tempfile::tempdir().unwrap();
    let (local, incoming) = scenario(root.path());
    let proposed = propose(emit(&local, &incoming).unwrap());
    let row = &proposed.rows[0];
    assert!(row.decision.is_none(), "an agent proposal must not decide");
    let p = row.proposal.as_ref().unwrap();
    assert!(!p.evidence.is_empty());
    assert!(
        p.evidence.iter().any(|e| e.contains("operator decides")),
        "{p:?}"
    );
}

#[test]
fn deciding_theirs_finishes_the_merge_in_one_attributed_transaction() {
    let root = tempfile::tempdir().unwrap();
    let (mut local, incoming) = scenario(root.path());
    let mut file = emit(&local, &incoming).unwrap();
    let bytes = decide(
        &mut file,
        &Decision::Choose {
            choose: Side::Theirs,
        },
    );
    let out = apply(
        &mut local,
        &incoming,
        &file,
        &bytes,
        "stiwi",
        "2026-10-07T01:00:00Z",
        Some("agent"),
    )
    .unwrap();
    assert_eq!(out.merge.outcome, "merged");
    assert_eq!(values(&local, STATUS), ["\"closed\""]);
    // The clean part lands too: both sides' tags, the base tag kept.
    assert_eq!(values(&local, TAG), ["\"a\"", "\"ours\"", "\"theirs\""]);
    let tx = local
        .get_transaction(out.merge.tx_id.unwrap())
        .unwrap()
        .unwrap();
    let source = tx.source.unwrap();
    assert!(source.contains("reviewer=stiwi"), "{source}");
    assert!(
        source.contains(&format!("decisions={}", out.decisions_sha256)),
        "{source}"
    );
    assert_eq!(out.applied.len(), 1);
}

#[test]
fn an_edited_value_is_applied_and_an_invalid_one_refused() {
    let root = tempfile::tempdir().unwrap();
    let (mut local, incoming) = scenario(root.path());
    let mut file = emit(&local, &incoming).unwrap();
    let bad = decide(
        &mut file,
        &Decision::Values {
            values: vec!["not a term".into()],
        },
    );
    let before = values(&local, STATUS);
    assert!(
        apply(
            &mut local,
            &incoming,
            &file,
            &bad,
            "r",
            "2026-10-07T01:00:00Z",
            None
        )
        .is_err()
    );
    assert_eq!(
        values(&local, STATUS),
        before,
        "a refused apply writes nothing"
    );
    let good = decide(
        &mut file,
        &Decision::Values {
            values: vec!["\"resolved\"".into()],
        },
    );
    apply(
        &mut local,
        &incoming,
        &file,
        &good,
        "r",
        "2026-10-07T01:00:00Z",
        None,
    )
    .unwrap();
    assert_eq!(values(&local, STATUS), ["\"resolved\""]);
}

#[test]
fn refusals_write_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut local, incoming) = scenario(root.path());
    let emitted = emit(&local, &incoming).unwrap();
    let before = (values(&local, STATUS), values(&local, TAG));
    let attempt = |local: &mut Store, file: &DecisionFile| {
        let bytes = serde_json::to_vec(file).unwrap();
        apply(
            local,
            &incoming,
            file,
            &bytes,
            "r",
            "2026-10-07T01:00:00Z",
            None,
        )
        .unwrap_err()
        .to_string()
    };

    // Undecided.
    let err = attempt(&mut local, &emitted);
    assert!(err.contains("undecided"), "{err}");

    // Over sh:maxCount.
    let mut over = emitted.clone();
    decide(
        &mut over,
        &Decision::Values {
            values: vec!["\"x\"".into(), "\"y\"".into()],
        },
    );
    let err = attempt(&mut local, &over);
    assert!(err.contains("maxCount"), "{err}");

    // A file for a different conflict set.
    let mut foreign = emitted.clone();
    decide(&mut foreign, &Decision::Choose { choose: Side::Ours });
    foreign.rows[0].record.theirs = vec!["\"other\"".into()];
    let err = attempt(&mut local, &foreign);
    assert!(err.contains("stale"), "{err}");

    assert_eq!((values(&local, STATUS), values(&local, TAG)), before);

    // ROOT moved after emit: stale, even with every row decided.
    let mut decided = emitted.clone();
    decide(
        &mut decided,
        &Decision::Choose {
            choose: Side::Theirs,
        },
    );
    put(
        &mut local,
        &format!("<{S}> <{TAG}> \"later\" .\n"),
        "2026-10-07T00:30:00Z",
    );
    let moved = (values(&local, STATUS), values(&local, TAG));
    let err = attempt(&mut local, &decided);
    assert!(err.contains("stale") && err.contains("ROOT"), "{err}");
    assert_eq!((values(&local, STATUS), values(&local, TAG)), moved);
}

#[test]
fn a_reordered_file_is_not_stale_and_an_empty_reviewer_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (mut local, incoming) = scenario(root.path());
    let mut file = emit(&local, &incoming).unwrap();
    let bytes = decide(&mut file, &Decision::Choose { choose: Side::Ours });
    let err = apply(
        &mut local,
        &incoming,
        &file,
        &bytes,
        " ",
        "2026-10-07T01:00:00Z",
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("reviewer"), "{err}");
    file.rows.reverse();
    apply(
        &mut local,
        &incoming,
        &file,
        &bytes,
        "r",
        "2026-10-07T01:00:00Z",
        None,
    )
    .unwrap();
    assert_eq!(values(&local, STATUS), ["\"blocked\""]);
}
