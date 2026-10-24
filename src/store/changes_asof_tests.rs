//! As-of share conformance (as-of-share.md §Verification plan; aegis-s24.1).
//!
//! A follower bootstraps from a cut at `T`, then tails `/changes` from `T`.
//! It must equal its source at `T` and again at the tail's end, by the same
//! live-fact digest the feed-replay tests use.

use std::path::Path;

use serde_json::Value as Json;

use super::replay_tests::{apply_tx, datum, live_digest, replay_onto};
use crate::asof_share::{AbortReason, Cut, CutError, Limits, Request, cut, cut_with_control};
use crate::store::Store;
use crate::types::{Op, Value};

const G: &str = "http://example.org/graph/hosts";
const TS1: &str = "2026-04-01T00:00:00Z";
const TS2: &str = "2026-04-02T00:00:00Z";
const TS3: &str = "2026-04-03T00:00:00Z";

fn request(store: &Store, tx: Option<i64>) -> Request {
    Request {
        graphs: vec![G.into()],
        context_graphs: Vec::new(),
        tx,
        expected_store_id: store.store_id().unwrap(),
    }
}

/// Every record the spool holds, in order, after checking each part's
/// running digest and the cut's final one.
fn spooled_records(cut: &Cut) -> Vec<Json> {
    use sha2::{Digest, Sha256};
    let text = std::fs::read_to_string(&cut.spool).unwrap();
    let mut hasher = Sha256::new();
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let part: Json = serde_json::from_str(line).unwrap();
        assert_eq!(part["index"], index, "{line}");
        assert_eq!(
            part["digest_before"],
            hex::encode(hasher.clone().finalize()),
            "part {index} digest_before"
        );
        assert_eq!(part["source_tx"], cut.manifest.watermark_tx);
        hasher.update(line.as_bytes());
        records.extend(part["records"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(hex::encode(hasher.finalize()), cut.parts.digest);
    assert_eq!(text.lines().count(), cut.parts.parts);
    assert_eq!(records.len(), cut.parts.records);
    records
}

/// Load a cut into an empty replica: one transaction per graph.
fn bootstrap(cut: &Cut) -> Store {
    let mut replica = Store::open_in_memory().unwrap();
    let records = spooled_records(cut);
    let mut by_graph: Vec<(String, Vec<&Json>)> = Vec::new();
    for record in &records {
        let graph = record["graph"].as_str().unwrap().to_string();
        match by_graph.last_mut() {
            Some((last, list)) if *last == graph => list.push(record),
            _ => by_graph.push((graph, vec![record])),
        }
    }
    for (_, records) in by_graph {
        apply_tx(&mut replica, &records, TS1);
    }
    replica
}

fn assert_equal(source: &Store, replica: &Store, when: &str) {
    let (want, got) = (live_digest(source), live_digest(replica));
    assert!(!want.is_empty(), "fixture wrote nothing live");
    let missing: Vec<_> = want.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{when}: replica diverges from source\n  missing: {missing:#?}\n  extra: {extra:#?}"
    );
}

/// A graph with bounded and open valid time, a byte value, a retract, and a
/// ref, all in `G`.
fn fixture(store: &mut Store) -> i64 {
    let g = store.graph_create(G).unwrap();
    let host = store.intern("http://example.org/korimako").unwrap();
    let cores = store.intern("http://example.org/cpuCores").unwrap();
    let lease = store.intern("http://example.org/lease").unwrap();
    let blob = store.intern("http://example.org/body").unwrap();
    let runs = store.intern("http://example.org/runsOn").unwrap();
    let node = store.intern("http://example.org/pve1").unwrap();
    let mut bounded = datum(
        host,
        lease,
        Value::Str("globex".into()),
        "2024-01-01T00:00:00Z",
        Op::Assert,
    );
    bounded.valid_to = Some("2024-12-31T00:00:00Z".into());
    store
        .transact_to_graph(
            &[
                datum(host, cores, Value::Int(4), TS1, Op::Assert),
                datum(
                    host,
                    lease,
                    Value::Str("acme".into()),
                    "2025-01-01T00:00:00Z",
                    Op::Assert,
                ),
                bounded,
                datum(
                    host,
                    blob,
                    Value::Bytes(vec![0, 159, 146, 150]),
                    TS1,
                    Op::Assert,
                ),
                datum(host, runs, Value::Ref(node), TS1, Op::Assert),
            ],
            TS1,
            None,
            None,
            g,
        )
        .unwrap();
    store
        .transact_to_graph(
            &[datum(host, cores, Value::Int(4), TS2, Op::Retract)],
            TS2,
            None,
            None,
            g,
        )
        .unwrap();
    g
}

/// Positive control and the core claim: a cut at the head equals the source,
/// and tailing after later writes keeps it equal.
#[test]
fn a_cut_then_the_tail_equals_the_source() {
    let mut source = Store::open_in_memory().unwrap();
    let g = fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let cut = cut(
        &source,
        &request(&source, None),
        spool.path(),
        &Limits::default(),
    )
    .unwrap();
    let replica = bootstrap(&cut);
    assert_equal(&source, &replica, "at T");

    let host = source.intern("http://example.org/korimako").unwrap();
    let lease = source.intern("http://example.org/lease").unwrap();
    let mem = source.intern("http://example.org/memGiB").unwrap();
    source
        .transact_to_graph(
            &[
                datum(host, mem, Value::Int(32), TS3, Op::Assert),
                datum(
                    host,
                    lease,
                    Value::Str("acme".into()),
                    "2025-01-01T00:00:00Z",
                    Op::Retract,
                ),
            ],
            TS3,
            None,
            None,
            g,
        )
        .unwrap();
    let replica = replay_onto(&source, cut.manifest.watermark_tx, replica);
    assert_equal(&source, &replica, "after the tail");
}

/// A cut pinned before a retract carries the fact as live; the tail then
/// delivers the retract once.
#[test]
fn a_cut_pinned_before_a_retract_carries_the_fact_and_the_tail_closes_it() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let before_retract = source.latest_tx_id().unwrap() - 1;
    let spool = tempfile::tempdir().unwrap();
    let cut = cut(
        &source,
        &request(&source, Some(before_retract)),
        spool.path(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(cut.manifest.watermark_tx, before_retract);
    let cores: Vec<_> = spooled_records(&cut)
        .into_iter()
        .filter(|r| r["attribute"] == "http://example.org/cpuCores")
        .collect();
    assert_eq!(cores.len(), 1, "the later-retracted fact is live at T");
    assert!(cores[0]["valid_to"].is_null(), "{cores:?}");
    let replica = replay_onto(&source, cut.manifest.watermark_tx, bootstrap(&cut));
    assert_equal(&source, &replica, "after the tail");
}

fn file_store(dir: &Path) -> (Store, String) {
    let path = dir.join("source.db").to_str().unwrap().to_string();
    (Store::open(&path).unwrap(), path)
}

/// A write committed while the cut's read transaction is open is not in the
/// cut and arrives exactly once in the tail.
#[test]
fn a_write_during_the_cut_arrives_once_in_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let (mut source, path) = file_store(dir.path());
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let mut wrote = None;
    let cut = cut_with_control(
        &source,
        &request(&source, None),
        spool.path(),
        &Limits::default(),
        |seen| {
            if seen == 0 && wrote.is_none() {
                let mut writer = Store::open(&path).unwrap();
                let g = writer.graph_create(G).unwrap();
                let host = writer.intern("http://example.org/korimako").unwrap();
                let disk = writer.intern("http://example.org/diskGiB").unwrap();
                wrote = Some(
                    writer
                        .transact_to_graph(
                            &[datum(host, disk, Value::Int(512), TS3, Op::Assert)],
                            TS3,
                            None,
                            None,
                            g,
                        )
                        .unwrap(),
                );
            }
            Ok(())
        },
    )
    .unwrap();
    let wrote = wrote.expect("the hook wrote during the cut");
    assert!(cut.manifest.watermark_tx < wrote, "{:?}", cut.manifest);
    assert!(
        !spooled_records(&cut)
            .iter()
            .any(|r| r["attribute"] == "http://example.org/diskGiB"),
        "a write after T leaked into the cut"
    );
    let replica = replay_onto(&source, cut.manifest.watermark_tx, bootstrap(&cut));
    assert_equal(&source, &replica, "after the tail");
    let disk = live_digest(&replica)
        .into_iter()
        .filter(|f| f.contains("diskGiB"))
        .count();
    assert_eq!(disk, 1, "the concurrent write arrived exactly once");
}

/// A cut larger than one part arrives as numbered parts whose running
/// digests chain to the final digest, and reassembles to the same replica.
#[test]
fn a_cut_over_the_part_budget_splits_and_reassembles() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        part_max_len: 900,
        ..Limits::default()
    };
    let cut = cut(&source, &request(&source, None), spool.path(), &limits).unwrap();
    assert!(cut.parts.parts > 1, "{:?}", cut.parts);
    assert_equal(&source, &bootstrap(&cut), "reassembled");
}

