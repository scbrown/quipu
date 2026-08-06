//! Tests for the server's lock discipline and handler wiring.

use std::sync::Arc;

use axum::extract::State;
use quipu::{EmbeddingProvider, Store};
use serde_json::json;

use super::SharedStore;
use super::base::{STATS_CACHE, StatsCache, stats};
use super::tools::search;

/// An embedding provider whose embed is deliberately SLOW, so that whether the
/// store lock is held across it is observable in wall-clock time.
struct SleepyProvider;
impl EmbeddingProvider for SleepyProvider {
    fn embed_text(&self, _text: &str) -> quipu::Result<Vec<f32>> {
        std::thread::sleep(std::time::Duration::from_millis(200));
        Ok(vec![0.1f32; 384])
    }
    fn dimension(&self) -> usize {
        384
    }
}

/// m4s2: the query-embed must run OUTSIDE the global `Store` mutex, so
/// concurrent /search embeds OVERLAP instead of serializing on the lock.
///
/// Deterministic stand-in for the ONNX 30/50-concurrency probe: with a
/// provider that sleeps 200ms per embed, four concurrent searches finish in
/// ~one embed's time when the lock is released across the embed (the fix), and
/// in ~four embeds' time when it is not. The 600ms threshold sits between the
/// parallel (~200-300ms) and serial (~800ms) outcomes, so this test PASSES on
/// the embed-outside-lock handler and FAILS on the pre-fix in-lock one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_search_embeds_do_not_serialize_on_the_store_lock() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(SleepyProvider));
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));

    let call = || {
        let s = shared.clone();
        async move { search(State(s), axum::Json(json!({ "query": "x", "limit": 1 }))).await }
    };

    let start = std::time::Instant::now();
    let (a, b, c, d) = tokio::join!(call(), call(), call(), call());
    let elapsed = start.elapsed();

    for r in [a, b, c, d] {
        assert!(
            r.is_ok(),
            "search over an empty store should succeed with a scoped:false empty result"
        );
    }
    assert!(
        elapsed < std::time::Duration::from_millis(600),
        "four concurrent embeds serialized on the store lock ({elapsed:?}) — the embed is \
         being held inside the mutex again (m4s2 regressed)"
    );
}

/// Guard the fix's invariant: a caller-supplied `embedding` must pass straight
/// through, never touching the provider — so pre-computed callers keep working
/// and the injection path only fires for text queries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn precomputed_embedding_bypasses_the_provider() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(SleepyProvider)); // would sleep 200ms if consulted
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));

    let start = std::time::Instant::now();
    let r = search(
        State(shared),
        axum::Json(json!({ "embedding": vec![0.2f32; 384], "limit": 1 })),
    )
    .await;
    assert!(r.is_ok(), "a pre-computed embedding search should succeed");
    assert!(
        start.elapsed() < std::time::Duration::from_millis(150),
        "the sleepy provider was consulted despite a pre-computed embedding"
    );
}

/// /stats returns the generation-keyed cache when `Store::latest_tx_id()`
/// is unchanged, and recomputes when it moves (a write). Both paths are proven by
/// poisoning `STATS_CACHE` at a matching vs a stale generation — matching returns the
/// poison (so the scan was skipped), stale is ignored and the true stats recomputed —
/// which needs no triple-insert plumbing and is fully deterministic.
#[tokio::test]
async fn stats_uses_generation_cache_and_invalidates_on_write() {
    let store = Store::open_in_memory().unwrap();
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));

    // Baseline on the empty store (facts == 0), at the current generation.
    *STATS_CACHE.lock().unwrap() = None;
    let base = stats(State(shared.clone()))
        .await
        .ok()
        .expect("stats handler should succeed")
        .0;
    assert_eq!(
        base["facts"].as_u64().unwrap(),
        0,
        "empty store has 0 facts"
    );
    let cur_gen = shared.lock().latest_tx_id().unwrap();

    // Cache HIT: poison at the CURRENT generation. The handler must return the
    // poison — proving it read the cache and did NOT re-scan (a scan would give 0).
    *STATS_CACHE.lock().unwrap() = Some(StatsCache {
        generation: cur_gen,
        value: Arc::new(json!({ "facts": 42, "entities": 7, "predicates": 3 })),
    });
    let hit = stats(State(shared.clone()))
        .await
        .ok()
        .expect("stats handler should succeed")
        .0;
    assert_eq!(
        hit["facts"].as_u64().unwrap(),
        42,
        "matching generation must return the cached aggregate, not re-scan"
    );

    // INVALIDATION: poison at a STALE generation, exactly as a write leaves the
    // cache. The handler must ignore it and recompute the true (empty) stats.
    *STATS_CACHE.lock().unwrap() = Some(StatsCache {
        generation: cur_gen - 1,
        value: Arc::new(json!({ "facts": 42, "entities": 7, "predicates": 3 })),
    });
    let fresh = stats(State(shared.clone()))
        .await
        .ok()
        .expect("stats handler should succeed")
        .0;
    assert_eq!(
        fresh["facts"].as_u64().unwrap(),
        0,
        "a stale generation must be ignored and the stats recomputed"
    );
}

