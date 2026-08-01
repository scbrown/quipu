//! Tests for the SQLite-backed knowledge vector store.

use super::*;

fn make_embedding(seed: f32, dim: usize) -> Vec<f32> {
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

#[test]
fn embed_and_search() {
    let store = Store::open_in_memory().unwrap();

    let e1 = store.intern("http://example.org/alice").unwrap();
    let e2 = store.intern("http://example.org/bob").unwrap();
    let e3 = store.intern("http://example.org/carol").unwrap();

    let emb1 = make_embedding(1.0, 8);
    let emb2 = make_embedding(1.1, 8); // similar to emb1
    let emb3 = make_embedding(5.0, 8); // different

    store
        .embed_entity(e1, "Alice the engineer", &emb1, "2026-01-01")
        .unwrap();
    store
        .embed_entity(e2, "Bob the developer", &emb2, "2026-01-01")
        .unwrap();
    store
        .embed_entity(e3, "Carol the manager", &emb3, "2026-01-01")
        .unwrap();

    assert_eq!(store.vector_count().unwrap(), 3);

    // Search with emb1 — Alice should be top match, Bob close second.
    let results = store.vector_search(&emb1, 3, None).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].entity_id, e1); // Alice = exact match
    assert_eq!(results[1].entity_id, e2); // Bob = similar
    assert!(results[0].score > results[1].score);
    assert!(results[1].score > results[2].score);
}

#[test]
fn dimension_mismatch_fails_loud() {
    // hq-7v0: searching with a query whose dim differs from the stored
    // vectors must error, not silently score every candidate 0.0.
    let store = Store::open_in_memory().unwrap();
    let e1 = store.intern("http://example.org/alice").unwrap();
    store
        .embed_entity(e1, "Alice", &make_embedding(1.0, 8), "2026-01-01")
        .unwrap();

    // 4-dim query vs 8-dim stored → loud error mentioning the dimensions.
    let err = store
        .vector_search(&make_embedding(1.0, 4), 3, None)
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("dimension mismatch"), "got: {msg}");

    // Matching dimension still works.
    assert_eq!(
        store
            .vector_search(&make_embedding(1.0, 8), 3, None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn temporal_vector_search() {
    let store = Store::open_in_memory().unwrap();

    let e1 = store.intern("http://example.org/svc").unwrap();
    let emb_old = make_embedding(1.0, 8);
    let emb_new = make_embedding(2.0, 8);

    // Old embedding, valid until March.
    store
        .embed_entity(e1, "old description", &emb_old, "2026-01-01")
        .unwrap();
    store.close_embedding(e1, "2026-03-01").unwrap();

    // New embedding, current.
    store
        .embed_entity(e1, "new description", &emb_new, "2026-03-01")
        .unwrap();

    // Current search: only new embedding.
    let results = store.vector_search(&emb_old, 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "new description");

    // Time-travel search to February: only old embedding.
    let results = store
        .vector_search(&emb_old, 10, Some("2026-02-01"))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "old description");

    // Time-travel to April: only new embedding.
    let results = store
        .vector_search(&emb_new, 10, Some("2026-04-01"))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "new description");
}

#[test]
fn cosine_similarity_self() {
    let v = vec![1.0, 2.0, 3.0];
    let sim = cosine_similarity(&v, &v);
    assert!((sim - 1.0).abs() < 1e-10);
}

#[test]
fn cosine_similarity_orthogonal() {
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    let sim = cosine_similarity(&a, &b);
    assert!(sim.abs() < 1e-10);
}

#[test]
fn embedding_round_trip() {
    let original: Vec<f32> = vec![1.0, -2.5, 3.25, 0.0, f32::MAX, f32::MIN];
    let bytes = f32_slice_to_bytes(&original);
    let decoded = bytes_to_f32_slice(&bytes);
    assert_eq!(original, decoded);
}

#[test]
fn limit_results() {
    let store = Store::open_in_memory().unwrap();

    for i in 0..20 {
        let eid = store.intern(&format!("http://example.org/e{i}")).unwrap();
        let emb = make_embedding(i as f32, 8);
        store
            .embed_entity(eid, &format!("entity {i}"), &emb, "2026-01-01")
            .unwrap();
    }

    let query = make_embedding(0.0, 8);
    let results = store.vector_search(&query, 5, None).unwrap();
    assert_eq!(results.len(), 5);
}

#[test]
fn vector_store_trait_object() {
    let store = Store::open_in_memory().unwrap();
    let vs: &dyn KnowledgeVectorStore = store.vector_store();

    let eid = store.intern("http://example.org/test").unwrap();
    let emb = make_embedding(1.0, 8);

    vs.embed_entity(eid, "test entity", &emb, "2026-01-01")
        .unwrap();
    assert_eq!(vs.vector_count().unwrap(), 1);

    let results = vs.vector_search(&emb, 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, eid);
}