/// The manifest names the source, scope and per-graph totals, and its hash
/// names the spool.
#[test]
fn the_manifest_binds_source_scope_and_counts() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let cut = cut(
        &source,
        &request(&source, None),
        spool.path(),
        &Limits::default(),
    )
    .unwrap();
    let m = &cut.manifest;
    assert_eq!(m.schema, "quipu-asof-share/v1");
    assert_eq!(m.source.store_id, source.store_id().unwrap());
    assert_eq!(m.source.log_epoch, source.log_epoch().unwrap());
    assert_eq!(m.watermark_tx, source.latest_tx_id().unwrap());
    assert_eq!(m.scope, vec![G.to_string()]);
    assert!(m.completeness.valid_time);
    assert_eq!(m.completeness.bytes, "lossless");
    // Four live facts: cpuCores was retracted.
    assert_eq!(m.completeness.graphs[G].records, 4);
    assert!(
        cut.spool
            .to_str()
            .unwrap()
            .contains(cut.manifest_sha256.trim_start_matches("sha256:"))
    );
    // The same source at the same T cuts identically.
    let again = cut_again(&source, spool.path());
    assert_eq!(again.manifest_sha256, cut.manifest_sha256);
    assert_eq!(again.parts.digest, cut.parts.digest);
}

fn cut_again(source: &Store, spool: &Path) -> Cut {
    cut(source, &request(source, None), spool, &Limits::default()).unwrap()
}

