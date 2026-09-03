//! Tests for the server's lock discipline and handler wiring.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    response::IntoResponse,
};
use quipu::{EmbeddingProvider, Store};
use serde_json::json;

use super::SharedStore;
use super::base::{
    MAX_QUERY_URI_BYTES, QueryParams, STATS_CACHE, StatsCache, metrics_handler, query, query_get,
    query_post, stats,
};
use super::entity::{EntityParams, SPOTLIGHT_CACHE, entity_query_conneg, spotlight_handler};
use super::publication::{export as export_handler, share_payload};
use super::tools::{episode, search};

#[tokio::test]
async fn query_form_entity_dereferences_json_ld_and_html() {
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    let iri = "https://example.org/resource/one#it";

    let mut json_headers = axum::http::HeaderMap::new();
    json_headers.insert(
        axum::http::header::ACCEPT,
        "application/ld+json".parse().unwrap(),
    );
    let json_response = entity_query_conneg(
        State(shared.clone()),
        axum::extract::Query(EntityParams {
            iri: iri.into(),
            expanded: None,
        }),
        json_headers,
    )
    .await
    .unwrap();
    assert_eq!(
        json_response.headers()[axum::http::header::CONTENT_TYPE],
        "application/ld+json"
    );
    let body = axum::body::to_bytes(json_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(document["@id"], iri);
    assert!(document.get("@context").is_some());

    let html_response = entity_query_conneg(
        State(shared),
        axum::extract::Query(EntityParams {
            iri: iri.into(),
            expanded: None,
        }),
        axum::http::HeaderMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        html_response.headers()[axum::http::header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let body = axum::body::to_bytes(html_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("class=\"card\""));
    assert!(html.contains(">it</div>"));
}

#[tokio::test]
async fn post_share_returns_reconstructable_canonical_files() {
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes(
            "http-share-test",
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
            "2026-08-29",
        )
        .unwrap();
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));

    let response = share_payload(
        State(shared),
        axum::Json(quipu::share::SharePayloadRequest::default()),
    )
    .await
    .expect("POST /share should return a remote payload")
    .0;
    let hash = |bytes: &[u8]| {
        let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
        format!("sha256:{}", hex::encode(digest.as_ref()))
    };

    assert_eq!(response.files.len(), 3);
    assert_eq!(
        response.manifest.graph_hash,
        hash(response.files["export.nt"].as_bytes())
    );
    assert_eq!(
        response.manifest.shapes_hash,
        hash(response.files["shapes.ttl"].as_bytes())
    );
    let manifest: quipu::share::ShareManifest =
        serde_json::from_str(&response.files["manifest.json"]).unwrap();
    assert_eq!(manifest, response.manifest);
}

/// aegis-ibft0 acceptance: prove the HTTP handler's NEGATIVE outcome. A clean
/// payload passing would also pass on the buggy append-two-comments behavior.
#[tokio::test]
async fn post_episode_refuses_one_node_name_twice_and_writes_nothing() {
    let store = Store::open_in_memory().unwrap();
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));
    let input = json!({
        "name": "duplicate-description-payload",
        "nodes": [
            {"name": "one-rule", "type": "Directive", "description": "first"},
            {"name": "one-rule", "type": "FailureMode", "description": "second"}
        ]
    });

    let err = episode(State(shared.clone()), axum::Json(input))
        .await
        .expect_err("POST /episode must refuse a repeated node name");
    let response = err.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert!(
        shared
            .lock()
            .lookup("http://aegis.gastown.local/ontology/one-rule")
            .unwrap()
            .is_none(),
        "the refused HTTP request must commit no partial node"
    );
}

