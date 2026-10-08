use axum::extract::State;
use quipu::{EmbeddingProvider, Store};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct ForbiddenEmbed(Arc<AtomicUsize>);
impl EmbeddingProvider for ForbiddenEmbed {
    fn embed_text(&self, _: &str) -> quipu::Result<Vec<f32>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(quipu::Error::InvalidValue("keyword must not embed".into()))
    }
    fn dimension(&self) -> usize {
        2
    }
}

#[tokio::test]
async fn keyword_http_uses_wal_reader_without_embedding_or_writer_lock() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("corpus.db");
    let path = db.to_str().unwrap();
    let mut store = Store::open(path).unwrap();
    store.search_config_mut().keyword = true;
    store.search_config_mut().hybrid = true;
    store.search_config_mut().mode = "keyword".into();
    store.search_config_mut().named_graphs = true;
    store.initialize_lexical_index().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    store.set_embedding_provider(Arc::new(ForbiddenEmbed(calls.clone())));
    let datum = quipu::Datum {
        entity: store.intern("https://example.org/device").unwrap(),
        attribute: store
            .intern("http://www.w3.org/2000/01/rdf-schema#label")
            .unwrap(),
        value: quipu::Value::Str("walneedle".into()),
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op: quipu::Op::Assert,
    };
    store
        .transact(&[datum], "2026-01-01T00:00:00Z", None, None)
        .unwrap();
    let graph = store.graph_create("urn:test:keyword:graph").unwrap();
    let named = quipu::Datum {
        entity: store.intern("https://example.org/named-device").unwrap(),
        attribute: store
            .intern("http://www.w3.org/2000/01/rdf-schema#label")
            .unwrap(),
        value: quipu::Value::Str("walneedle namedneedle".into()),
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op: quipu::Op::Assert,
    };
    store
        .transact_to_graph(&[named], "2026-01-01T00:00:00Z", None, None, graph)
        .unwrap();
    let readers = super::super::ReadPool::open(path, &store, 1);
    assert_eq!(readers.len(), 1);
    let shared = Arc::new(super::super::StoreHandle::serving(
        store,
        readers,
        path,
        Default::default(),
    ));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let worker = std::thread::spawn(move || {
        let _hold = holder.lock();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    ready_rx.recv().unwrap();
    let mut results = Vec::new();
    for (input, count) in [
        (json!({"mode":"keyword","query":"walneedle"}), 1),
        (json!({"query":"walneedle"}), 1),
        (json!({"mode":"hybrid","alpha":0,"query":"walneedle"}), 1),
        (
            json!({"mode":"keyword","query":"walneedle","graph":"urn:test:keyword:graph"}),
            1,
        ),
        (
            json!({"mode":"keyword","query":"walneedle","all_graphs":true}),
            2,
        ),
        (
            json!({"mode":"keyword","query":"walneedle","graphs":["urn:test:keyword:graph"]}),
            1,
        ),
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::super::tools::search(State(shared.clone()), axum::Json(input)),
        )
        .await;
        results.push((result, count));
    }
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    for (result, count) in results {
        let result = result.unwrap().unwrap();
        assert_eq!(result.0["count"], count);
        assert_eq!(result.0["ranking"], "keyword");
        assert!(result.0.get("ignored_fields").is_none());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn semantic_embedding_does_not_wait_for_a_busy_reader() {
    use quipu::KnowledgeVectorStore as _;
    struct SignalEmbed(std::sync::mpsc::Sender<()>);
    impl EmbeddingProvider for SignalEmbed {
        fn embed_text(&self, _: &str) -> quipu::Result<Vec<f32>> {
            let _ = self.0.send(());
            Ok(vec![1., 0.])
        }
        fn dimension(&self) -> usize {
            2
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("corpus.db");
    let path = db.to_str().unwrap();
    let mut store = Store::open(path).unwrap();
    let entity = store.intern("https://example.org/one").unwrap();
    let datum = quipu::Datum {
        entity,
        attribute: store
            .intern("http://www.w3.org/2000/01/rdf-schema#label")
            .unwrap(),
        value: quipu::Value::Str("one".into()),
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op: quipu::Op::Assert,
    };
    store
        .transact(&[datum], "2026-01-01T00:00:00Z", None, None)
        .unwrap();
    store
        .embed_entity(entity, "one", &[1., 0.], "2026-01-01T00:00:00Z")
        .unwrap();
    let (embedded_tx, embedded_rx) = std::sync::mpsc::channel();
    store.set_embedding_provider(Arc::new(SignalEmbed(embedded_tx)));
    let readers = super::super::ReadPool::open(path, &store, 1);
    assert_eq!(readers.len(), 1);
    let shared = Arc::new(super::super::StoreHandle::serving(
        store,
        readers,
        path,
        Default::default(),
    ));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let holder = shared.clone();
    let worker = std::thread::spawn(move || {
        let hold = holder.readers.conns[0].lock();
        ready_tx.send(()).unwrap();
        let embedded_before_release = embedded_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_ok();
        drop(hold);
        embedded_before_release
    });
    ready_rx.recv().unwrap();
    let result =
        super::super::tools::search(State(shared.clone()), axum::Json(json!({"query":"one"})))
            .await
            .unwrap();
    assert_eq!(
        result.0["count"], 1,
        "positive control: the vector search completed"
    );
    assert!(
        worker.join().unwrap(),
        "configuration lookup must not park embedding behind a busy reader"
    );
}