fn aborted(result: Result<Cut, CutError>) -> AbortReason {
    match result {
        Err(CutError::Aborted(reason)) => reason,
        other => panic!("expected an abort, got {other:?}"),
    }
}

fn spool_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

#[test]
fn a_time_ceiling_aborts_the_cut_and_leaves_no_spool() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        read_timeout: std::time::Duration::ZERO,
        ..Limits::default()
    };
    let reason = aborted(cut(&source, &request(&source, None), spool.path(), &limits));
    assert_eq!(reason, AbortReason::TimeCeiling);
    assert!(spool_is_empty(spool.path()));
}

/// WAL growth past the ceiling aborts the cut; afterwards the read
/// transaction is closed, so a checkpoint can complete.
#[test]
fn a_wal_ceiling_aborts_the_cut_and_releases_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut source, path) = file_store(dir.path());
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        wal_growth_max_bytes: 0,
        ..Limits::default()
    };
    let reason = aborted(cut_with_control(
        &source,
        &request(&source, None),
        spool.path(),
        &limits,
        |seen| {
            if seen == 0 {
                let mut writer = Store::open(&path).unwrap();
                let host = writer.intern("http://example.org/korimako").unwrap();
                let disk = writer.intern("http://example.org/diskGiB").unwrap();
                writer
                    .transact(
                        &[datum(host, disk, Value::Int(512), TS3, Op::Assert)],
                        TS3,
                        None,
                        None,
                    )
                    .unwrap();
            }
            Ok(())
        },
    ));
    assert_eq!(reason, AbortReason::WalCeiling);
    assert!(spool_is_empty(spool.path()));
    let (busy, _, _): (i64, i64, i64) = source
        .conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(busy, 0, "a checkpoint is blocked after the abort");
}

