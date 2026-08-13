//! Native half of the wasm-vs-native ratio (quipu-ajz, wasm-support.md §5.5).
//!
//! MUST stay methodology-identical to the wasm half
//! (`wasm/harness/src/lib.rs::scenario_bench`): ingest `n` `scale_bench`-shaped
//! episodes; reopen; optionally time the read-model build; run each of the
//! three representative queries once cold, then warm iterations until 300ms
//! cumulative or 30 iters. Emits the same JSON so the two halves diff cleanly.
//!
//! ```bash
//! cargo run --release --no-default-features --example wasm_native_baseline -- 1000
//! QUIPU_READ_MODEL=1 cargo run --release --no-default-features --example wasm_native_baseline -- 1000
//! ```

use std::time::Instant;

/// Same synthetic Gas Town-shaped episode as `examples/scale_bench.rs` and
/// the wasm harness.
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
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let path = args
        .next()
        .unwrap_or_else(|| "wasm_native_baseline.db".to_string());
    let read_model = std::env::var("QUIPU_READ_MODEL").is_ok();

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }

    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;

    let t0 = Instant::now();
    let mut store = quipu::Store::open(&path).expect("open store");
    let mut triples = 0usize;
    for i in 0..n {
        let episode: quipu::episode::Episode =
            serde_json::from_str(&episode_json(i)).expect("episode parses");
        let ts = format!(
            "2026-08-13T{:02}:{:02}:{:02}Z",
            i % 24,
            i % 60,
            (i / 60) % 60
        );
        let (_tx, count, _outcome) =
            quipu::episode::ingest_episode_outcome(&mut store, &episode, &ts, NS)
                .expect("ingest succeeds");
        triples += count;
    }
    let ingest_ms = ms(t0);
    drop(store);

    let mut store = quipu::Store::open(&path).expect("reopen store");
    store.search_config_mut().query_timeout_ms = 600_000;
    let rm_build_ms = if read_model {
        store.set_read_model_enabled(true);
        let t = Instant::now();
        let _ = store.read_model().expect("read model builds");
        Some(ms(t))
    } else {
        None
    };

    print!(
        r#"{{"episodes":{n},"triples":{triples},"ingest_ms":{ingest_ms:.1},"rm_build_ms":{},"queries":{{"#,
        rm_build_ms.map_or("null".into(), |v| format!("{v:.1}")),
    );
    let queries = [
        (
            "point",
            format!("SELECT ?p ?o WHERE {{ <{NS}service-1> ?p ?o }}"),
        ),
        (
            "scan",
            format!("SELECT ?s WHERE {{ ?s a <{NS}Service> }} LIMIT 100"),
        ),
        (
            "join",
            format!("SELECT ?d ?s WHERE {{ ?d <{NS}targets> ?s . ?s a <{NS}Service> }} LIMIT 100"),
        ),
    ];
    for (idx, (label, sparql)) in queries.iter().enumerate() {
        let run = |sparql: &str| -> usize {
            match quipu::sparql::query(&store, sparql).expect("query succeeds") {
                quipu::sparql::QueryResult::Select { rows, .. } => rows.len(),
                _ => panic!("expected SELECT"),
            }
        };
        let t = Instant::now();
        let rows = run(sparql);
        let cold_ms = ms(t);
        let mut iters = 0u32;
        let warm_t = Instant::now();
        while iters < 30 && ms(warm_t) < 300.0 {
            run(sparql);
            iters += 1;
        }
        let warm_mean_ms = ms(warm_t) / f64::from(iters.max(1));
        if idx > 0 {
            print!(",");
        }
        print!(
            r#""{label}":{{"rows":{rows},"cold_ms":{cold_ms:.1},"warm_mean_ms":{warm_mean_ms:.3},"iters":{iters}}}"#
        );
    }
    println!("}}}}");
}
