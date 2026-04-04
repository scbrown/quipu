//! `LanceDB`-backed implementation of [`KnowledgeVectorStore`].
//!
//! Gated behind `--features lancedb`. Bridges async `LanceDB` calls into the
//! synchronous trait interface via `block_in_place` + `block_on`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lancedb::index::Index;
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
    conn: Connection,
    table: Option<Table>,
    has_fts_index: AtomicBool,
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

        Ok(Self {
            conn,
            table,
            has_fts_index: AtomicBool::new(false),
        })
    }

    /// Open an in-memory store (useful for tests).
    pub async fn open_in_memory() -> Result<Self> {
        let conn = lancedb::connect("memory://quipu-vectors")
            .execute()
            .await
            .map_err(|e| Error::Store(format!("LanceDB in-memory connect: {e}")))?;
        Ok(Self {
            conn,
            table: None,
            has_fts_index: AtomicBool::new(false),
        })
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
        let fts_score = batch
            .column_by_name("_score")
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>());
        for i in 0..batch.num_rows() {
            // Prefer BM25 _score (FTS) over _distance (vector search).
            let score = if let Some(s) = fts_score {
                s.value(i) as f64
            } else {
                dist.map_or(0.0, |d| 1.0 - d.value(i) as f64)
            };
            out.push(VectorMatch {
                entity_id: ids.value(i),
                text: texts.value(i).to_string(),
                score,
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

    fn text_search(
        &self,
        query: &str,
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        if !self.has_fts_index.load(Ordering::Acquire) {
            return Ok(vec![]);
        }
        let Some(table) = &self.table else {
            return Ok(vec![]);
        };

        Self::block_on(async {
            use lancedb::index::scalar::FullTextSearchQuery;

            let mut conditions = Vec::new();
            match valid_at {
                Some(vt) => conditions.push(format!(
                    "valid_from <= '{vt}' AND (valid_to IS NULL OR valid_to > '{vt}')"
                )),
                None => conditions.push("valid_to IS NULL".to_string()),
            }

            let mut qb = table
                .query()
                .full_text_search(FullTextSearchQuery::new(query.to_string()))
                .limit(limit);
            if !conditions.is_empty() {
                qb = qb.only_if(conditions.join(" AND "));
            }

            let results = qb
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB FTS execute: {e}")))?;

            use futures::TryStreamExt;
            let batches: Vec<RecordBatch> = results
                .try_collect()
                .await
                .map_err(|e| Error::Store(format!("LanceDB FTS collect: {e}")))?;

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

    fn ensure_fts_index(&self) -> Result<()> {
        let Some(table) = &self.table else {
            return Ok(());
        };

        Self::block_on(async {
            table
                .create_index(&["text"], Index::FTS(Default::default()))
                .replace(true)
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB FTS index: {e}")))
        })??;

        self.has_fts_index.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
#[path = "vector_lance_tests.rs"]
mod tests;