#[test]
fn a_full_spool_quota_refuses_the_cut() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        spool_quota_bytes: 1,
        ..Limits::default()
    };
    let reason = aborted(cut(&source, &request(&source, None), spool.path(), &limits));
    assert_eq!(reason, AbortReason::SpoolQuota);
    assert!(spool_is_empty(spool.path()));
}

#[test]
fn invalid_selections_are_refused() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let refuse = |request: Request| {
        assert!(
            matches!(
                cut(&source, &request, spool.path(), &Limits::default()),
                Err(CutError::Refused(_))
            ),
            "{request:?}"
        );
    };
    let mut r = request(&source, None);
    r.graphs = Vec::new();
    refuse(r);
    let mut r = request(&source, None);
    r.graphs = vec![G.into(), G.into()];
    refuse(r);
    let mut r = request(&source, None);
    r.graphs = vec!["http://example.org/graph/unregistered".into()];
    refuse(r);
    let mut r = request(&source, None);
    r.context_graphs = vec![G.into()];
    refuse(r);
    let mut r = request(&source, None);
    r.expected_store_id = "urn:uuid:not-this-store".into();
    refuse(r);
    assert!(spool_is_empty(spool.path()));
}

/// The scan is one sequential pass over facts in rowid order: no index
/// lookups per row and no temporary B-tree, however large the graphs are.
#[test]
fn the_cut_scan_is_one_sequential_pass() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let mut stmt = source
        .conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT g, e, a, v, tx, valid_from, \
                    CASE WHEN retracted_tx IS NULL THEN valid_to END \
             FROM facts NOT INDEXED \
             WHERE g IN (1, 2) AND op = 1 AND tx <= ?1 \
               AND (retracted_tx IS NULL OR retracted_tx > ?1) \
             ORDER BY rowid",
        )
        .unwrap();
    let plan: Vec<String> = stmt
        .query_map([1], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(plan, vec!["SCAN facts".to_string()], "{plan:?}");
}

/// A cut pinned at an earlier `T` leaves out an assert committed after `T`;
/// the tail delivers it once.
#[test]
fn a_pinned_cut_leaves_out_later_asserts_for_the_tail() {
    let mut source = Store::open_in_memory().unwrap();
    let g = fixture(&mut source);
    let pinned = source.latest_tx_id().unwrap();
    let host = source.intern("http://example.org/korimako").unwrap();
    let mem = source.intern("http://example.org/memGiB").unwrap();
    source
        .transact_to_graph(
            &[datum(host, mem, Value::Int(32), TS3, Op::Assert)],
            TS3,
            None,
            None,
            g,
        )
        .unwrap();
    let spool = tempfile::tempdir().unwrap();
    let cut = cut(
        &source,
        &request(&source, Some(pinned)),
        spool.path(),
        &Limits::default(),
    )
    .unwrap();
    assert!(
        !spooled_records(&cut)
            .iter()
            .any(|r| r["attribute"] == "http://example.org/memGiB"),
        "an assert after T is in the cut"
    );
    let replica = replay_onto(&source, cut.manifest.watermark_tx, bootstrap(&cut));
    assert_equal(&source, &replica, "after the tail");
}

/// The applier's digest, verbatim from quipu #803
/// (`applier::share_graph_digest`). It is the oracle here until that PR
/// lands; then this test calls the applier's function directly.
fn applier_share_graph_digest(records: &[&Json]) -> String {
    use sha2::{Digest, Sha256};
    fn canonical(v: &Json) -> String {
        match v {
            Json::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let body: Vec<String> = keys
                    .into_iter()
                    .map(|k| format!("{}:{}", Json::String(k.clone()), canonical(&map[k])))
                    .collect();
                format!("{{{}}}", body.join(","))
            }
            Json::Array(items) => {
                let body: Vec<String> = items.iter().map(canonical).collect();
                format!("[{}]", body.join(","))
            }
            other => other.to_string(),
        }
    }
    let mut lines: Vec<String> = records.iter().map(|r| canonical(r)).collect();
    lines.sort();
    hex::encode(Sha256::digest(lines.join("\n").as_bytes()))
}

