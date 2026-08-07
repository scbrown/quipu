//! Scale benchmark — the measurements behind `docs/design/wasm-support.md` §5.
//!
//! Ingests N synthetic Gas Town-shaped episodes, reports storage cost per
//! episode, then times three representative reads: a bound-subject point
//! lookup, a type scan, and a 2-hop join.
//!
//! The 2-hop join is the interesting one. `eval_bgp` (`src/sparql/triple.rs`)
//! is a nested-loop join that issues one SQL statement per accumulated row per
//! pattern, so join cost grows quadratically in store size while point lookups
//! stay flat. This example exists so that claim stays checkable rather than
//! becoming folklore in a design doc.
//!
//! ```bash
//! cargo run --release --example scale_bench -- 10000
//! ```
//!
//! The query budget is raised to 10 minutes so the join returns a number
//! instead of the default 30s timeout — above ~2,500 episodes it would
//! otherwise only ever report "timed out".

use std::time::Instant;

use quipu::Store;

/// Nodes and edges per episode; keep in sync with the doc's "20 triples /
/// episode" figure if this shape changes.
fn episode_json(i: usize) -> String {
    format!(
        r#"{{
      "name": "episode-{i}",
      "episode_body": "Agent observed a build failure in service-{i} during deploy window {i}.",
      "source": "gastown",
      "group_id": "rig-{group}",
      "nodes": [
        {{"name": "service-{i}", "type": "Service", "description": "Backend service number {i} in the fleet"}},
        {{"name": "deploy-{i}", "type": "Deployment", "description": "Deployment event {i} to production"}},
        {{"name": "agent-{agent}", "type": "Agent", "description": "The polecat that observed this"}}
      ],
      "edges": [
        {{"source": "deploy-{i}", "target": "service-{i}", "relation": "targets"}},
        {{"source": "agent-{agent}", "target": "deploy-{i}", "relation": "observed"}}
      ]
    }}"#,
        group = i % 8,
        agent = i % 50,
    )
}

const NS: &str = "http://gastown.example/";

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let path = args.next().unwrap_or_else(|| "scale_bench.db".to_string());

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }

    let mut store = Store::open(&path).expect("open store");
    let started = Instant::now();
    let mut triples = 0usize;
    for i in 0..n {
        let episode: quipu::episode::Episode =
            serde_json::from_str(&episode_json(i)).expect("episode parses");
        // Spread timestamps so valid_from is not a single constant.
        let ts = format!(
            "2026-08-07T{:02}:{:02}:{:02}Z",
            i % 24,
            (i / 24) % 60,
            (i / 1440) % 60
        );
        let (_tx, count, _outcome) =
            quipu::episode::ingest_episode_outcome(&mut store, &episode, &ts, NS)
                .expect("ingest succeeds");
        triples += count;
    }
    let ingest = started.elapsed();
    drop(store);

    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    println!("episodes      : {n}");
    println!(
        "triples       : {triples} ({:.1} per episode)",
        triples as f64 / n as f64
    );
    println!(
        "ingest        : {:.2}s ({:.0} episodes/s)",
        ingest.as_secs_f64(),
        n as f64 / ingest.as_secs_f64()
    );
    println!("db size       : {bytes} bytes");
    println!("bytes/episode : {:.1}", bytes as f64 / n as f64);
    println!("bytes/triple  : {:.1}", bytes as f64 / triples as f64);

    let mut store = Store::open(&path).expect("reopen store");
    // Without this the join reports a timeout rather than a duration past
    // ~2,500 episodes, which hides the shape of the curve.
    store.search_config_mut().query_timeout_ms = 600_000;

    let queries = [
        (
            "point lookup",
            format!("SELECT ?p ?o WHERE {{ <{NS}service-1> ?p ?o }}"),
        ),
        (
            "type scan   ",
            format!("SELECT ?s WHERE {{ ?s a <{NS}Service> }} LIMIT 100"),
        ),
        (
            "2-hop join  ",
            format!("SELECT ?d ?s WHERE {{ ?d <{NS}targets> ?s . ?s a <{NS}Service> }} LIMIT 100"),
        ),
    ];

    for (label, sparql) in &queries {
        let t = Instant::now();
        let elapsed = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
        match quipu::sparql::query(&store, sparql) {
            Ok(quipu::sparql::QueryResult::Select { rows, .. }) => {
                println!("{label}  : {:>10.2}ms ({} rows)", elapsed(t), rows.len());
            }
            Ok(_) => println!("{label}  : {:>10.2}ms (non-select)", elapsed(t)),
            Err(e) => println!("{label}  : failed after {:>8.2}ms: {e}", elapsed(t)),
        }
    }
}