/// A provider that RECORDS the size of every batch it is handed, so the
/// chunking bound is observable rather than assumed.
struct RecordingProvider {
    batches: Arc<parking_lot::Mutex<Vec<usize>>>,
}
impl EmbeddingProvider for RecordingProvider {
    fn embed_text(&self, _text: &str) -> quipu::Result<Vec<f32>> {
        self.batches.lock().push(1);
        Ok(vec![0.1f32; 8])
    }
    fn embed_batch(&self, texts: &[&str]) -> quipu::Result<Vec<Vec<f32>>> {
        self.batches.lock().push(texts.len());
        Ok(texts.iter().map(|_| vec![0.1f32; 8]).collect())
    }
    fn dimension(&self) -> usize {
        8
    }
}

/// `finish_deferred_embed` must CHUNK the ONNX call by `embed_batch_size`,
/// never hand the whole drained work set to one `embed_batch`.
///
/// Why this is a memory bug and not a throughput nit: one `embed_batch(N)`
/// builds N sequences' worth of ONNX Runtime attention activations, and ORT's
/// arena allocator never returns that memory to the OS. Measured on the
/// deployed model/config (all-MiniLM-L6-v2, seq 256, `intra_threads` 1): ~7.4 MB
/// retained PER TEXT, linear in N — 2048 texts in one call cost +15.1 GB that
/// later small calls did not reclaim, while the same 2048 texts in chunks of 32
/// cost +492 MB and plateaued flat. The resident high-water is set by the
/// LARGEST SINGLE BATCH, so this bound is what keeps RSS O(batch) instead of
/// O(entities touched). In production the unbounded path took quipu-server from
/// 350 MB to 5.83 GB on one drain and held it there.
///
/// The deferred path also MERGES work across transactions before draining, so
/// without this bound the batch grows with write volume, not just per-write
/// fan-out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deferred_embed_chunks_the_onnx_call_by_batch_size() {
    use quipu::KnowledgeVectorStore as _;

    let batches: Arc<parking_lot::Mutex<Vec<usize>>> = Arc::new(parking_lot::Mutex::new(vec![]));

    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(RecordingProvider {
        batches: batches.clone(),
    }));
    store.embedding_config_mut().auto_embed = true;
    store.embedding_config_mut().embed_batch_size = 8;
    store.embedding_config_mut().dimension = 8;
    store.set_defer_auto_embed(true);

    // 50 embeddable entities in one write -> one drained work set of 50.
    let mut turtle = String::from("@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n");
    for i in 0..50 {
        turtle.push_str(&format!(
            "<http://example.org/e{i}> rdfs:label \"Entity {i}\" .\n"
        ));
    }
    quipu::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let work = store
        .take_deferred_embed()
        .expect("a write touching 50 embeddable entities must queue work");
    assert_eq!(work.texts().len(), 50, "all 50 entities should be queued");

    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));
    // AppError deliberately has no Debug impl, so assert rather than expect().
    assert!(
        super::tools::finish_deferred_embed(&shared, &work).is_ok(),
        "deferred embed should succeed"
    );

    let seen = batches.lock().clone();
    assert!(!seen.is_empty(), "the provider was never called");

    // THE BOUND: no single ONNX call may exceed embed_batch_size. Pre-fix this
    // is a single call of 50 and the assert fails.
    let largest = *seen.iter().max().unwrap();
    assert!(
        largest <= 8,
        "finish_deferred_embed handed ONNX a batch of {largest} against embed_batch_size=8 \
         (call sizes {seen:?}) — the unbounded embed_batch is back, and with it an RSS \
         high-water proportional to the entities one drain touched"
    );

    // And chunking must not LOSE work: every entity still gets embedded.
    assert_eq!(
        seen.iter().sum::<usize>(),
        50,
        "chunking dropped or duplicated texts (call sizes {seen:?})"
    );
    assert_eq!(
        shared.lock().vector_count().unwrap(),
        50,
        "every queued entity should end up with a vector"
    );
}