/// The producer's per-graph digest equals the applier's, both when it sorts
/// in memory and when it spills sorted runs to disk and merges them.
#[test]
fn the_graph_digest_is_the_appliers_with_and_without_spilled_runs() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    for run_bytes in [usize::MAX, 64, 700] {
        let spool = tempfile::tempdir().unwrap();
        let limits = Limits {
            digest_run_bytes: run_bytes,
            ..Limits::default()
        };
        let cut = cut(&source, &request(&source, None), spool.path(), &limits).unwrap();
        let records = spooled_records(&cut);
        let refs: Vec<&Json> = records.iter().collect();
        assert_eq!(
            cut.manifest.completeness.graphs[G].digest,
            format!("sha256:{}", applier_share_graph_digest(&refs)),
            "run_bytes {run_bytes}"
        );
        // The run directory is gone; only the spool remains.
        let left: Vec<_> = std::fs::read_dir(spool.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }
}

/// Records carry the /changes transaction fields the applier reads.
#[test]
fn records_carry_their_transaction_time_actor_and_source() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let cut = cut(
        &source,
        &request(&source, None),
        spool.path(),
        &Limits::default(),
    )
    .unwrap();
    for record in spooled_records(&cut) {
        assert_eq!(record["timestamp"], TS1, "{record}");
        assert!(record.get("actor").is_some() && record.get("source").is_some());
    }
}

/// ROOT is cut only when named, and then equals the source with the tail.
#[test]
fn root_is_cut_when_named_and_tails_to_equality() {
    let mut source = Store::open_in_memory().unwrap();
    let e = source.intern("http://example.org/korimako").unwrap();
    let a = source.intern("http://example.org/name").unwrap();
    source
        .transact(
            &[datum(e, a, Value::Str("korimako".into()), TS1, Op::Assert)],
            TS1,
            None,
            None,
        )
        .unwrap();
    let spool = tempfile::tempdir().unwrap();
    let mut r = request(&source, None);
    r.graphs = vec!["ROOT".into()];
    let cut = cut(&source, &r, spool.path(), &Limits::default()).unwrap();
    assert_eq!(cut.manifest.completeness.graphs["ROOT"].records, 1);
    let records = spooled_records(&cut);
    assert_eq!(records[0]["graph"], "ROOT");
    let b = source.intern("http://example.org/cpuCores").unwrap();
    source
        .transact(
            &[datum(e, b, Value::Int(8), TS2, Op::Assert)],
            TS2,
            None,
            None,
        )
        .unwrap();
    let replica = replay_onto(&source, cut.manifest.watermark_tx, bootstrap(&cut));
    assert_equal(&source, &replica, "ROOT after the tail");
}

/// One slow SQLite step cannot outrun the time ceiling: the progress handler
/// interrupts it mid-statement, without waiting for a 10,000-record check.
#[test]
fn the_time_ceiling_interrupts_a_single_slow_step() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    let source = Store::open_in_memory().unwrap();
    let slow = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000000) \
                SELECT COUNT(*) FROM n";
    let run = |timeout: std::time::Duration| {
        let flag = Arc::new(AtomicU8::new(0));
        let limits = Limits {
            read_timeout: timeout,
            ..Limits::default()
        };
        let guard = crate::asof_share::ReadPhaseGuard::install(
            &source,
            std::time::Instant::now(),
            &limits,
            None,
            0,
            Arc::clone(&flag),
        )
        .unwrap();
        let started = std::time::Instant::now();
        let result: Result<i64, _> = source.conn.query_row(slow, [], |r| r.get(0));
        drop(guard);
        (
            result.is_ok(),
            flag.load(Ordering::Acquire),
            started.elapsed(),
        )
    };
    let (ok, flag, took) = run(std::time::Duration::ZERO);
    assert!(!ok, "the step ran to completion");
    assert_eq!(flag, 1, "interrupted by the time ceiling");
    assert!(took < std::time::Duration::from_secs(1), "{took:?}");
    // Control: removed on drop, so the same step now completes.
    let done: i64 = source.conn.query_row(slow, [], |r| r.get(0)).unwrap();
    assert_eq!(done, 5_000_000);
}

