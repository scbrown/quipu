//! `LanceDB`-backed implementation of [`KnowledgeVectorStore`].
//!
//! Gated behind `--features lancedb`. Bridges async `LanceDB` calls into the
//! synchronous trait interface via `block_in_place` + `block_on`.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{Connection, Table};

use crate::error::{Error, Result};
use crate::vector::{KnowledgeVectorStore, VectorMatch};

/// Embedding dimension (matches all-MiniLM-L6-v2 output).
const EMBEDDING_DIM: i32 = 384;

/// Table name inside the `LanceDB` database.
const TABLE_NAME: &str = "vectors";

/// `LanceDB`-backed vector store for entity embeddings.
pub struct LanceVectorStore {
    #[allow(dead_code)]
    conn: Connection,
    table: Option<Table>,
}

impl LanceVectorStore {
    /// Open (or create) a `LanceDB` vector store at the given URI.
    pub async fn open(uri: &str) -> Result<Self> {
        let conn = lancedb::connect(uri)
            .execute()
            .await
            .map_err(|e| Error::Store(format!("LanceDB connect: {e}")))?;
        let tables = conn
            .table_names()
            .execute()
            .await
            .map_err(|e| Error::Store(format!("LanceDB list tables: {e}")))?;

        let table = if tables.contains(&TABLE_NAME.to_string()) {
            Some(
                conn.open_table(TABLE_NAME)
                    .execute()
                    .await
                    .map_err(|e| Error::Store(format!("LanceDB open table: {e}")))?,
            )
        } else {
            None
        };

        Ok(Self { conn, table })
    }

    /// Open an in-memory store (useful for tests).
    pub async fn open_in_memory() -> Result<Self> {
        let conn = lancedb::connect("memory://quipu-vectors")
            .execute()
            .await
            .map_err(|e| Error::Store(format!("LanceDB in-memory connect: {e}")))?;
        Ok(Self { conn, table: None })
    }

    /// Arrow schema for the vectors table.
    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("entity_id", DataType::Int64, false),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    EMBEDDING_DIM,
                ),
                false,
            ),
            Field::new("valid_from", DataType::Utf8, false),
            Field::new("valid_to", DataType::Utf8, true),
            Field::new("entity_type", DataType::Utf8, true),
            Field::new("source_episode", DataType::Utf8, true),
        ]))
    }

    fn make_batch(
        entity_id: i64,
        text: &str,
        embedding: &[f32],
        valid_from: &str,
    ) -> Result<RecordBatch> {
        Self::make_batch_typed(entity_id, text, embedding, valid_from, None)
    }

    /// Build a single-row `RecordBatch` with optional entity type metadata.
    fn make_batch_typed(
        entity_id: i64,
        text: &str,
        embedding: &[f32],
        valid_from: &str,
        entity_type: Option<&str>,
    ) -> Result<RecordBatch> {
        let entity_ids = Int64Array::from(vec![entity_id]);
        let texts = StringArray::from(vec![text]);
        let valid_froms = StringArray::from(vec![valid_from]);
        let valid_tos = StringArray::from(vec![None::<&str>]);
        let entity_types = StringArray::from(vec![entity_type]);
        let source_episodes = StringArray::from(vec![None::<&str>]);

        let values = Float32Array::from(embedding.to_vec());
        let vector = FixedSizeListArray::try_new(
            Arc::new(Field::new("item", DataType::Float32, true)),
            EMBEDDING_DIM,
            Arc::new(values) as ArrayRef,
            None,
        )
        .map_err(|e| Error::Store(format!("Arrow FixedSizeList: {e}")))?;

        RecordBatch::try_new(
            Self::schema(),
            vec![
                Arc::new(entity_ids) as ArrayRef,
                Arc::new(texts),
                Arc::new(vector),
                Arc::new(valid_froms),
                Arc::new(valid_tos),
                Arc::new(entity_types),
                Arc::new(source_episodes),
            ],
        )
        .map_err(|e| Error::Store(format!("Arrow RecordBatch: {e}")))
    }

    /// Ensure the table exists, creating it from the batch if needed.
    #[cfg(test)]
    async fn ensure_table(&mut self, batch: RecordBatch) -> Result<()> {
        match &self.table {
            Some(table) => {
                table
                    .add(vec![batch])
                    .execute()
                    .await
                    .map_err(|e| Error::Store(format!("LanceDB add: {e}")))?;
            }
            None => {
                let table = self
                    .conn
                    .create_table(TABLE_NAME, vec![batch])
                    .execute()
                    .await
                    .map_err(|e| Error::Store(format!("LanceDB create table: {e}")))?;
                self.table = Some(table);
            }
        }
        Ok(())
    }

    /// Embed an entity with type metadata for predicate pushdown.
    pub fn embed_entity_typed(
        &self,
        entity_id: i64,
        text: &str,
        embedding: &[f32],
        valid_from: &str,
        entity_type: Option<&str>,
    ) -> Result<()> {
        let batch = Self::make_batch_typed(entity_id, text, embedding, valid_from, entity_type)?;
        self.add_batch(batch)
    }

    fn add_batch(&self, batch: RecordBatch) -> Result<()> {
        Self::block_on(async {
            match &self.table {
                Some(table) => {
                    table
                        .add(vec![batch])
                        .execute()
                        .await
                        .map_err(|e| Error::Store(format!("LanceDB add: {e}")))?;
                }
                None => {
                    return Err(Error::Store(
                        "LanceDB table not initialized — call open() first".into(),
                    ));
                }
            }
            Ok(())
        })?
    }

    /// Parse a `RecordBatch` into `VectorMatch` values.
    fn collect_matches(batch: &RecordBatch, out: &mut Vec<VectorMatch>) -> Result<()> {
        fn col<'a, T: 'static>(batch: &'a RecordBatch, name: &str) -> Result<&'a T> {
            batch
                .column_by_name(name)
                .and_then(|c| c.as_any().downcast_ref::<T>())
                .ok_or_else(|| Error::Store(format!("missing {name} column")))
        }
        let ids = col::<Int64Array>(batch, "entity_id")?;
        let texts = col::<StringArray>(batch, "text")?;
        let vf = col::<StringArray>(batch, "valid_from")?;
        let vt = col::<StringArray>(batch, "valid_to")?;
        let dist = batch
            .column_by_name("_distance")
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>());
        for i in 0..batch.num_rows() {
            out.push(VectorMatch {
                entity_id: ids.value(i),
                text: texts.value(i).to_string(),
                score: dist.map_or(0.0, |d| 1.0 - d.value(i) as f64),
                valid_from: vf.value(i).to_string(),
                valid_to: if vt.is_null(i) {
                    None
                } else {
                    Some(vt.value(i).to_string())
                },
            });
        }
        Ok(())
    }

    fn block_on<F: std::future::Future<Output = T>, T>(f: F) -> Result<T> {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::Store("No Tokio runtime available for LanceDB".into()))?;
        Ok(tokio::task::block_in_place(|| handle.block_on(f)))
    }
}