/// Every local JS module the UI imports must be a registered route AND
/// reachable without a bearer token, or the page dead-ends on a blank view.
///
/// Both halves of this bit me while wiring the 3D Datalinks view: the module
/// resolves at build time via `include_str!`, so a missing `.route()` or a
/// missing `http_auth` allowlist entry compiles clean and only shows up as an
/// empty canvas at runtime. Parsing the HTML keeps the check honest — adding an
/// import without serving it now fails here instead of in a browser.
#[test]
fn every_ui_module_import_is_a_served_public_route() {
    let html = super::UI_HTML;
    let mut imports: Vec<&str> = Vec::new();
    for line in html.lines() {
        // `from '/path.js'` (ES import) and `src="/path.js"` (classic script).
        for (open, close) in [("from '", '\''), ("src=\"", '"')] {
            if let Some(idx) = line.find(open) {
                let rest = &line[idx + open.len()..];
                if let Some(end) = rest.find(close) {
                    let path = &rest[..end];
                    // Only local absolute paths; CDN scripts are not ours to serve.
                    if path.starts_with('/') {
                        imports.push(path);
                    }
                }
            }
        }
    }
    assert!(
        imports.len() >= 2,
        "found {} local module imports in ui/index.html — the parse likely broke",
        imports.len()
    );

    let served = quipu::http_auth::READ_ENDPOINTS;
    let mut missing = Vec::new();
    for path in &imports {
        if !served.contains(path) {
            missing.push(*path);
        }
    }
    assert!(
        missing.is_empty(),
        "ui/index.html imports these local modules, but they are not in \
         READ_ENDPOINTS so they are either unrouted or behind auth: {missing:?}"
    );
}

/// The 3D dependency is vendored, never fetched: an air-gapped deploy has to
/// render. Guards both that the vendored file is really three.js and that the
/// UI never reaches for a CDN to get it.
#[test]
fn three_js_is_vendored_and_never_fetched() {
    assert!(
        super::THREE_JS.contains("Three.js Authors")
            && super::THREE_JS.contains("SPDX-License-Identifier: MIT"),
        "ui/vendor/three.module.min.js is missing the three.js MIT header — \
         either it is not three.js, or the licence banner was stripped"
    );
    for module in [super::DATALINKS_JS, super::UI_HTML] {
        for host in ["unpkg.com", "cdn.jsdelivr.net", "cdn.skypack.dev"] {
            assert!(
                !module.contains(&format!("{host}/three")),
                "the 3D view must not load three.js from {host} — it is vendored"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Read-connection pool
//
// The bead's acceptance is a CURVE, because a pool that silently serialises —
// size 1, or every reader contending on one connection — reproduces the
// pre-pool numbers exactly. These tests pin the three ways that happens, in
// process, so a regression fails CI instead of failing a benchmark somebody has
// to remember to run.
// ---------------------------------------------------------------------------

/// Build a file-backed handle with a real pool, plus the tempdir that owns the
/// database file for the test's lifetime.
fn pooled_handle(readers: usize) -> (tempfile::TempDir, super::StoreHandle) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db").to_str().unwrap().to_string();
    let store = Store::open(&path).unwrap();
    let mut conns = Vec::new();
    for _ in 0..readers {
        let mut r = Store::open_read_only(&path).unwrap();
        r.adopt_read_config_from(&store);
        conns.push(parking_lot::FairMutex::new(r));
    }
    let handle = super::StoreHandle {
        writer: parking_lot::FairMutex::new(store),
        readers: super::ReadPool {
            conns,
            next: std::sync::atomic::AtomicUsize::new(0),
        },
    };
    (dir, handle)
}

/// A pooled reader must see what the writer COMMITTED. This is the failure that
/// would be catastrophic and silent: reads served from a different or empty
/// database still answer 200, just with nothing in them. It is exactly what a
/// pool over `:memory:` would do, which is why the server refuses to build one
/// there.
#[test]
fn pooled_read_sees_the_writers_committed_facts() {
    let (_dir, h) = pooled_handle(2);

    // Control: BEFORE the write, the pool must not already know the answer.
    let before = quipu::tool_query(
        &h.read(),
        &json!({"query": "SELECT ?s WHERE { ?s a <http://example.org/Widget> }"}),
    )
    .unwrap();
    assert_eq!(
        before["count"], 0,
        "control failed — the pool answered before the fact existed: {before}"
    );

    quipu::tool_knot(
        &mut h.lock(),
        &json!({"turtle": "@prefix ex: <http://example.org/> . ex:thing a ex:Widget .",
                "timestamp": "2026-01-01", "actor": "test"}),
    )
    .unwrap();

    let after = quipu::tool_query(
        &h.read(),
        &json!({"query": "SELECT ?s WHERE { ?s a <http://example.org/Widget> }"}),
    )
    .unwrap();
    assert_eq!(
        after["count"], 1,
        "a pooled reader did not see the writer's committed fact — WAL visibility \
         is the whole premise of the pool: {after}"
    );
}

/// Run `f` on a thread and FAIL — rather than hang — if it does not finish.
///
/// Every assertion below is about a lock being AVAILABLE, and the way that
/// breaks is a deadlock, not a wrong value. Asserting it inline would hang the
/// whole suite on regression, which in CI is indistinguishable from an
/// infrastructure stall and gets retried rather than read. This turns the hang
/// into a named failure. (Learned by sabotage: the first version of these tests
/// detected the defect correctly and reported it as a 240s timeout.)
fn must_not_block<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(f()));
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(v) => v,
        Err(_) => panic!(
            "{what} BLOCKED for 5s — a read is queueing where it must not. \
             This is the silent-serialisation failure: the pool would reproduce \
             the pre-pool curve exactly while appearing to work."
        ),
    }
}

