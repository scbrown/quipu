//! Release-mode timing harness for `/update` (aegis-jm1lcl).
//!
//! Ignored by default: it seeds stores of up to a million facts. Run with
//!
//! ```text
//! cargo test --release --features shacl,onnx,server --bin quipu-server \
//!     update_bench -- --ignored --nocapture
//! ```
//!
//! `QUIPU_UPDATE_BENCH_SIZES` (comma-separated fact counts, default
//! `10000,100000,1000000`), `QUIPU_UPDATE_BENCH_RUNS` (default 5) and
//! `QUIPU_UPDATE_BENCH_DIR` (a directory for file-backed stores; in-memory when
//! unset) tune it. The timed section is the whole `apply_update` call, which
//! takes the writer lock on entry and holds it until it returns, so the figure
//! is the lock-held time.

use std::sync::Arc;
use std::time::Instant;

use quipu::store::Datum;
use quipu::{Op, Store, Value};

use super::super::SharedStore;

const PREDICATES: usize = 50;
const NAMED_GRAPHS: usize = 3;

pub(super) fn claimed_by() -> &'static str {
    "http://example.org/p/claimedBy"
}

/// Seed `n` facts over `PREDICATES` predicates, the default graph and
/// `NAMED_GRAPHS` named graphs, mixing IRIs, strings and integers, plus a
/// `claimedBy` claim on one entity in a hundred (the CAS target is `e/1`).
pub(super) fn seed(store: &mut Store, n: usize) {
    let entities = (n / 10).max(10);
    let preds: Vec<i64> = (0..PREDICATES)
        .map(|p| store.intern(&format!("http://example.org/p/{p}")).unwrap())
        .collect();
    let graphs: Vec<i64> = std::iter::once(0)
        .chain((0..NAMED_GRAPHS).map(|g| {
            store
                .graph_create(&format!("http://example.org/g/{g}"))
                .unwrap()
        }))
        .collect();
    let claimed = store.intern(claimed_by()).unwrap();
    let ts = "2026-01-01T00:00:00Z";
    let datum = |entity, attribute, value| Datum {
        entity,
        attribute,
        value,
        valid_from: ts.into(),
        valid_to: None,
        op: Op::Assert,
    };
    let mut batch: Vec<Vec<Datum>> = vec![Vec::new(); graphs.len()];
    let mut pending = 0;
    let mut entity_ids = std::collections::HashMap::new();
    for i in 0..n {
        let e = i % entities;
        let entity = *entity_ids
            .entry(e)
            .or_insert_with(|| store.intern(&format!("http://example.org/e/{e}")).unwrap());
        let value = match i % 3 {
            0 => {
                let target = (i * 31) % entities;
                Value::Ref(*entity_ids.entry(target).or_insert_with(|| {
                    store
                        .intern(&format!("http://example.org/e/{target}"))
                        .unwrap()
                }))
            }
            1 => Value::Str(format!("value {i}")),
            _ => Value::Int(i as i64),
        };
        batch[i % graphs.len()].push(datum(entity, preds[(i / entities) % PREDICATES], value));
        if e % 100 == 1 && i < entities {
            let agent = if e == 1 {
                "agentA".into()
            } else {
                format!("agent{e}")
            };
            batch[0].push(datum(entity, claimed, Value::Str(agent)));
        }
        pending += 1;
        if pending == 50_000 || i + 1 == n {
            let batches: Vec<(i64, Vec<Datum>)> = graphs
                .iter()
                .copied()
                .zip(batch.iter_mut().map(std::mem::take))
                .collect();
            store
                .transact_graph_batches(&batches, ts, Some("bench"), Some("bench"))
                .unwrap();
            pending = 0;
        }
    }
}

fn cas(from: &str, to: &str) -> String {
    format!(
        "BASE <http://localhost/update>\nPREFIX p: <http://example.org/p/>\n\
         DELETE {{ <http://example.org/e/1> p:claimedBy ?o }} \
         INSERT {{ <http://example.org/e/1> p:claimedBy \"{to}\" }} \
         WHERE {{ <http://example.org/e/1> p:claimedBy ?o . FILTER(?o = \"{from}\") }}"
    )
}

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

#[test]
#[ignore = "release-mode timing harness; seeds up to 10^6 facts"]
fn update_bench() {
    let sizes: Vec<usize> = std::env::var("QUIPU_UPDATE_BENCH_SIZES")
        .unwrap_or_else(|_| "10000,100000,1000000".into())
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    let runs: usize = std::env::var("QUIPU_UPDATE_BENCH_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let dir = std::env::var("QUIPU_UPDATE_BENCH_DIR").ok();
    println!("size,runs,median_ms,min_ms,max_ms,seed_s,peak_rss_mib");
    for n in sizes {
        let mut store = match &dir {
            Some(d) => {
                std::fs::create_dir_all(d).unwrap();
                let path = format!("{d}/update-bench-{n}.db");
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{path}{suffix}"));
                }
                Store::open(&path).unwrap()
            }
            None => Store::open_in_memory().unwrap(),
        };
        let seeded = Instant::now();
        seed(&mut store, n);
        let seed_s = seeded.elapsed().as_secs_f64();
        let shared: SharedStore = Arc::new(super::super::StoreHandle::writer_only(store));
        let mut times = Vec::new();
        for run in 0..runs {
            let (from, to) = if run % 2 == 0 {
                ("agentA", "agentB")
            } else {
                ("agentB", "agentA")
            };
            let started = Instant::now();
            super::apply_update(&shared, &cas(from, to)).unwrap();
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            let store = shared.lock();
            let e1 = store.lookup("http://example.org/e/1").unwrap().unwrap();
            let attr = store.lookup(claimed_by()).unwrap().unwrap();
            let now: Vec<_> = store
                .current_facts_for_attributes_and_entities_in_graphs(&[attr], &[e1], &[0])
                .unwrap();
            assert_eq!(now.len(), 1, "CAS must leave exactly one claim");
            assert_eq!(
                now[0].value,
                Value::Str(to.into()),
                "CAS must have swapped the claim"
            );
        }
        let mut sorted = times.clone();
        sorted.sort_by(f64::total_cmp);
        println!(
            "{n},{runs},{:.2},{:.2},{:.2},{seed_s:.1},{}",
            sorted[sorted.len() / 2],
            sorted[0],
            sorted[sorted.len() - 1],
            peak_rss_kib() / 1024
        );
    }
}