/// grant's counterexample: moving an IRI between scope and context must
/// change the scope digest; reordering a list must not.
#[test]
fn the_scope_digest_binds_each_list_separately() {
    use crate::asof_share::scope_digest;
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
    assert_ne!(
        scope_digest(&s(&["urn:a"]), &s(&["urn:b", "urn:c"])),
        scope_digest(&s(&["urn:a", "urn:b"]), &s(&["urn:c"])),
        "boundary shift collided"
    );
    assert_ne!(
        scope_digest(&s(&["urn:a"]), &s(&[])),
        scope_digest(&s(&[]), &s(&["urn:a"])),
        "a list swap collided"
    );
    assert_eq!(
        scope_digest(&s(&["urn:b", "urn:a"]), &s(&["urn:d", "urn:c"])),
        scope_digest(&s(&["urn:a", "urn:b"]), &s(&["urn:c", "urn:d"])),
        "order changed the digest"
    );
}

/// A cut with no records at all still meets the final ceiling check before
/// any manifest exists.
#[test]
fn an_empty_cut_past_its_time_ceiling_is_aborted_without_a_spool() {
    let mut source = Store::open_in_memory().unwrap();
    source.graph_create(G).unwrap();
    let e = source.intern("http://example.org/x").unwrap();
    let a = source.intern("http://example.org/y").unwrap();
    source
        .transact(
            &[datum(e, a, Value::Int(1), TS1, Op::Assert)],
            TS1,
            None,
            None,
        )
        .unwrap();
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        read_timeout: std::time::Duration::ZERO,
        ..Limits::default()
    };
    let reason = aborted(cut(&source, &request(&source, None), spool.path(), &limits));
    assert_eq!(reason, AbortReason::TimeCeiling);
    assert!(spool_is_empty(spool.path()));
}

/// With no allowance (the disk floor above any free space) the cut is
/// refused before its read transaction opens.
#[test]
fn no_disk_allowance_refuses_before_reading() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        disk_floor_bytes: u64::MAX,
        ..Limits::default()
    };
    let mut reads = 0;
    let reason = aborted(cut_with_control(
        &source,
        &request(&source, None),
        spool.path(),
        &limits,
        |_| {
            reads += 1;
            Ok(())
        },
    ));
    assert_eq!(reason, AbortReason::SpoolQuota);
    assert_eq!(reads, 0, "the read phase started");
    assert!(spool_is_empty(spool.path()));
}

/// Bytes anywhere under the spool directory (a nested run directory, say)
/// count against the quota.
#[test]
fn nested_spool_bytes_count_against_the_quota() {
    let mut source = Store::open_in_memory().unwrap();
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    std::fs::create_dir(spool.path().join("digest-runs-old")).unwrap();
    std::fs::write(spool.path().join("digest-runs-old/run-0"), vec![b'x'; 4096]).unwrap();
    let limits = Limits {
        spool_quota_bytes: 4096,
        ..Limits::default()
    };
    let reason = aborted(cut(&source, &request(&source, None), spool.path(), &limits));
    assert_eq!(reason, AbortReason::SpoolQuota);
}

/// Each record carries its own transaction's time, across transactions with
/// different times, so the per-transaction metadata cache cannot go stale.
#[test]
fn each_record_carries_its_own_transactions_time() {
    let mut source = Store::open_in_memory().unwrap();
    let g = source.graph_create(G).unwrap();
    let e = source.intern("http://example.org/korimako").unwrap();
    let a = source.intern("http://example.org/cpuCores").unwrap();
    let b = source.intern("http://example.org/memGiB").unwrap();
    source
        .transact_to_graph(
            &[datum(e, a, Value::Int(4), TS1, Op::Assert)],
            TS1,
            None,
            None,
            g,
        )
        .unwrap();
    source
        .transact_to_graph(
            &[datum(e, b, Value::Int(32), TS2, Op::Assert)],
            TS2,
            None,
            None,
            g,
        )
        .unwrap();
    let spool = tempfile::tempdir().unwrap();
    let cut = cut(
        &source,
        &request(&source, None),
        spool.path(),
        &Limits::default(),
    )
    .unwrap();
    for record in spooled_records(&cut) {
        let want = if record["attribute"] == "http://example.org/cpuCores" {
            TS1
        } else {
            TS2
        };
        assert_eq!(record["timestamp"], want, "{record}");
    }
}