#[tokio::test]
async fn query_response_carries_structured_usage_metadata() {
    let store = Store::open_in_memory().unwrap();
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));
    let response = query(
        State(shared),
        axum::http::HeaderMap::new(),
        axum::Json(json!({"query": "SELECT ?s WHERE { ?s ?p ?o }"})),
    )
    .await
    .unwrap();
    assert_eq!(
        response
            .extensions()
            .get::<quipu::request_usage::RequestUsage>(),
        Some(&quipu::request_usage::RequestUsage {
            query_shape: quipu::request_usage::QueryShape::Select,
            result_size: 0,
        })
    );
}

fn query_headers(content_type: &str, accept: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        content_type.parse().unwrap(),
    );
    headers.insert(axum::http::header::ACCEPT, accept.parse().unwrap());
    headers
}

#[tokio::test]
async fn sparql_protocol_get_negotiates_select_results() {
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    let response = query_get(
        State(shared),
        query_headers("application/json", "application/sparql-results+json"),
        OriginalUri(
            "/query?query=SELECT%20%3Fs%20WHERE%20%7B%20%3Fs%20%3Fp%20%3Fo%20%7D"
                .parse()
                .unwrap(),
        ),
        Query(QueryParams {
            query: "SELECT ?s WHERE { ?s ?p ?o }".into(),
            verbose: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers()[axum::http::header::CONTENT_TYPE],
        "application/sparql-results+json"
    );
}

#[tokio::test]
async fn sparql_protocol_post_supports_ask_and_graph_negotiation() {
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    let ask = query_post(
        State(shared.clone()),
        query_headers(
            "application/sparql-query; charset=utf-8",
            "application/sparql-results+xml",
        ),
        Bytes::from_static(b"ASK { ?s ?p ?o }"),
    )
    .await
    .unwrap();
    assert_eq!(ask.status(), axum::http::StatusCode::OK);
    assert_eq!(
        ask.headers()[axum::http::header::CONTENT_TYPE],
        "application/sparql-results+xml"
    );

    let graph = query_post(
        State(shared),
        query_headers("application/sparql-query", "text/turtle"),
        Bytes::from_static(b"CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"),
    )
    .await
    .unwrap();
    assert_eq!(graph.status(), axum::http::StatusCode::OK);
    assert_eq!(
        graph.headers()[axum::http::header::CONTENT_TYPE],
        "text/turtle"
    );
}

#[tokio::test]
async fn sparql_protocol_negotiates_csv_and_tsv() {
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    for (accept, expected) in [
        ("text/csv", "text/csv; charset=utf-8"),
        (
            "text/tab-separated-values",
            "text/tab-separated-values; charset=utf-8",
        ),
    ] {
        let response = query_post(
            State(shared.clone()),
            query_headers("application/sparql-query", accept),
            Bytes::from_static(b"SELECT ?s WHERE { ?s ?p ?o }"),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            expected
        );
    }
}

#[tokio::test]
async fn legacy_json_post_and_protocol_rejections_are_preserved() {
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    let legacy = query_post(
        State(shared.clone()),
        query_headers("application/json", "application/json"),
        Bytes::from_static(br#"{"query":"ASK { ?s ?p ?o }"}"#),
    )
    .await
    .unwrap();
    assert_eq!(legacy.status(), axum::http::StatusCode::OK);

    let unsupported = query_post(
        State(shared.clone()),
        query_headers("text/plain", "application/json"),
        Bytes::from_static(b"ASK {}"),
    )
    .await
    .unwrap();
    assert_eq!(
        unsupported.status(),
        axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );

    let long_uri = format!("/query?query={}", "x".repeat(MAX_QUERY_URI_BYTES));
    let too_long = query_get(
        State(shared),
        axum::http::HeaderMap::new(),
        OriginalUri(long_uri.parse().unwrap()),
        Query(QueryParams {
            query: "ASK {}".into(),
            verbose: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(too_long.status(), axum::http::StatusCode::URI_TOO_LONG);
}

#[tokio::test]
async fn sparql_protocol_post_uses_the_existing_query_deadline() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = (0..100)
        .map(|i| format!("<http://example.org/s{i}> <http://example.org/p> \"{i}\" ."))
        .collect::<Vec<_>>()
        .join("\n");
    quipu::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-09-02T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    store.search_config_mut().query_timeout_ms = 1;
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));
    let err = query_post(
        State(shared),
        query_headers(
            "application/sparql-query",
            "application/sparql-results+json",
        ),
        Bytes::from_static(b"SELECT ?s WHERE { ?s ?p ?o . ?a ?b ?c . ?d ?e ?f . ?g ?h ?i }"),
    )
    .await
    .expect_err("a standard transport must retain the configured query deadline");
    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::REQUEST_TIMEOUT
    );
}

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

/// Supplied-vector search is read-only. It must use the WAL read pool rather
/// than queueing behind an unrelated writer, or concurrent mixed load turns
/// every search into writer-lock latency.
#[tokio::test(flavor = "current_thread")]
async fn precomputed_search_does_not_queue_behind_the_writer() {
    let (_dir, handle) = pooled_handle(1);
    let shared = Arc::new(handle);
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let thread = std::thread::spawn(move || {
        let _writer = holder.lock();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        search(
            State(shared),
            axum::Json(json!({ "embedding": vec![0.2f32; 384], "limit": 1 })),
        ),
    )
    .await
    .expect("precomputed search queued behind the writer instead of using the read pool")
    .expect("precomputed search failed on a read-only pooled connection");
    release_tx.send(()).unwrap();
    thread.join().unwrap();
}

/// Tokio cannot cancel blocking tasks after they start. Admission therefore
/// has to happen in async space: cancelling a waiter must prevent its closure
/// from ever entering the blocking pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_write_waiter_never_starts_blocking_work() {
    let admission = Box::leak(Box::new(tokio::sync::Semaphore::new(1)));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let leader = tokio::spawn(super::admission::write_blocking_with(
        admission,
        move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        },
    ));
    started_rx.await.unwrap();

    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ran_in_waiter = ran.clone();
    let waiter = tokio::spawn(super::admission::write_blocking_with(
        admission,
        move || {
            ran_in_waiter.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        },
    ));
    tokio::task::yield_now().await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());

    release_tx.send(()).unwrap();
    leader.await.unwrap().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(
        !ran.load(std::sync::atomic::Ordering::SeqCst),
        "cancelled write entered spawn_blocking and survived its HTTP future"
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

/// Blocks the first batch until the test releases it, making the unlocked
/// ONNX window deterministic instead of relying on scheduler timing.
struct BlockingBatchProvider {
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    calls: std::sync::atomic::AtomicUsize,
}
impl EmbeddingProvider for BlockingBatchProvider {
    fn embed_text(&self, _text: &str) -> quipu::Result<Vec<f32>> {
        Ok(vec![0.1f32; 8])
    }
    fn embed_batch(&self, texts: &[&str]) -> quipu::Result<Vec<Vec<f32>>> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            if let Some(tx) = self.entered.lock().unwrap().take() {
                tx.send(()).unwrap();
            }
            self.release.lock().unwrap().recv().unwrap();
        }
        Ok(texts.iter().map(|_| vec![0.1f32; 8]).collect())
    }
    fn dimension(&self) -> usize {
        8
    }
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

#[test]
fn backfill_replaces_stale_current_vector() {
    use quipu::KnowledgeVectorStore as _;

    let batches = Arc::new(parking_lot::Mutex::new(vec![]));
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(RecordingProvider { batches }));
    store.embedding_config_mut().dimension = 8;

    let entity = store.intern("http://example.org/entity").unwrap();
    store
        .vector_store()
        .embed_entity(entity, "obsolete text", &[0.1; 8], "1")
        .unwrap();
    quipu::ingest_rdf(
        &mut store,
        &b"<http://example.org/entity> <http://www.w3.org/2000/01/rdf-schema#label> \"corrected text\" ."[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));
    assert_eq!(
        super::tools::backfill_embeddings(&shared).unwrap().embedded,
        1
    );
    assert_eq!(
        shared.lock().vector_count().unwrap(),
        1,
        "old vector stayed current"
    );
    let matches = shared.lock().vector_search(&[0.1; 8], 1, None).unwrap();
    assert_eq!(matches[0].text, "corrected text");
}