/// The pool must hand CONCURRENT readers DIFFERENT connections. A pool that
/// returns the same connection to everyone reproduces the pre-pool curve
/// exactly while looking like a fix — the specific false negative the bead
/// warns the acceptance must be able to detect.
#[test]
fn concurrent_readers_get_distinct_connections() {
    let (_dir, h) = pooled_handle(3);
    let h = Arc::new(h);

    // Hold one reader, then prove two MORE can be acquired without blocking.
    let first = h.read();
    let first_ptr = std::ptr::from_ref::<quipu::Store>(&*first) as usize;

    let h2 = h.clone();
    let second_ptr = must_not_block("second concurrent read", move || {
        let g = h2.read();
        std::ptr::from_ref::<quipu::Store>(&*g) as usize
    });
    let h3 = h.clone();
    let third_ptr = must_not_block("third concurrent read", move || {
        let g = h3.read();
        std::ptr::from_ref::<quipu::Store>(&*g) as usize
    });

    assert_ne!(
        first_ptr, second_ptr,
        "two concurrent readers were handed the SAME connection"
    );
    assert_ne!(
        first_ptr, third_ptr,
        "two concurrent readers were handed the SAME connection"
    );
}

/// A read must never take the WRITER's lock while a connection is free —
/// otherwise readers still queue behind writes and the pool buys nothing under
/// the write flood it exists to survive (the mfg0 case: a `SELECT ... LIMIT 1`
/// measured waiting 38.5s behind a write flood).
#[test]
fn a_read_does_not_queue_behind_a_held_writer_lock() {
    let (_dir, h) = pooled_handle(2);
    let h = Arc::new(h);
    let _writer_held = h.lock();

    let h2 = h.clone();
    let ok = must_not_block("read while the writer lock is held", move || {
        let r = h2.read();
        quipu::tool_query(
            &r,
            &json!({"query": "SELECT ?s WHERE { ?s ?p ?o } LIMIT 1"}),
        )
        .is_ok()
    });
    assert!(ok, "a pooled read failed while the writer lock was held");
}

/// An EMPTY pool must still serve reads — from the writer — because that is the
/// configured rollback (`read_pool_size = 0`) and the in-memory path. A fallback
/// that panicked or divided by zero would turn a safe knob into an outage.
#[test]
fn an_empty_pool_falls_back_to_the_writer() {
    let h = super::StoreHandle::writer_only(Store::open_in_memory().unwrap());
    let out = quipu::tool_query(&h.read(), &json!({"query": "SELECT ?s WHERE { ?s ?p ?o }"}));
    assert!(out.is_ok(), "empty-pool fallback failed: {out:?}");
}

/// Pool connections are opened READ-ONLY. The borrow checker cannot enforce
/// this the way `&Store` vs `&mut Store` does in the tool layer, so `SQLite` is
/// the mechanism: a write through a pooled connection must FAIL, not race.
#[test]
fn pooled_connections_refuse_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro.db").to_str().unwrap().to_string();
    // Control: the writer CAN write to this database, so a failure below is
    // about the connection's mode and not about the database being unusable.
    let mut w = Store::open(&path).unwrap();
    quipu::tool_knot(
        &mut w,
        &json!({"turtle": "@prefix ex: <http://example.org/> . ex:a a ex:B .",
                "timestamp": "2026-01-01", "actor": "t"}),
    )
    .unwrap();

    let mut r = Store::open_read_only(&path).unwrap();
    let err = quipu::tool_knot(
        &mut r,
        &json!({"turtle": "@prefix ex: <http://example.org/> . ex:c a ex:D .",
                "timestamp": "2026-01-02", "actor": "t"}),
    );
    assert!(
        err.is_err(),
        "a write through a read-only pool connection SUCCEEDED — the pool's \
         read-only guarantee is not enforced by anything"
    );
}

