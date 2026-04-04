use super::*;

fn make_embedding(seed: f32, dim: usize) -> Vec<f32> {
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_embed_and_search() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();

    let emb1 = make_embedding(1.0, EMBEDDING_DIM as usize);
    let emb2 = make_embedding(1.1, EMBEDDING_DIM as usize);
    let emb3 = make_embedding(5.0, EMBEDDING_DIM as usize);

    // Bootstrap the table with the first insert.
    let batch = LanceVectorStore::make_batch(1, "Alice the engineer", &emb1, "2026-01-01").unwrap();
    store.ensure_table(batch).await.unwrap();

    // Remaining inserts go through the trait.
    store
        .embed_entity(2, "Bob the developer", &emb2, "2026-01-01")
        .unwrap();
    store
        .embed_entity(3, "Carol the manager", &emb3, "2026-01-01")
        .unwrap();

    assert_eq!(store.vector_count().unwrap(), 3);

    let results = store.vector_search(&emb1, 3, None).unwrap();
    assert_eq!(results.len(), 3);
    // Alice should be top match (closest to query).
    assert_eq!(results[0].entity_id, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_close_embedding() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();

    let emb = make_embedding(1.0, EMBEDDING_DIM as usize);
    let batch = LanceVectorStore::make_batch(1, "entity one", &emb, "2026-01-01").unwrap();
    store.ensure_table(batch).await.unwrap();

    assert_eq!(store.vector_count().unwrap(), 1);
    store.close_embedding(1, "2026-03-01").unwrap();
    assert_eq!(store.vector_count().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_filtered_search_by_entity_type() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();
    let emb_a = make_embedding(1.0, EMBEDDING_DIM as usize);
    let emb_b = make_embedding(1.1, EMBEDDING_DIM as usize);
    let type_filter = Some("entity_type = 'http://example.org/Person'");

    let batch = LanceVectorStore::make_batch_typed(
        1,
        "Alice",
        &emb_a,
        "2026-01-01",
        Some("http://example.org/Person"),
    )
    .unwrap();
    store.ensure_table(batch).await.unwrap();
    store
        .embed_entity_typed(
            2,
            "Bot",
            &emb_b,
            "2026-01-01",
            Some("http://example.org/Bot"),
        )
        .unwrap();

    // Unfiltered → both; filtered → only Person.
    assert_eq!(
        store
            .vector_search_filtered(&emb_a, 10, None, None)
            .unwrap()
            .len(),
        2
    );
    let filtered = store
        .vector_search_filtered(&emb_a, 10, type_filter, None)
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].entity_id, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_filtered_search_combined_temporal_and_type() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();
    let emb = make_embedding(1.0, EMBEDDING_DIM as usize);
    let type_filter = Some("entity_type = 'http://example.org/Person'");

    let batch = LanceVectorStore::make_batch_typed(
        1,
        "Old person",
        &emb,
        "2026-01-01",
        Some("http://example.org/Person"),
    )
    .unwrap();
    store.ensure_table(batch).await.unwrap();
    store.close_embedding(1, "2026-03-01").unwrap();
    store
        .embed_entity_typed(
            2,
            "Current person",
            &emb,
            "2026-03-01",
            Some("http://example.org/Person"),
        )
        .unwrap();

    // Current + type filter → only entity 2.
    let r = store
        .vector_search_filtered(&emb, 10, type_filter, None)
        .unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].entity_id, 2);
    // Time-travel to Feb + type filter → only entity 1.
    let r = store
        .vector_search_filtered(&emb, 10, type_filter, Some("2026-02-01"))
        .unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].entity_id, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_temporal_search() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();

    let emb_old = make_embedding(1.0, EMBEDDING_DIM as usize);
    let emb_new = make_embedding(2.0, EMBEDDING_DIM as usize);

    // Old embedding.
    let batch = LanceVectorStore::make_batch(1, "old desc", &emb_old, "2026-01-01").unwrap();
    store.ensure_table(batch).await.unwrap();
    store.close_embedding(1, "2026-03-01").unwrap();

    // New embedding.
    store
        .embed_entity(1, "new desc", &emb_new, "2026-03-01")
        .unwrap();

    // Current: only new.
    let results = store.vector_search(&emb_old, 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "new desc");

    // Time-travel to February: only old.
    let results = store
        .vector_search(&emb_old, 10, Some("2026-02-01"))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "old desc");
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_fts_index_and_search() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();

    let emb1 = make_embedding(1.0, EMBEDDING_DIM as usize);
    let emb2 = make_embedding(2.0, EMBEDDING_DIM as usize);
    let emb3 = make_embedding(3.0, EMBEDDING_DIM as usize);

    let batch =
        LanceVectorStore::make_batch(1, "Rust programming language", &emb1, "2026-01-01").unwrap();
    store.ensure_table(batch).await.unwrap();
    store
        .embed_entity(2, "Python scripting language", &emb2, "2026-01-01")
        .unwrap();
    store
        .embed_entity(3, "JavaScript runtime engine", &emb3, "2026-01-01")
        .unwrap();

    // Before FTS index, text_search returns empty.
    let results = store.text_search("Rust", 10, None).unwrap();
    assert!(results.is_empty());

    // Create FTS index.
    store.ensure_fts_index().unwrap();

    // Now FTS should find "Rust".
    let results = store.text_search("Rust", 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, 1);
    assert!(results[0].score > 0.0);

    // Search for "language" should match two entries.
    let results = store.text_search("language", 10, None).unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn lance_fts_respects_temporal_filter() {
    let mut store = LanceVectorStore::open_in_memory().await.unwrap();

    let emb = make_embedding(1.0, EMBEDDING_DIM as usize);
    let batch = LanceVectorStore::make_batch(1, "temporal entity", &emb, "2026-01-01").unwrap();
    store.ensure_table(batch).await.unwrap();
    store.close_embedding(1, "2026-03-01").unwrap();

    store
        .embed_entity(1, "updated entity", &emb, "2026-03-01")
        .unwrap();

    store.ensure_fts_index().unwrap();

    // Current: only "updated entity" is visible.
    let results = store.text_search("entity", 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "updated entity");

    // Time-travel to February: only "temporal entity".
    let results = store.text_search("entity", 10, Some("2026-02-01")).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "temporal entity");
}
