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