/// Every tool registered as `ro_handler!` must survive a READ-ONLY connection.
///
/// This is the safety half of the read pool, and it cannot be delegated to the
/// type system: `Store::intern` takes `&self` and INSERTs, as do `load_shapes`,
/// `remove_shapes`, `load_ontology` and `remove_ontology`. So `&Store` says
/// nothing about whether a tool writes, and two handlers in this file were
/// already caught mis-registered as `ro_handler!` on exactly that mistake.
///
/// A writing tool on a pooled connection does not corrupt anything — the
/// connection is `SQLITE_OPEN_READ_ONLY`, so `SQLite` refuses — but it does turn a
/// working read endpoint into a 500. That is the regression this pins.
///
/// The discriminator is the `SQLite` read-only error specifically, not "did it
/// error": most of these tools error on the deliberately-thin inputs below, and
/// that is fine. Only `attempt to write a readonly database` is a failure.
/// Validated against the real message rather than a guessed one.
#[test]
fn every_pooled_tool_survives_a_read_only_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro-tools.db").to_str().unwrap().to_string();
    {
        let mut w = Store::open(&path).unwrap();
        quipu::tool_knot(
            &mut w,
            &json!({"turtle": "@prefix ex: <http://example.org/> . ex:n a ex:Widget ; ex:label \"n\" .",
                    "timestamp": "2026-01-01", "actor": "t"}),
        )
        .unwrap();
    }
    let ro = Store::open_read_only(&path).unwrap();

    // POSITIVE CONTROL FIRST. If a known write does NOT produce the error this
    // test greps for, every "pass" below is vacuous — an all-clear from an
    // instrument that cannot detect the failure.
    let control = ro.intern("http://example.org/definitely-new");
    let control_msg = format!("{control:?}").to_lowercase();
    assert!(
        control_msg.contains("readonly") || control_msg.contains("read-only"),
        "CONTROL FAILED — a write through a read-only connection did not report a \
         read-only error, so this test cannot detect the thing it exists to catch. \
         Got: {control:?}"
    );

    let entity = "http://example.org/n";
    /// One registered read-only tool: name, entry point, minimal valid input.
    type ToolCase = (
        &'static str,
        fn(&Store, &serde_json::Value) -> quipu::Result<serde_json::Value>,
        serde_json::Value,
    );
    let cases: Vec<ToolCase> = vec![
        (
            "tool_query",
            quipu::tool_query,
            json!({"query": "SELECT ?s WHERE { ?s ?p ?o } LIMIT 2"}),
        ),
        (
            "tool_cord",
            quipu::tool_cord,
            json!({"type": "http://example.org/Widget", "limit": 3}),
        ),
        (
            "tool_graph_view",
            quipu::tool_graph_view,
            json!({"limit": 5}),
        ),
        ("tool_unravel", quipu::tool_unravel, json!({"tx": 1})),
        (
            "tool_search_nodes",
            quipu::tool_search_nodes,
            json!({"query": "n", "limit": 3}),
        ),
        (
            "tool_search_facts",
            quipu::tool_search_facts,
            json!({"query": "n", "limit": 3}),
        ),
        (
            "tool_unified_search",
            quipu::tool_unified_search,
            json!({"query": "n", "limit": 3}),
        ),
        (
            "tool_ask",
            quipu::tool_ask,
            json!({"question": "what is n?"}),
        ),
        (
            "tool_resolve_entity",
            quipu::tool_resolve_entity,
            json!({"name": "n"}),
        ),
        (
            "tool_cooccurrence",
            quipu::tool_cooccurrence,
            json!({"limit": 5}),
        ),
        (
            "tool_context",
            quipu::tool_context,
            json!({"entity": entity}),
        ),
        ("tool_report", quipu::tool_report, json!({})),
        ("tool_list_proposals", quipu::tool_list_proposals, json!({})),
        (
            "tool_overlay_compose",
            quipu::tool_overlay_compose,
            json!({"overlays": []}),
        ),
    ];

    for (name, f, input) in cases {
        let got = f(&ro, &input);
        let msg = format!("{got:?}").to_lowercase();
        assert!(
            !(msg.contains("readonly") || msg.contains("attempt to write")),
            "{name} attempted a WRITE on a pooled read-only connection. Either it \
             belongs on rw_handler! (like set_predicate and the named-graphs \
             registry before it), or it needs a read path that does not intern. \
             Got: {got:?}"
        );
    }
}
