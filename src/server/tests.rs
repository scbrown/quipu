//! Tests for the server's lock discipline and handler wiring.

use std::sync::Arc;

use axum::extract::State;
use parking_lot::FairMutex;
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
    let shared: SharedStore = Arc::new(FairMutex::new(store));

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
    let shared: SharedStore = Arc::new(FairMutex::new(store));

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
    let shared: SharedStore = Arc::new(FairMutex::new(store));

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

    let shared: SharedStore = Arc::new(FairMutex::new(store));
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
