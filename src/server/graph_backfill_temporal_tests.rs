//! A later embedding must not leak new text into an earlier graph snapshot.
use super::*;

#[test]
fn old_membership_does_not_admit_a_later_backfill_vector() {
    use quipu::KnowledgeVectorStore as _;
    let mut store = Store::open_in_memory().unwrap();
    store.search_config_mut().named_graphs = true;
    store.embedding_config_mut().dimension = 8;
    store.set_embedding_provider(Arc::new(RecordingProvider {
        batches: Arc::new(parking_lot::Mutex::new(vec![])),
    }));
    let graph = store.graph_create("urn:test:historical").unwrap();
    // Entity already belongs to this graph at the old cutoff. Checking graph
    // membership alone therefore cannot hide its later label/vector.
    quipu::rdf::ingest_rdf_to_graph(&mut store,
        &br#"<https://example.org/historical> <http://www.w3.org/2000/01/rdf-schema#label> "old label" ."#[..],
        oxrdfio::RdfFormat::Turtle,None,"2026-01-01T00:00:00Z",None,None,graph).unwrap();
    quipu::rdf::ingest_rdf_to_graph(&mut store,
        &br#"<https://example.org/historical> <http://www.w3.org/2000/01/rdf-schema#comment> "later secret text" ."#[..],
        oxrdfio::RdfFormat::Turtle,None,"2026-02-01T00:00:00Z",None,None,graph).unwrap();
    let shared: SharedStore = Arc::new(super::super::StoreHandle::writer_only(store));
    let result =
        super::super::graph_backfill::backfill_graph_embeddings(&shared, "urn:test:historical", 1)
            .unwrap();
    assert_eq!(result.embedded, 1);
    let store = shared.lock();
    let query = json!({"embedding":vec![1.0;8],"graph":"urn:test:historical","verbose":true});
    let current = quipu::tool_search(&store, &query).unwrap();
    assert_eq!(
        current["count"], 1,
        "positive control: new vector must be present now"
    );
    assert!(
        current["results"][0]["text"]
            .as_str()
            .unwrap()
            .contains("later secret text")
    );
    let mut past = query.clone();
    past["valid_at"] = json!("2026-01-15T00:00:00Z");
    let historic = quipu::tool_search(&store, &past).unwrap();
    assert_eq!(
        historic["count"], 0,
        "old graph membership must not admit later text from a numeric timestamp"
    );
    let timestamp = current["results"][0]["valid_from"].as_str().unwrap();
    assert_eq!(
        quipu::time::normalize_rfc3339_utc(timestamp).as_deref(),
        Some(timestamp)
    );
    assert_eq!(store.vector_count().unwrap(), 1);
}
