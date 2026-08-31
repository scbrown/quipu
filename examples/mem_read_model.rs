//! In-memory read model measurements — `docs/design/in-memory-read-model.md` §4.
//!
//! Builds a [`quipu::store::read_model::ReadModel`] over a store's ROOT graph
//! and answers the same 2-hop join that `examples/scale_bench.rs` puts through
//! `eval_bgp`. The pairing is the point: the SQL nested-loop join is quadratic
//! and times out past ~2,500 episodes, while the hash join here is linear and
//! sub-millisecond.
//!
//! This drives the REAL type as of quipu-d6x — it was a standalone prototype
//! before the model existed. The join below is written by hand because
//! `eval_bgp` does not consult the model yet; routing it there, behind the
//! scope guard, is quipu-syt.
//!
//! Run it against a store built by `scale_bench`:
//!
//! ```bash
//! cargo run --release --example scale_bench -- 10000 /tmp/q.db
//! cargo run --release --example mem_read_model -- /tmp/q.db
//! ```
//!
//! Build timings are page-cache sensitive — run twice and read the second.

use std::collections::HashSet;
use std::time::Instant;

use quipu::Store;
use quipu::sparql::QueryResult;
use quipu::store::read_model::ReadModel;
use quipu::types::Value;

/// Namespace `scale_bench` ingests under.
const NS: &str = "http://gastown.example/";

/// Resident set size in KiB, for the memory-per-fact figure.
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

/// `?d <pred> ?s . ?s a <type_iri>` as a hash join: build a set from the type
/// side, probe it with the edge side. This is the operation `eval_bgp`
/// currently performs as one SQL statement per accumulated row.
fn two_hop(
    store: &Store,
    model: &ReadModel,
    pred: i64,
    rdf_type: i64,
    type_id: i64,
) -> Vec<(i64, i64)> {
    let typed: HashSet<i64> = model
        .by_predicate_object(store, rdf_type, &Value::Ref(type_id))
        .expect("type lookup")
        .iter()
        .copied()
        .collect();
    model
        .by_predicate(store, pred)
        .expect("predicate lookup")
        .iter()
        .filter_map(|(subject, value)| match value {
            Value::Ref(target) if typed.contains(target) => Some((*subject, *target)),
            _ => None,
        })
        .collect()
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scale_bench.db".to_string());

    let baseline_rss = rss_kb();
    let store = Store::open(&path).expect("open store");

    let started = Instant::now();
    let model = store
        .build_read_model(quipu::schema::ROOT_GRAPH)
        .expect("build read model");
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
    let resident = rss_kb().saturating_sub(baseline_rss);

    println!("triples indexed: {}", model.len());
    println!("build          : {build_ms:>9.1}ms");
    println!("resident       : {:>9.1} MB", resident as f64 / 1024.0);
    println!(
        "bytes/triple   : {:>9.0}",
        (resident as f64 * 1024.0) / model.len().max(1) as f64
    );

    let Ok(Some(pred)) = store.lookup(&format!("{NS}targets")) else {
        println!("(store has no {NS}targets — build one with scale_bench first)");
        return;
    };
    let (Ok(Some(rdf_type)), Ok(Some(type_id))) = (
        store.lookup(quipu::namespace::RDF_TYPE),
        store.lookup(&format!("{NS}Service")),
    ) else {
        println!("(store has no {NS}Service type)");
        return;
    };

    // Warm once so the reported figure is the join, not first-touch paging.
    let _ = two_hop(&store, &model, pred, rdf_type, type_id);
    let started = Instant::now();
    let rows = two_hop(&store, &model, pred, rdf_type, type_id);
    println!(
        "2-hop join     : {:>9.3}ms ({} rows, unlimited)",
        started.elapsed().as_secs_f64() * 1000.0,
        rows.len()
    );

    let query =
        format!("SELECT ?d ?s WHERE {{ ?d <{NS}targets> ?s . ?s a <{NS}Service> }} LIMIT 100");
    let mut samples = Vec::with_capacity(100);
    for iteration in 0..101 {
        let started = Instant::now();
        let result = quipu::sparql_query(&store, &query).expect("SPARQL 2-hop query");
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert!(matches!(result, QueryResult::Select { ref rows, .. } if rows.len() == 100));
        if iteration > 0 {
            samples.push(elapsed_ms);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!("/query p99     : {:>9.3}ms (100 warm samples)", samples[98]);

    if let Ok(Some(subject)) = store.lookup(&format!("{NS}service-1")) {
        let started = Instant::now();
        let n = model
            .by_subject(&store, subject)
            .expect("subject lookup")
            .len();
        println!(
            "point lookup   : {:>9.4}ms ({n} facts)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