impl KnowledgeVectorStore for LanceVectorStore {
    fn embed_entity(
        &self,
        entity_id: i64,
        text: &str,
        embedding: &[f32],
        valid_from: &str,
    ) -> Result<()> {
        let batch = Self::make_batch(entity_id, text, embedding, valid_from)?;
        self.add_batch(batch)
    }

    fn close_embedding(&self, entity_id: i64, valid_to: &str) -> Result<()> {
        let Some(table) = &self.table else {
            return Ok(());
        };

        Self::block_on(async {
            table
                .update()
                .only_if(format!("entity_id = {entity_id} AND valid_to IS NULL"))
                .column("valid_to", format!("'{valid_to}'"))
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB update: {e}")))?;
            Ok(())
        })?
    }

    fn vector_search(
        &self,
        query: &[f32],
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        self.vector_search_filtered(query, limit, None, valid_at)
    }

    fn vector_search_filtered(
        &self,
        query: &[f32],
        limit: usize,
        filter: Option<&str>,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        let Some(table) = &self.table else {
            return Ok(vec![]);
        };

        Self::block_on(async {
            // Build combined filter: temporal + optional predicate pushdown.
            let mut conditions = Vec::new();
            match valid_at {
                Some(vt) => conditions.push(format!(
                    "valid_from <= '{vt}' AND (valid_to IS NULL OR valid_to > '{vt}')"
                )),
                None => conditions.push("valid_to IS NULL".to_string()),
            }
            if let Some(f) = filter {
                conditions.push(format!("({f})"));
            }

            let results = table
                .vector_search(query.to_vec())
                .map_err(|e| Error::Store(format!("LanceDB vector_search: {e}")))?
                .only_if(conditions.join(" AND "))
                .limit(limit)
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB execute: {e}")))?;

            use futures::TryStreamExt;
            let batches: Vec<RecordBatch> = results
                .try_collect()
                .await
                .map_err(|e| Error::Store(format!("LanceDB collect: {e}")))?;

            let mut matches = Vec::new();
            for batch in &batches {
                Self::collect_matches(batch, &mut matches)?;
            }

            matches.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            matches.truncate(limit);
            Ok(matches)
        })?
    }

    fn vector_count(&self) -> Result<usize> {
        let Some(table) = &self.table else {
            return Ok(0);
        };

        Self::block_on(async {
            let count = table
                .count_rows(Some("valid_to IS NULL".to_string()))
                .await
                .map_err(|e| Error::Store(format!("LanceDB count: {e}")))?;
            Ok(count)
        })?
    }
}

#[cfg(test)]
mod tests {
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
        let batch =
            LanceVectorStore::make_batch(1, "Alice the engineer", &emb1, "2026-01-01").unwrap();
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
}
