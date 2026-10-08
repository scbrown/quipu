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