#[test]
fn backfill_enumerates_subjects_without_sparql() {
    use quipu::KnowledgeVectorStore as _;

    let batches = Arc::new(parking_lot::Mutex::new(vec![]));
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(RecordingProvider { batches }));
    store.embedding_config_mut().dimension = 8;
    quipu::ingest_rdf(
        &mut store,
        &b"<http://example.org/a> <http://example.org/p> \"one\" .\n<http://example.org/a> <http://example.org/q> \"two\" .\n<http://example.org/b> <http://example.org/p> \"three\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();

    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));
    assert_eq!(
        super::tools::backfill_embeddings(&shared).unwrap().embedded,
        2
    );
    assert_eq!(shared.lock().vector_count().unwrap(), 2);
}

/// gd26r acceptance: a multi-window backfill must not hold the writer lock
/// across ONNX work, and an entity edited in that unlocked window must be
/// retried rather than overwritten by the stale vector or silently omitted.
#[test]
fn backfill_yields_the_writer_lock_and_retries_stale_snapshots() {
    use quipu::KnowledgeVectorStore as _;

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let provider = Arc::new(BlockingBatchProvider {
        entered: std::sync::Mutex::new(Some(entered_tx)),
        release: std::sync::Mutex::new(release_rx),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });

    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(provider.clone());
    store.embedding_config_mut().dimension = 8;
    quipu::ingest_rdf(
        &mut store,
        &b"<http://example.org/changing> <http://www.w3.org/2000/01/rdf-schema#label> \"before\" ."
            [..],
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));

    let backfill_store = shared.clone();
    let backfill =
        std::thread::spawn(move || super::tools::backfill_embeddings(&backfill_store).unwrap());
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("backfill never reached the first embedding batch");

    // Equivalent store read while the provider is blocked. Pre-fix this waits
    // behind the one full-pass lock and the timeout fails deterministically.
    let (read_tx, read_rx) = std::sync::mpsc::channel();
    let read_store = shared.clone();
    std::thread::spawn(move || {
        let count = read_store.lock().current_facts().unwrap().len();
        read_tx.send(count).unwrap();
    });
    assert_eq!(
        read_rx
            .recv_timeout(std::time::Duration::from_millis(250))
            .expect("store read could not complete while a backfill batch embedded"),
        1
    );

    // Change the entity in the same unlocked window. The first vector is now
    // stale; apply must requeue the entity and embed its new version.
    {
        let mut s = shared.lock();
        quipu::ingest_rdf(
            &mut s,
            &b"<http://example.org/changing> <http://www.w3.org/2000/01/rdf-schema#label> \"after\" ."[..],
            oxrdfio::RdfFormat::NTriples,
            None,
            "2026-01-02",
            None,
            None,
        )
        .unwrap();
    }
    release_tx.send(()).unwrap();

    let outcome = backfill.join().unwrap();
    assert_eq!(outcome.embedded, 1);
    assert_eq!(outcome.stale_retries, 1);
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the stale entity was not requeued into a second bounded batch"
    );
    let s = shared.lock();
    assert_eq!(s.vector_count().unwrap(), 1);
    let matches = s.vector_search(&[0.1; 8], 1, None).unwrap();
    assert!(matches[0].text.contains("after"));
    assert!(!matches[0].text.contains("before"));
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
    let html = super::assets::UI_HTML;
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
        super::assets::THREE_JS.contains("Three.js Authors")
            && super::assets::THREE_JS.contains("SPDX-License-Identifier: MIT"),
        "ui/vendor/three.module.min.js is missing the three.js MIT header — \
         either it is not three.js, or the licence banner was stripped"
    );
    for module in [super::assets::DATALINKS_JS, super::assets::UI_HTML] {
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
        vector_reads_pooled: true,
        federation: quipu::config::FederationConfig::default(),
        readers: super::ReadPool {
            conns,
            next: std::sync::atomic::AtomicUsize::new(0),
        },
        #[cfg(feature = "reactive-reasoner")]
        reasoner: None,
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

/// Prometheus cancellations do not cancel `spawn_blocking` work. If metrics
/// queues on the writer, every timed-out scrape leaves a task behind and the
/// service eventually reaches `TasksMax`. Holding the writer here is the exact
/// discriminator: the handler can finish only if it uses the read pool.
#[tokio::test(flavor = "current_thread")]
async fn metrics_does_not_queue_behind_the_writer() {
    let (_dir, handle) = pooled_handle(1);
    let shared = Arc::new(handle);
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let thread = std::thread::spawn(move || {
        let _writer = holder.lock();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    let _response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        metrics_handler(State(shared.clone())),
    )
    .await
    .expect("metrics queued behind the writer instead of using the read pool")
    .expect("metrics failed on a read-only pooled connection");
    release_tx.send(()).unwrap();
    thread.join().unwrap();
}

/// Export is read-only but can serialize tens of megabytes. Holding the writer
/// for that interval stalls every ingest, so prove the HTTP path completes from
/// a pooled connection while the writer is deliberately unavailable.
#[tokio::test(flavor = "current_thread")]
async fn export_does_not_queue_behind_the_writer() {
    let (_dir, handle) = pooled_handle(1);
    let shared = Arc::new(handle);
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let thread = std::thread::spawn(move || {
        let _writer = holder.lock();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        export_handler(
            State(shared.clone()),
            axum::Json(json!({"format": "ntriples"})),
        ),
    )
    .await
    .expect("export queued behind the writer instead of using the read pool")
    .expect("export failed on a read-only pooled connection");
    release_tx.send(()).unwrap();
    thread.join().unwrap();
}

/// A cold Spotlight cache fill performs a full-label SPARQL fetch. At production
/// size that outlives Bobbin's 2s timeout, and `spawn_blocking` keeps executing
/// after the client leaves. If the fetch takes the writer, repeated cold calls
/// wedge both reads and writes. Holding the writer is the exact discriminator:
/// Spotlight can finish only when its complete cold path uses the read pool.
#[tokio::test(flavor = "current_thread")]
async fn cold_spotlight_does_not_queue_behind_the_writer() {
    let (_dir, handle) = pooled_handle(1);
    let shared = Arc::new(handle);
    *SPOTLIGHT_CACHE.lock().unwrap() = None;

    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let thread = std::thread::spawn(move || {
        let _writer = holder.lock();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    let _response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        spotlight_handler(
            State(shared.clone()),
            axum::Json(json!({"text": "anything", "confidence": 0.5})),
        ),
    )
    .await
    .expect("cold Spotlight queued behind the writer instead of using the read pool")
    .expect("cold Spotlight failed on a read-only pooled connection");
    release_tx.send(()).unwrap();
    thread.join().unwrap();
}

/// A request burst must produce one cache-fill leader, not a serialized queue
/// of abandoned fills. Holding the admission mutex models that leader; a
/// follower must immediately degrade to an empty annotation set.
#[tokio::test(flavor = "current_thread")]
async fn cold_spotlight_follower_does_not_queue_behind_the_fill() {
    let shared = Arc::new(super::StoreHandle::writer_only(
        Store::open_in_memory().unwrap(),
    ));
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let leader = std::thread::spawn(move || {
        let _fill = SPOTLIGHT_CACHE.lock().unwrap();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        spotlight_handler(
            State(shared),
            axum::Json(json!({"text": "anything", "confidence": 0.5})),
        ),
    )
    .await
    .expect("Spotlight follower queued behind the active cache fill")
    .expect("Spotlight follower did not degrade gracefully");
    assert_eq!(response.0["annotations"], json!([]));
    release_tx.send(()).unwrap();
    leader.join().unwrap();
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
        // Both refuse this input (no steps / no topic) — the point here is
        // that refusing must not require a write.
        (
            "tool_path_cone",
            quipu::tool_path_cone,
            json!({"trajectory": entity}),
        ),
        (
            "tool_path_backtest",
            quipu::tool_path_backtest,
            json!({"exemplar": entity}),
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

/// quipu-tkh: `"federated": true` on /query fans out through the federated
/// provider and REPORTS who answered. With no remotes configured the
/// federation is the local store alone, so the surface — `_provider` tagging,
/// the outcome list, `complete`, and the params refusal — is testable without
/// a network.
#[tokio::test]
async fn a_federated_query_reports_its_providers() {
    let mut store = Store::open_in_memory().unwrap();
    quipu::tool_knot(
        &mut store,
        &json!({
            "turtle": "@prefix ex: <http://example.org/> .\nex:a ex:name \"A\" .",
        }),
    )
    .unwrap();
    let shared: SharedStore = Arc::new(super::StoreHandle::writer_only(store));

    let resp = super::query(
        State(shared.clone()),
        axum::http::HeaderMap::new(),
        axum::Json(json!({
            "query": "SELECT ?s ?o WHERE { ?s <http://example.org/name> ?o }",
            "federated": true,
        })),
    )
    .await
    .expect("a federated query over the local member alone must succeed");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(body["complete"], json!(true));
    assert_eq!(body["count"], json!(1));
    assert_eq!(body["providers"][0]["name"], json!("local"));
    assert_eq!(body["providers"][0]["ok"], json!(true));
    assert_eq!(body["providers"][0]["rows"], json!(1));
    assert_eq!(
        body["rows"][0]["_provider"],
        json!("local"),
        "every federated row names its source"
    );

    // The temporal/graph params shape the LOCAL evaluator's context and are
    // not forwarded to members — refused loudly, never silently dropped.
    let refused = super::query(
        State(shared),
        axum::http::HeaderMap::new(),
        axum::Json(json!({
            "query": "SELECT ?s WHERE { ?s ?p ?o }",
            "federated": true,
            "valid_at": "2026-01-01T00:00:00Z",
        })),
    )
    .await;
    assert!(
        refused.is_err(),
        "valid_at on a federated query must be refused"
    );
}

/// Every route in the router has a presence in the book's REST reference
/// (quipu-83v) — the same pinning the MCP roster got after drifting to 25/26
/// while the manifest grew. The page documented 34 of 63 routes when this
/// test was written; a new route without a doc section (or a place in the
/// UI-assets exclusion note) fails here, not in a docs audit months later.
#[test]
fn book_rest_reference_covers_every_route() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let server_src = std::fs::read_to_string(root.join("src/server.rs")).unwrap();
    let page = std::fs::read_to_string(root.join("docs/book/src/reference/rest-api.md")).unwrap();

    let mut paths = Vec::new();
    for line in server_src.lines() {
        if let Some(idx) = line.find(".route(\"") {
            let rest = &line[idx + ".route(\"".len()..];
            if let Some(end) = rest.find('"') {
                paths.push(&rest[..end]);
            }
        }
    }
    assert!(
        paths.len() >= 60,
        "route extraction broke — found only {} routes; fix the parser before \
         trusting the assertions below",
        paths.len()
    );

    for path in paths {
        // "/" is the UI root; its coverage is the UI-assets exclusion note,
        // asserted via "/ui" (a bare "/" matches any page trivially).
        let probe = if path == "/" { "`GET /` " } else { path };
        assert!(
            page.contains(probe),
            "route {path} has no presence in rest-api.md — document it or add \
             it to the UI-assets exclusion note"
        );
    }
}
