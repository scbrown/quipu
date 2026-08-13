//! Browser test harness for quipu on `wasm32-unknown-unknown` (quipu-qd2).
//!
//! Runs inside a **dedicated Web Worker** (the opfs-sahpool VFS requires
//! `FileSystemSyncAccessHandle`, which only exists in workers). The Playwright
//! driver (`run.mjs`) loads `www/index.html`, which spawns the worker and
//! relays commands.
//!
//! The wasm side stays dumb on purpose: open a store, ingest synthetic
//! episodes (the `scale_bench` shape), run the three representative reads,
//! and report counts as JSON. Assertions and timing live in the driver.

use wasm_bindgen::prelude::*;

/// Same synthetic Gas Town-shaped episode as `examples/scale_bench.rs`.
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

fn err_js(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Install the opfs-sahpool VFS and make it the process-wide DEFAULT VFS, so
/// `Store::open(path)` — plain rusqlite `Connection::open` — lands on OPFS.
/// Call once per worker before any `scenario_*` that should persist.
#[wasm_bindgen]
pub async fn install_opfs() -> Result<(), JsValue> {
    use sqlite_wasm_vfs::sahpool::{OpfsSAHPoolCfg, install};
    install::<sqlite_wasm_rs::WasmOsCallback>(&OpfsSAHPoolCfg::default(), true)
        .await
        .map(|_| ())
        .map_err(err_js)
}

/// Open (or create) the store at `path` and ingest `n` synthetic episodes.
/// Returns `{"triples": N}` on success.
#[wasm_bindgen]
pub fn scenario_write(path: &str, n: u32) -> Result<String, JsValue> {
    let mut store = quipu::Store::open(path).map_err(err_js)?;
    let mut triples = 0usize;
    for i in 0..n as usize {
        let episode: quipu::episode::Episode =
            serde_json::from_str(&episode_json(i)).map_err(err_js)?;
        let ts = format!("2026-08-13T{:02}:{:02}:{:02}Z", i % 24, i % 60, (i / 60) % 60);
        let (_tx, count, _outcome) =
            quipu::episode::ingest_episode_outcome(&mut store, &episode, &ts, NS)
                .map_err(err_js)?;
        triples += count;
    }
    Ok(format!(r#"{{"triples":{triples}}}"#))
}

/// Timed bench for the quipu-ajz spike — the wasm half of the wasm-vs-native
/// ratio. MUST stay methodology-identical to
/// `examples/wasm_native_baseline.rs`: ingest `n` episodes; reopen; if
/// `read_model`, time the build explicitly; run each `scale_bench` query
/// once cold, then warm iterations until 300ms cumulative or 30 iters.
/// Timing is `Date.now()` (ms) — the iteration policy exists because of it.
#[wasm_bindgen]
pub fn scenario_bench(path: &str, n: u32, read_model: bool) -> Result<String, JsValue> {
    let now = js_sys::Date::now;

    let t0 = now();
    let mut store = quipu::Store::open(path).map_err(err_js)?;
    let mut triples = 0usize;
    for i in 0..n as usize {
        let episode: quipu::episode::Episode =
            serde_json::from_str(&episode_json(i)).map_err(err_js)?;
        let ts = format!("2026-08-13T{:02}:{:02}:{:02}Z", i % 24, i % 60, (i / 60) % 60);
        let (_tx, count, _outcome) =
            quipu::episode::ingest_episode_outcome(&mut store, &episode, &ts, NS)
                .map_err(err_js)?;
        triples += count;
    }
    let ingest_ms = now() - t0;
    drop(store);

    let mut store = quipu::Store::open(path).map_err(err_js)?;
    store.search_config_mut().query_timeout_ms = 600_000;
    let rm_build_ms = if read_model {
        store.set_read_model_enabled(true);
        let t = now();
        let _ = store.read_model().map_err(err_js)?;
        Some(now() - t)
    } else {
        None
    };

    let mut out = format!(
        r#"{{"episodes":{n},"triples":{triples},"ingest_ms":{ingest_ms:.1},"rm_build_ms":{},"queries":{{"#,
        rm_build_ms.map_or("null".into(), |ms| format!("{ms:.1}")),
    );
    let queries = [
        ("point", format!("SELECT ?p ?o WHERE {{ <{NS}service-1> ?p ?o }}")),
        ("scan", format!("SELECT ?s WHERE {{ ?s a <{NS}Service> }} LIMIT 100")),
        (
            "join",
            format!("SELECT ?d ?s WHERE {{ ?d <{NS}targets> ?s . ?s a <{NS}Service> }} LIMIT 100"),
        ),
    ];
    for (idx, (label, sparql)) in queries.iter().enumerate() {
        let run = |sparql: &str| -> Result<usize, JsValue> {
            match quipu::sparql::query(&store, sparql).map_err(err_js)? {
                quipu::sparql::QueryResult::Select { rows, .. } => Ok(rows.len()),
                _ => Err(JsValue::from_str("expected SELECT")),
            }
        };
        let t = now();
        let rows = run(sparql)?;
        let cold_ms = now() - t;
        let mut iters = 0u32;
        let warm_t = now();
        while iters < 30 && now() - warm_t < 300.0 {
            run(sparql)?;
            iters += 1;
        }
        let warm_mean_ms = (now() - warm_t) / f64::from(iters.max(1));
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#""{label}":{{"rows":{rows},"cold_ms":{cold_ms:.1},"warm_mean_ms":{warm_mean_ms:.3},"iters":{iters}}}"#
        ));
    }
    out.push_str("}}");
    Ok(out)
}

/// Export the store at `path` as the exact bytes of a `.db` file
/// (quipu-2l5). The driver writes them to disk and proves they open in the
/// `quipu` CLI and `sqlite3` unchanged.
#[wasm_bindgen]
pub fn scenario_export(path: &str) -> Result<Vec<u8>, JsValue> {
    let store = quipu::Store::open(path).map_err(err_js)?;
    store.serialize_db().map_err(err_js)
}

/// Round-trip the other direction: import `bytes` as a store and report the
/// three read counts, proving a native-produced `.db` opens in the browser.
#[wasm_bindgen]
pub fn scenario_import(bytes: &[u8]) -> Result<String, JsValue> {
    let store = quipu::Store::open_from_bytes(bytes)
        .map_err(|e| JsValue::from_str(&format!("open_from_bytes: {e}")))?;
    let count = |sparql: &str| -> Result<usize, JsValue> {
        match quipu::sparql::query(&store, sparql).map_err(err_js)? {
            quipu::sparql::QueryResult::Select { rows, .. } => Ok(rows.len()),
            _ => Err(JsValue::from_str("expected SELECT result")),
        }
    };
    let scan = count(&format!("SELECT ?s WHERE {{ ?s a <{NS}Service> }} LIMIT 100"))?;
    Ok(format!(r#"{{"scan":{scan}}}"#))
}

/// Produce a pack IN THE BROWSER (quipu-2l5): a labelled overlay graph with
/// two facts — one an object-position `Ref`, the case re-interning exists
/// for — packed to `.db` bytes via the same `pack_to_bytes` the native tests
/// cover. The driver ships the bytes out and attaches them to a native
/// store.
#[wasm_bindgen]
pub fn scenario_pack(path: &str) -> Result<Vec<u8>, JsValue> {
    use quipu::types::{Op, Value};
    let mut store = quipu::Store::open(path).map_err(err_js)?;
    let g = store
        .overlay_create("urn:g:browser-pack", 0)
        .map_err(err_js)?;
    let s = store.intern("http://example.org/s").map_err(err_js)?;
    let p = store.intern("http://example.org/p").map_err(err_js)?;
    let q = store.intern("http://example.org/q").map_err(err_js)?;
    let o = store.intern("http://example.org/o").map_err(err_js)?;
    store
        .overlay_write(g, Op::Assert, s, p, Value::Str("from-a-tab".into()), TS)
        .map_err(err_js)?;
    store
        .overlay_write(g, Op::Assert, s, q, Value::Ref(o), TS)
        .map_err(err_js)?;
    let (_manifest, bytes) = quipu::pack::pack_to_bytes(
        &store,
        "urn:g:browser-pack",
        &quipu::pack::PackOptions::default(),
        TS,
    )
    .map_err(err_js)?;
    Ok(bytes)
}

const TS: &str = "2026-08-13T00:00:00Z";

/// What journal mode does a store on this VFS actually get? `Store::init`
/// requests WAL; a VFS without shared-memory support keeps the prior mode
/// instead of erroring, and this pins which mode that is. Opens a THROWAWAY
/// db at `path` (neither wasm VFS supports two connections to one file).
#[wasm_bindgen]
pub fn journal_mode(path: &str) -> Result<String, JsValue> {
    let conn = rusqlite::Connection::open(path).map_err(err_js)?;
    let requested: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .map_err(err_js)?;
    let effective: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(err_js)?;
    Ok(format!(
        r#"{{"requested_wal_got":"{requested}","effective":"{effective}"}}"#
    ))
}

/// Open the store at `path` READ-side and run the three representative reads
/// from `scale_bench`: bound-subject point lookup, type scan, 2-hop join.
/// Returns `{"point": N, "scan": N, "join": N}` — row counts, which the
/// driver asserts against what `scenario_write` ingested.
#[wasm_bindgen]
pub fn scenario_read(path: &str) -> Result<String, JsValue> {
    let store = quipu::Store::open(path).map_err(err_js)?;
    let count = |sparql: &str| -> Result<usize, JsValue> {
        match quipu::sparql::query(&store, sparql).map_err(err_js)? {
            quipu::sparql::QueryResult::Select { rows, .. } => Ok(rows.len()),
            _ => Err(JsValue::from_str("expected SELECT result")),
        }
    };
    let point = count(&format!("SELECT ?p ?o WHERE {{ <{NS}service-1> ?p ?o }}"))?;
    let scan = count(&format!("SELECT ?s WHERE {{ ?s a <{NS}Service> }} LIMIT 100"))?;
    let join = count(&format!(
        "SELECT ?d ?s WHERE {{ ?d <{NS}targets> ?s . ?s a <{NS}Service> }} LIMIT 100"
    ))?;
    Ok(format!(r#"{{"point":{point},"scan":{scan},"join":{join}}}"#))
}