/// The record struct serializes to exactly the canonical line a `Value`
/// with sorted keys gives, including null fields and nested values.
#[test]
fn an_assert_record_serializes_as_the_canonical_line() {
    use crate::asof_share::AssertRecord;
    let value = serde_json::json!({"bytes_len": 3, "base64": "AJ/u"});
    for (actor, valid_to) in [
        (None, None),
        (
            Some("malcolm".to_string()),
            Some("2026-12-31T00:00:00Z".to_string()),
        ),
    ] {
        let record = AssertRecord {
            actor: &actor,
            attribute: "http://example.org/body",
            entity: "http://example.org/\"quoted\" é",
            graph: "ROOT",
            op: "assert",
            source: &None,
            timestamp: &Some(TS1.to_string()),
            tx: 42,
            valid_from: TS1,
            valid_to: &valid_to,
            value: &value,
        };
        let want = serde_json::json!({
            "op": "assert",
            "tx": 42,
            "timestamp": TS1,
            "actor": actor,
            "source": null,
            "graph": "ROOT",
            "entity": "http://example.org/\"quoted\" é",
            "attribute": "http://example.org/body",
            "value": value,
            "valid_from": TS1,
            "valid_to": valid_to,
        })
        .to_string();
        assert_eq!(serde_json::to_string(&record).unwrap(), want);
    }
}

/// grant's R2: merging digest runs and writing the last part happen after
/// the read transaction closes, so they never pin the WAL. At that point a
/// TRUNCATE checkpoint from another connection completes.
#[test]
fn digest_merging_runs_after_the_read_transaction_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut source, path) = file_store(dir.path());
    fixture(&mut source);
    let spool = tempfile::tempdir().unwrap();
    let limits = Limits {
        digest_run_bytes: 64,
        ..Limits::default()
    };
    let mut checked = None;
    let mut inside = None;
    cut_with_control(
        &source,
        &request(&source, None),
        spool.path(),
        &limits,
        |seen| {
            let other = rusqlite::Connection::open(&path).unwrap();
            // Write so the WAL has frames a checkpoint must move past the reader.
            other
                .execute_batch("CREATE TABLE IF NOT EXISTS t(x); INSERT INTO t VALUES (1);")
                .unwrap();
            let busy: i64 = other
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
                .unwrap();
            if seen == crate::asof_share::AFTER_READ {
                checked = Some(busy);
            } else if inside.is_none() {
                inside = Some(busy);
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        inside,
        Some(1),
        "control: while reading, the snapshot blocks it"
    );
    assert_eq!(
        checked,
        Some(0),
        "after the read closed, nothing pins the WAL"
    );
}

/// A frozen graph's rows live in its pack, not in main, and it is not one of
/// the attachment-contributed graphs. Until frozen packs can be read
/// (s24.1.1) the cut must refuse it, never cut it as an empty graph.
#[test]
fn a_frozen_graph_is_refused_not_cut_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (mut source, _path) = file_store(dir.path());
    fixture(&mut source);
    source
        .freeze_graph(G, dir.path().to_str().unwrap(), TS3, None)
        .unwrap();
    let spool = tempfile::tempdir().unwrap();
    let result = cut(
        &source,
        &request(&source, None),
        spool.path(),
        &Limits::default(),
    );
    assert!(
        matches!(&result, Err(CutError::Refused(e)) if e.to_string().contains("frozen")),
        "{result:?}"
    );
    assert!(spool_is_empty(spool.path()));
    // The same after reopening, when the pack is mounted from the registry.
    drop(source);
    let reopened = Store::open(&_path).unwrap();
    let result = cut(
        &reopened,
        &request(&reopened, None),
        spool.path(),
        &Limits::default(),
    );
    assert!(matches!(result, Err(CutError::Refused(_))), "{result:?}");
}
